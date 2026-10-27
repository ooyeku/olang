//! Matrices and model formulas (docs/ods.md).
//!
//! - Numbers: `ods.matrix`, `@`, `mat.*`, `ods.corr`/`ods.cov` and the
//!   formula fits, pinned against NumPy 2.5 / SciPy 1.18 / pandas 3.0 on
//!   the fixtures in tests/fixtures/matrix (reference.json; make_data.py
//!   wrote it), in the interpreter and with every function compiled.
//! - Tiers: programs that print run three ways (`--no-ovm`,
//!   `--ovm-tier=1`, the default) and must print the same text.
//! - Errors: a formula's errors point into the formula; shapes, nulls and
//!   singular systems say what is wrong.

use olang::ast::Value;
use olang::{Interpreter, Parser};
use serde_json::Value as Json;
use std::process::Command;

fn fixtures() -> String {
    format!("{}/tests/fixtures/matrix", env!("CARGO_MANIFEST_DIR"))
}

fn eval_tier(source: &str, tier: Option<u32>) -> Result<Value, String> {
    let source = source.replace("FIX", &fixtures());
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if let Some(t) = tier {
                interpreter.enable_bytecode_tier(t, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// The program's value, which both tiers must compute alike.
fn eval(source: &str) -> Value {
    let a = eval_tier(source, None).unwrap_or_else(|e| panic!("{e}\n--- source ---\n{source}"));
    let b = eval_tier(source, Some(1))
        .unwrap_or_else(|e| panic!("tier 1: {e}\n--- source ---\n{source}"));
    assert_eq!(
        format!("{a:?}"),
        format!("{b:?}"),
        "the tiers disagree on\n{source}"
    );
    a
}

fn reference() -> Json {
    let text =
        std::fs::read_to_string(format!("{}/reference.json", fixtures())).expect("reference.json");
    serde_json::from_str(&text).expect("json")
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Float(x) => *x,
        Value::Integer(x) => *x as f64,
        other => panic!("expected a number, got {:?}", other),
    }
}

fn nums(v: &Value) -> Vec<f64> {
    match v {
        Value::List(items) => items
            .iter()
            .flat_map(|x| match x {
                Value::List(_) => nums(x),
                other => vec![num(other)],
            })
            .collect(),
        other => panic!("expected a list, got {:?}", other),
    }
}

fn items(v: &Value) -> Vec<Value> {
    match v {
        Value::List(items) => items.as_ref().clone(),
        other => panic!("expected a list, got {:?}", other),
    }
}

fn jnums(j: &Json) -> Vec<f64> {
    match j {
        Json::Array(a) => a.iter().flat_map(jnums).collect(),
        other => vec![other.as_f64().expect("number")],
    }
}

fn all_close(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(
        got.len(),
        want.len(),
        "{what}: length {} vs {}",
        got.len(),
        want.len()
    );
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= tol * (1.0 + w.abs()),
            "{what}[{i}]: got {g}, want {w} (tol {tol})"
        );
    }
}

const READ: &str = r#"
let df = unwrap(ods.read_csv_file("FIX/data.csv"))
let X = ods.matrix(df, ["x1", "x2", "x3"])
fn flat(m) = flatten(mat.to_list(m))
fn l(s) = ods.to_list(s)
"#;

#[test]
fn matrices_match_numpy() {
    let r = reference();
    let v = items(&eval(&format!(
        "{READ}\n{}",
        r#"
let G = X.gram()
let P = ods.slice(df, 0, 60) |> ods.matrix(["x1", "x2", "x3"])
let C = ods.slice(df, 0, 5) |> ods.matrix(["x1", "x2", "x3"])
let e = mat.eigh(G)
[flat(G), take(l(X @ [0.5, -1.0, 2.0]), 10), l(X.t() @ df.price), l(mat.solve(G, X.t() @ df.price)),
 flat(mat.inv(G)), flat(mat.cholesky(G)), l(e.values), flat(e.vectors),
 l(X.mean()), l(X.std(axis: 0)), take(l(X.mean(axis: 1)), 10),
 flat(mat.cdist(P, C).t()), flat(mat.cdist(P, C, metric: "euclidean").t()),
 flat(mat.cdist(P, C, metric: "cityblock").t()), flat(mat.cdist(P, C, metric: "cosine").t()),
 flat(mat.matmul(G, mat.inv(G))), l(df["price"] @ X)]
"#
    )));
    let want = |k: &str| jnums(&r[k]);
    all_close(&nums(&v[0]), &want("gram"), 1e-12, "gram");
    all_close(&nums(&v[1]), &want("xb"), 1e-12, "X @ b");
    all_close(&nums(&v[2]), &want("xty"), 1e-12, "X.t() @ y");
    all_close(&nums(&v[3]), &want("solve"), 1e-10, "solve");
    all_close(&nums(&v[4]), &want("inv"), 1e-10, "inv");
    all_close(&nums(&v[5]), &want("cholesky"), 1e-12, "cholesky");
    all_close(&nums(&v[6]), &want("eig_values"), 1e-10, "eigh values");
    all_close(&nums(&v[7]), &want("eig_vectors"), 1e-9, "eigh vectors");
    all_close(&nums(&v[8]), &want("mean0"), 1e-12, "mean axis 0");
    all_close(&nums(&v[9]), &want("std0"), 1e-12, "std axis 0");
    all_close(&nums(&v[10]), &want("mean1"), 1e-12, "mean axis 1");
    all_close(
        &nums(&v[11]),
        &want("cdist_sqeuclidean"),
        1e-12,
        "cdist sqeuclidean",
    );
    all_close(
        &nums(&v[12]),
        &want("cdist_euclidean"),
        1e-12,
        "cdist euclidean",
    );
    all_close(
        &nums(&v[13]),
        &want("cdist_cityblock"),
        1e-12,
        "cdist cityblock",
    );
    all_close(&nums(&v[14]), &want("cdist_cosine"), 1e-10, "cdist cosine");
    let id = nums(&v[15]);
    all_close(
        &id,
        &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        1e-10,
        "G G⁻¹",
    );
    all_close(&nums(&v[16]), &want("xty"), 1e-12, "y @ X");
}

#[test]
fn correlation_and_covariance_match_pandas() {
    let r = reference();
    let v = items(&eval(&format!(
        "{READ}\n{}",
        r#"
let cols = ["x1", "x2", "x3", "w"]
fn cells(t) = flatten(map(cols, (c) => l(t[c])))
let p = ods.corr(df, cols)
[cells(p), cells(df.corr(cols, method: "spearman")), cells(ods.cov(df, cols)), l(p["term"])]
"#
    )));
    // A table's cells column by column; the matrices are symmetric.
    all_close(&nums(&v[0]), &jnums(&r["corr_pearson"]), 1e-12, "pearson");
    all_close(&nums(&v[1]), &jnums(&r["corr_spearman"]), 1e-12, "spearman");
    all_close(&nums(&v[2]), &jnums(&r["cov"]), 1e-12, "cov");
    assert_eq!(format!("{:?}", v[3]).matches("String").count(), 4);
}

#[test]
fn formula_fits_match_least_squares() {
    let r = reference();
    let v = items(&eval(&format!(
        "{READ}\n{}",
        r#"
let f1 = learn.linear(df, "price ~ x1 + x2 + I(x1^2) + C(hood) + C(kind):C(hour)")
let f2 = learn.linear(df, "log(price + 10) ~ x1 * kind")
let f3 = learn.linear(df, "price ~ C(hood) + x1 - 1")
let q = ods.head(df, 6)
[[f1.params.intercept, f1.params.x1, f1.params.x2, map_get(f1.params, "I(x1^2)")],
 [f1.params.hood_north, f1.params.hood_south, f1.params.hood_west],
 f1.r2, len(f1.features), take(l(f1.fitted), 10), l(f1.predict(q)),
 l(f2.coef["estimate"]), f2.r2, l(f2.coef["term"]),
 l(f3.coef["estimate"]), l(f3.coef["term"]), f1.y, f2.y]
"#
    )));
    let want = |k: &str| jnums(&r[k]);
    all_close(&nums(&v[0]), &want("f1_head"), 1e-8, "f1 head");
    all_close(&nums(&v[1]), &want("f1_hood"), 1e-8, "f1 hood");
    all_close(&[num(&v[2])], &want("f1_r2"), 1e-10, "f1 r2");
    // the design's columns, the intercept apart
    assert_eq!(
        num(&v[3]) as usize + 1,
        r["f1_k"].as_u64().unwrap() as usize
    );
    all_close(&nums(&v[4]), &want("f1_fitted"), 1e-8, "f1 fitted");
    all_close(&nums(&v[5]), &want("f1_predict"), 1e-8, "f1 predict");
    all_close(&nums(&v[6]), &want("f2_beta"), 1e-8, "f2 coef");
    all_close(&[num(&v[7])], &want("f2_r2"), 1e-10, "f2 r2");
    let terms = format!("{:?}", v[8]);
    for t in [
        "intercept",
        "x1",
        "kind_house",
        "kind_loft",
        "x1:kind_house",
        "x1:kind_loft",
    ] {
        assert!(terms.contains(&format!("\"{t}\"")), "{t} in {terms}");
    }
    all_close(&nums(&v[9]), &want("f3_beta"), 1e-8, "f3 coef");
    let terms = format!("{:?}", v[10]);
    assert!(
        terms.contains("hood_east") && !terms.contains("intercept"),
        "{terms}"
    );
    assert_eq!(format!("{:?}", v[11]), "String(\"price\")");
    assert_eq!(format!("{:?}", v[12]), "String(\"log(price+10)\")");
}

// ---------------------------------------------------------------------
// Three tiers, printed
// ---------------------------------------------------------------------

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn run(code: &str, flags: &[&str]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "olang_matrix_formula_{}_{}",
        std::process::id(),
        code.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("p{}.ol", flags.len()));
    std::fs::write(&path, code.replace("FIX", &fixtures())).unwrap();
    let out = Command::new(olang())
        .args(flags)
        .arg(&path)
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn agrees(code: &str, expected: &str) {
    for flags in [&["--no-ovm"][..], &["--ovm-tier=1"][..], &[][..]] {
        let (out, err) = run(code, flags);
        assert_eq!(
            out.trim_end(),
            expected.trim_start_matches('\n').trim_end(),
            "flags {flags:?}\nstderr: {err}\nprogram:\n{code}"
        );
    }
}

#[test]
fn a_matrix_prints_multiplies_and_answers_methods() {
    agrees(
        r#"
let f = ods.frame({ a: [1.0, 2.0, 3.0], b: [0.0, 1.0, -1.0], n: [1, 2, 3] })
fn prod(m, v) = m @ v
let X = f.matrix(["a", "b"])
println(X)
println(typeof(X))
println(len(X))
println(X.shape())
println(prod(X, [1.0, 2.0]).to_list())
println((X.t() @ X) == X.gram())
println(X.gram().to_frame(["a", "b"])["b"].to_list())
println(X.col(0).to_list())
println(X.row(2))
println(X.mean(axis: 1).to_list())
println(ods.matrix([f.a, f.n]).std().to_list())
println(mat.solve(mat.of([[2.0, 0.0], [0.0, 4.0]]), [1.0, 1.0]).to_list())
println(mat.identity(2) @ mat.of([[1.0], [2.0]]) == mat.of([[1.0], [2.0]]))
println(ods.round(mat.eigh(mat.of([[2.0, 1.0], [1.0, 2.0]])).values, 9).to_list())
println(mat.cdist(X, mat.of([[0.0, 0.0]]), metric: "cityblock").col(0).to_list())
println(f.cov(["a", "n"])["a"].to_list())
"#,
        r#"
Matrix[3 x 2]
┌───────┬───────┐
│     0 │     1 │
├───────┼───────┤
│   1.0 │   0.0 │
│   2.0 │   1.0 │
│   3.0 │  -1.0 │
└───────┴───────┘
Matrix
3
[3, 2]
[1.0, 4.0, 1.0]
true
[-1.0, 2.0]
[1.0, 2.0, 3.0]
[3.0, -1.0]
[0.5, 1.5, 1.0]
[1.0, 1.0]
[0.5, 0.25]
true
[3.0, 1.0]
[1.0, 3.0, 4.0]
[1.0, 1.0]
"#,
    );
}

#[test]
fn matrix_errors_say_what_is_wrong() {
    agrees(
        r#"
let f = ods.frame({ a: [1.0, (), 3.0], b: [1.0, 2.0, 3.0], s: ["x", "y", "z"] })
fn say(r) = match r { Ok(v) => println(v), Err(e) => println(e) }
say(attempt(() => ods.matrix(f, ["a", "b"])))
println(ods.matrix(f, ["a", "b"], nulls: "drop").shape())
say(attempt(() => ods.matrix(f, ["s"])))
say(attempt(() => mat.of([[1.0, 2.0]]) @ mat.of([[1.0, 2.0]])))
say(attempt(() => mat.solve(mat.of([[1.0, 2.0], [2.0, 4.0]]), [1.0, 1.0])))
say(attempt(() => mat.eigh(mat.of([[1.0, 2.0], [0.0, 1.0]]))))
say(attempt(() => mat.cholesky(mat.of([[1.0, 2.0], [2.0, 1.0]]))))
say(attempt(() => mat.cdist(ods.matrix(f, ["b"]), mat.of([[1.0, 2.0]]), metric: "hamming")))
say(attempt(() => ods.corr(f, ["a", "b"], method: "kendall")))
"#,
        r#"
ods.matrix: column 'a' has 1 null; drop them first, or pass nulls: "drop" to leave out the rows that have any
[2, 2]
ods.matrix: column 's' is String; a matrix takes Float, Int or Bool columns
`@`: the shapes do not chain: 1×2 @ 1×2 (the left's columns must equal the right's rows)
mat.solve: the matrix is singular
mat.eigh: needs a symmetric matrix; [1, 0] is 0 but [0, 1] is 2
mat.cholesky: the matrix is not positive definite
mat.cdist: metric is one of sqeuclidean, euclidean, cityblock, manhattan, chebyshev, cosine, got "hamming"
ods.corr: method is "pearson" or "spearman", got "kendall"
"#,
    );
}

#[test]
fn formulas_fit_predict_and_refit_on_every_tier() {
    agrees(
        r#"
let df = unwrap(ods.read_csv_file("FIX/data.csv"))
let fit = learn.linear(df, "price ~ x1 + I(x1^2) + C(hood) + C(kind):C(hour)")
println(len(fit.features))
println(take(fit.features, 6))
println(fit.features[len(fit.features) - 1])
let q = ods.head(df, 3)
println(ods.round(fit.predict(q), 6).to_list() == ods.round(take_fitted(fit, 3), 6).to_list())
fn take_fitted(m, k) = ods.series(take(ods.to_list(m.fitted), k))
println(ods.n_rows(learn.cross_validate(fit, df, folds: 4)))
let tree = learn.tree(df, "price ~ x1 + hood", max_depth: 3)
println(tree.inputs)
println(ods.n_rows(tree.importance()))
let lg = learn.logistic(df.with({ hi: df.price > 9.0 }), "hi ~ x1 + kind")
println(lg.terms)
println(ods.columns(fit.conf_int(0.9)))
println(ods.columns(fit.predict(q, interval: "prediction", level: 0.9)))
println(len(fit.resid) == fit.n)
println(ods.n_rows(learn.permutation_importance(fit, df, metric: "rmse", repeats: 2, seed: 1)))
"#,
        r#"
16
["x1", "I(x1^2)", "hood_north", "hood_south", "hood_west", "kind_house"]
kind_loft:hour_3
true
4
["x1", "hood"]
4
["intercept", "x1", "kind_house", "kind_loft"]
["term", "lo", "hi"]
["fit", "se", "lo", "hi"]
true
4
"#,
    );
}

#[test]
fn formula_errors_point_into_the_formula() {
    agrees(
        r#"
let df = ods.frame({ y: [1.0, 2.0, 3.0, 4.0], x: [1.0, 3.0, 2.0, 5.0], g: ["a", "b", "a", "b"] })
fn say(r) = match r { Ok(v) => println("ok"), Err(e) => println(e) }
say(attempt(() => learn.linear(df, "y ~ x + z")))
say(attempt(() => learn.linear(df, "y ~ x +")))
say(attempt(() => learn.linear(df, "y ~ (x + g)^2.5")))
say(attempt(() => learn.linear(df, "y ~ wobble(x)")))
say(attempt(() => learn.linear(df, "y ~ x", ["x"])))
say(attempt(() => learn.linear(df, "y ~ y + x")))
say(attempt(() => learn.logistic(df.with({ b: df.y > 2.0 }), "b ~ x - 1")))
"#,
        r#"
learn: in the formula, no column 'z' (the Frame has y, x, g):
    y ~ x + z
            ^
learn: in the formula, a term is missing here:
    y ~ x +
           ^
learn: in the formula, a power of terms is a whole number of at least 1 (`(a + b)^2`); arithmetic goes inside I(…):
    y ~ (x + g)^2.5
                ^^^
learn: in the formula, 'wobble' is not a formula function (C, I, Q, or a math function: log, exp, sqrt, abs, …):
    y ~ wobble(x)
        ^^^^^^
learn.linear: the formula names the inputs; give them there or as x, not both
learn: in the formula, the response 'y' is also on the right of `~`:
    y ~ y + x
    ^
learn.logistic: this model always fits an intercept; leave `- 1` out of the formula
"#,
    );
}

#[test]
fn learn_takes_keywords_and_fit_methods_win_over_entries() {
    agrees(
        r#"
let n = 60
let f = ods.frame({ x: ods.arange(n) * 0.5, z: (ods.arange(n) % 7) * 1.0, y: ods.arange(n) * 1.5 + 2.0 })
let fo = learn.forest(f, y: "y", x: ["x", "z"], n_trees: 20, max_depth: 4, seed: 3)
println(ods.columns(fo.importance()))
println(typeof(fo.importance))
let s = learn.split(f, test: 0.25, seed: 7)
println(ods.n_rows(s.test))
let b = learn.boost(f, y: "y", n_trees: 10, learning_rate: 0.3, seed: 1)
println(b.n_trees)
println(len(learn.kfold(f, 3, seed: 2)))
"#,
        r#"
["feature", "importance"]
Frame
15
10
60
"#,
    );
}

#[test]
fn a_matrix_takes_elementwise_arithmetic_and_column_functions() {
    agrees(
        r#"
let A = mat.of([[1.0, 2.0], [3.0, 4.0]])
let B = mat.of([[0.5, 0.0], [0.0, 0.5]])
fn add(a, b) = a + b
fn layer(x, w, b) = (x @ w + b).clip(lo: 0.0)
println(mat.to_list(A - B))
println(mat.to_list(A * B))
println(mat.to_list(add(A, A) / A))
println(mat.to_list(2 * A))
println(mat.to_list(1.0 - A))
println(mat.to_list(-A))
println(mat.to_list(A + ods.series([10.0, 20.0])))
println(mat.to_list([10.0, 20.0] - A))
println(mat.to_list(A * mat.of([[2.0], [3.0]])))
println(mat.to_list(A.t() - A))
println(mat.to_list(A.tanh().round(3)))
println(mat.to_list(ods.sigmoid(-A).round(4)))
println(mat.to_list(A.pow(2)))
println(mat.to_list(layer(A, B, [-1.0, 0.5])))
println(A.sum(axis: 0).to_list())
println(A.sum(axis: 1).to_list())
println(attempt(() => A + mat.of([[1.0, 2.0, 3.0]])))
println(attempt(() => A + ods.series([1.0, 2.0, 3.0])))
println(attempt(() => A % 2))
println(attempt(() => mat.tanh(ods.series([1.0]))))
"#,
        r#"
[[0.5, 2.0], [3.0, 3.5]]
[[0.5, 0.0], [0.0, 2.0]]
[[2.0, 2.0], [2.0, 2.0]]
[[2.0, 4.0], [6.0, 8.0]]
[[0.0, -1.0], [-2.0, -3.0]]
[[-1.0, -2.0], [-3.0, -4.0]]
[[11.0, 22.0], [13.0, 24.0]]
[[9.0, 18.0], [7.0, 16.0]]
[[2.0, 4.0], [9.0, 12.0]]
[[0.0, 1.0], [-1.0, 0.0]]
[[0.762, 0.964], [0.995, 0.999]]
[[0.2689, 0.1192], [0.0474, 0.018]]
[[1.0, 4.0], [9.0, 16.0]]
[[0.0, 1.5], [0.5, 2.5]]
[4.0, 6.0]
[3.0, 7.0]
Err("a 2×2 matrix + a 1×3 one: the shapes must be equal, or one a single row or column of the other")
Err("`+`: a row of 3 beside a 2×2 matrix: a Series or list is applied to every row, so it is as long as the matrix is wide (2); a column is an n × 1 Matrix (ods.matrix([s]))")
Err("Invalid binary operation: cannot apply '%' to Matrix and Int")
Err("mat.tanh: needs a Matrix, got Series (ods.tanh takes a Series)")
"#,
    );
}
