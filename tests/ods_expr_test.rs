//! The data-ergonomics column functions and `ods.col` expressions, from
//! source: each against the scalar language or a list reference, every
//! program run on the tree-walker and with functions promoted to the
//! bytecode tier (results must agree exactly).

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier_threshold: Option<u32>) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let parser = Parser::new();
            let program = parser.parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if let Some(threshold) = tier_threshold {
                interpreter.enable_bytecode_tier(threshold, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// Run on both tiers; they must agree, and the answer is returned.
/// Every program can say `every([...])`.
fn both(source: &str) -> Result<Value, String> {
    let source = &format!("fn every(xs) = all(xs, (x) => x == true)\n{source}");
    let a = eval(source, None);
    let b = eval(source, Some(2));
    match (&a, &b) {
        (Ok(x), Ok(y)) => assert_eq!(x, y, "tiers disagree on\n{source}"),
        (Err(x), Err(y)) => assert_eq!(x, y, "tiers raise differently on\n{source}"),
        _ => panic!("tiers disagree on success:\n  {a:?}\n  {b:?}\n{source}"),
    }
    a
}

fn yes(source: &str) {
    match both(source) {
        Ok(Value::Boolean(true)) => {}
        other => panic!("expected true, got {other:?} from\n{source}"),
    }
}

fn raises(source: &str, needle: &str) {
    match both(source) {
        Err(e) => assert!(e.contains(needle), "{e}\n  wanted: {needle}"),
        Ok(v) => panic!("expected an error with {needle:?}, got {v:?}"),
    }
}

#[test]
fn math_functions_are_the_scalar_functions_over_a_column() {
    yes(r#"
        let xs = [0.25, 0.5, 1.5, 2.0, 7.25]
        let s = ods.series(xs)
        fn same(col, f) = ods.to_list(col) == map(xs, f)
        every([
            same(ods.exp(s), (x) => math.exp(x)),
            same(ods.sqrt(s), (x) => math.sqrt(x)),
            same(ods.ln(s), (x) => math.ln(x)),
            same(ods.log10(s), (x) => math.log10(x)),
            same(ods.sin(s), (x) => math.sin(x)),
            same(ods.tanh(s), (x) => math.tanh(x)),
            // `math.floor` answers an Int; the column keeps its Float type.
            same(ods.floor(s), (x) => to_float(math.floor(x))),
            same(ods.abs(s * -1.0), (x) => math.abs(x)),
            same(ods.pow(s, 2.0), (x) => math.pow(x, 2.0)),
            ods.to_list(ods.map(s, "exp")) == ods.to_list(ods.exp(s)),
        ])
    "#);
    yes(r#"
        let s = ods.series([-2.0, 0.0, 2.0])
        let sig = ods.to_list(ods.sigmoid(s))
        let back = ods.to_list(ods.logit(ods.sigmoid(s)))
        sig[1] == 0.5 && math.abs(back[0] + 2.0) < 1e-12 && math.abs(back[2] - 2.0) < 1e-12
            && ods.to_list(ods.sign(s)) == [-1.0, 0.0, 1.0]
            && ods.to_list(ods.round(ods.series([1.25, 2.675, -1.5]), 1)) == [1.3, 2.7, -1.5]
            && ods.to_list(ods.round(ods.series([1234.0]), -2)) == [1200.0]
    "#);
    // An Int column keeps its type where the function does.
    yes(r#"
        let i = ods.series([-3, 4])
        typeof(ods.get(ods.abs(i), 0)) == "Int" && typeof(ods.get(ods.exp(i), 0)) == "Float"
    "#);
    raises("ods.sqrt(ods.series([4.0, -1.0]))", "sqrt: cannot take square root of negative number");
    raises("ods.logit(ods.series([0.5, 1.0]))", "logit: input must be strictly between 0 and 1");
    // A null position never raises.
    yes("ods.null_count(ods.ln(ods.series([1.0, ()]))) == 1");
}

#[test]
fn conditionals_and_nulls() {
    yes(r#"
        let s = ods.series([1.0, (), 3.0, -2.0])
        every([
            ods.to_list(ods.where(s > 0.0, s, 0.0)) == [1.0, (), 3.0, 0.0],
            ods.to_list(ods.coalesce(s, 9.0)) == [1.0, 9.0, 3.0, -2.0],
            ods.to_list(ods.coalesce(s, ods.series([7.0, (), 7.0, 7.0]), 5.0)) == [1.0, 5.0, 3.0, -2.0],
            ods.to_list(ods.clip(s, 0.0, 2.0)) == [1.0, (), 2.0, 0.0],
            ods.to_list(ods.clip(s, (), 2.0)) == [1.0, (), 2.0, -2.0],
            ods.to_list(ods.between(s, 0.0, 3.0)) == [true, (), true, false],
            ods.to_list(ods.between(s, 0.0, 3.0, "neither")) == [true, (), false, false],
            ods.to_list(ods.between(s, ())) == [true, (), true, true],
            ods.to_list(ods.between(s, [0.0, ()])) == [true, (), true, false],
            ods.to_list(ods.not_null(s)) == [true, false, true, true],
            ods.to_list(ods.is_nan(ods.series([1.0, 0.0]) / ods.series([1.0, 2.0]))) == [false, false],
            ods.to_list(ods.select([s > 2.0, s < 0.0], ["big", "neg"], "mid")) == ["mid", "mid", "big", "neg"],
            ods.to_list(ods.select([s > 2.0], [1], ())) == [(), (), 1, ()],
        ])
    "#);
    yes(r#"
        let s = ods.series([(), 1.0, (), (), 4.0, ()])
        every([
            ods.to_list(ods.fill_null(s, "ffill")) == [(), 1.0, 1.0, 1.0, 4.0, 4.0],
            ods.to_list(ods.fill_null(s, "ffill", 1)) == [(), 1.0, 1.0, (), 4.0, 4.0],
            ods.to_list(ods.fill_null(s, "bfill")) == [1.0, 1.0, 4.0, 4.0, 4.0, ()],
            ods.to_list(ods.fill_null(s, "linear")) == [(), 1.0, 2.0, 3.0, 4.0, ()],
            ods.to_list(ods.interpolate(s)) == [(), 1.0, 2.0, 3.0, 4.0, ()],
            ods.to_list(ods.fill_null(s, "mean")) == [2.5, 1.0, 2.5, 2.5, 4.0, 2.5],
            ods.to_list(ods.fill_null(s, "median")) == [2.5, 1.0, 2.5, 2.5, 4.0, 2.5],
            ods.to_list(ods.fill_null(s, 0.0)) == [0.0, 1.0, 0.0, 0.0, 4.0, 0.0],
            ods.to_list(ods.fill_null(ods.series([1, ()]), 2.5)) == [1.0, 2.5],
            ods.to_list(ods.fill_null(ods.series(["a", ()]), "z")) == ["a", "z"],
        ])
    "#);
    yes(r#"
        let hood = ods.series([" north", "SOUTH", "west", ()])
        let m = #{ " north": "North", "SOUTH": "South" }
        every([
            ods.to_list(ods.recode(hood, m)) == ["North", "South", "west", ()],
            ods.to_list(ods.recode(hood, m, ())) == ["North", "South", (), ()],
            ods.to_list(ods.recode(hood, m, "Other")) == ["North", "South", "Other", ()],
            ods.to_list(ods.recode(ods.series([1, 2, 3]), [[1, 10], [2, 20]])) == [10, 20, 3],
            ods.to_list(ods.recode(ods.series([1, 2, 3]), #{ "1": 10.5 })) == [10.5, 2.0, 3.0],
        ])
    "#);
}

#[test]
fn constructors_positions_and_differences() {
    yes(r#"
        every([
            ods.to_list(ods.full(3, "x")) == ["x", "x", "x"],
            ods.to_list(ods.ones(2)) == [1.0, 1.0],
            ods.to_list(ods.arange(4)) == [0, 1, 2, 3],
            ods.to_list(ods.arange(2, 8, 3)) == [2, 5],
            ods.to_list(ods.arange(0.0, 1.0, 0.5)) == [0.0, 0.5],
            ods.to_list(ods.full_like(ods.series([1, 2]), 0.5)) == [0.5, 0.5],
            ods.to_list(ods.repeat(ods.series(["a", "b"]), 2)) == ["a", "a", "b", "b"],
            ods.to_list(ods.repeat(ods.series(["a", "b"]), [1, 3])) == ["a", "b", "b", "b"],
            ods.to_list(ods.tile(ods.series([1, 2]), 3)) == [1, 2, 1, 2, 1, 2],
        ])
    "#);
    yes(r#"
        let s = ods.series([3.0, (), 9.0, 1.0, 9.0])
        every([
            ods.argmax(s) == 2, ods.argmin(s) == 3, ods.argmax(ods.series([()])) == (),
            ods.to_list(ods.sort(s, true)) == [9.0, 9.0, 3.0, 1.0, ()],
            ods.to_list(ods.argsort(s, true)) == [2, 4, 0, 3, 1],
            ods.to_list(ods.argsort(s)) == [3, 0, 2, 4, 1],
            ods.to_list(ods.diff(ods.series([1, 4, 9]))) == [(), 3, 5],
            ods.to_list(ods.diff(ods.series([1, 4, 9]), 2)) == [(), (), 8],
            ods.to_list(ods.pct_change(ods.series([2.0, 3.0]))) == [(), 0.5],
            ods.to_list(ods.cumprod(ods.series([1, 2, (), 3]))) == [1, 2, (), 6],
            ods.mode(ods.series([3, 1, 1, 3, 2])) == 1,
            ods.any(s > 8.0), !ods.all(s > 8.0), ods.all(ods.series([true, ()])),
            ods.searchsorted(ods.series([1, 3, 5]), 4) == 2,
            ods.searchsorted(ods.series([1, 3, 3, 5]), 3, "right") == 3,
            ods.searchsorted(ods.series([1, 3, 5]), [0, 6]) == [0, 3],
            ods.count(s) == 4, ods.first(s) == 3.0, ods.last(ods.series([1, ()])) == (),
        ])
    "#);
}

#[test]
fn reductions_of_expressions_fuse_and_masks_count() {
    yes(r#"
        let a = ods.linspace(-1.0, 1.0, 5001)
        let b = ods.linspace(2.0, 3.0, 5001)
        let fused = ods.sum(a * b + 1.0)
        let eager = ods.sum(ods.series(ods.to_list(a * b + 1.0)))
        let d = ods.dot(a, b)
        math.abs(fused - eager) < 1e-9 && math.abs(d - ods.sum(a * b)) < 1e-9
            && ods.sum(a > 0.0) == 2500 && ods.mean(a > 0.0) == 2500.0 / 5001.0
            && ods.min(a * 2.0) == -2.0 && ods.max(ods.abs(a)) == 1.0
            && ods.count(ods.where(a > 0.0, a, ())) == 2500
    "#);
}

#[test]
fn errors_are_raised_by_the_expression_that_causes_them() {
    // A division is evaluated where it is written, never deferred.
    raises(
        "let n = ods.series([1.0, 2.0])\nlet z = ods.series([1.0, 0.0])\nlet q = (n + 1.0) / z\n0",
        "Division by zero",
    );
    raises("let i = ods.series([9223372036854775807])\nlet j = i + 1\n0", "Integer overflow");
    raises("ods.series([1, 2]) % 0", "Modulo by zero");
    yes("ods.to_list(ods.series([7, -7]) % 3) == [1, -1] && ods.to_list(ods.series([7.5]) % 2.0) == [1.5]");
    raises("ods.exp(ods.series([1.0]) + ods.series([1.0, 2.0]))", "length mismatch");
}

#[test]
fn expressions_resolve_against_a_frame() {
    yes(r#"
        use ods { col }
        let f = ods.frame([["g", ["a", "b", "a", "b"]], ["x", [1.0, (), 3.0, 4.0]], ["y", [1, 2, 3, 4]]])
        let w = f |> ods.with({ z: col("x") * 2.0 + col("y"), m: ods.over(ods.mean(col("x")), "g"), y: col("y") * 10 })
        every([
            ods.columns(w) == ["g", "x", "y", "z", "m"],
            ods.to_list(w["z"]) == [3.0, (), 9.0, 12.0],
            ods.to_list(w["m"]) == [2.0, 4.0, 2.0, 4.0],
            ods.to_list(w["y"]) == [10, 20, 30, 40],
        ])
    "#);
    yes(r#"
        use ods { col }
        let f = ods.frame([["g", ["a", "b", "a", "b"]], ["x", [1.0, (), 3.0, 4.0]], ["y", [1, 2, 3, 4]]])
        let filled = f |> ods.with(#{ "x": ods.fill_null(col("x"), ods.over(ods.mean(col("x")), "g")) })
        let kept = f |> ods.filter(ods.all_of([col("y") > 1, ods.not_null(col("x"))]))
        let picked = f |> ods.select(["g", ods.alias(col("y") * 2, "y2"), #{ "half": col("x") / 2.0 }])
        let sorted = f |> ods.sort_by(["g", col("y") * -1])
        let sorted2 = f |> ods.sort_by(["g", "y"], [false, true])
        every([
            ods.to_list(filled["x"]) == [1.0, 4.0, 3.0, 4.0],
            ods.to_list(kept["y"]) == [3, 4],
            ods.columns(picked) == ["g", "y2", "half"],
            ods.to_list(picked["y2"]) == [2, 4, 6, 8],
            ods.to_list(sorted["y"]) == [3, 1, 4, 2],
            ods.to_list(sorted2["y"]) == [3, 1, 4, 2],
            ods.to_list((f |> ods.sort_by("y", true))["y"]) == [4, 3, 2, 1],
        ])
    "#);
    // Every non-elementwise function takes an Expr, and a Series stands
    // for a literal column.
    yes(r#"
        use ods { col }
        let f = ods.frame([["x", [3.0, 1.0, 2.0]]])
        let w = f |> ods.with({
            r: ods.rank(col("x")),
            prev: ods.shift(col("x"), 1),
            dev: col("x") - ods.mean(col("x")),
            lit: ods.series([7, 8, 9]),
            k: 5,
        })
        every([
            ods.to_list(w["r"]) == [3, 1, 2],
            ods.to_list(w["prev"]) == [(), 3.0, 1.0],
            ods.to_list(w["dev"]) == [1.0, -1.0, 0.0],
            ods.to_list(w["lit"]) == [7, 8, 9],
            ods.to_list(w["k"]) == [5, 5, 5],
        ])
    "#);
    yes(r#"
        use ods { col, lit }
        let e = col("a") * 2 + lit(1)
        to_string(e) == "((col(\"a\") * 2) + 1)" && typeof(e) == "Expr"
    "#);
    raises(
        "use ods { col }\nlet f = ods.frame([[\"x\", [1]]])\nods.with(f, { y: col(\"nope\") })",
        "no column 'nope' in this Frame. It has: x",
    );
    raises(
        "use ods { col }\nlet f = ods.frame([[\"x\", [1]]])\nods.with(f, { y: ods.over(col(\"x\"), \"x\") })",
        "over takes an aggregation",
    );
    raises(
        "let f = ods.frame([[\"x\", [1]]])\nods.select(f, [ods.lit(1)])",
        "has no column name",
    );
}

#[test]
fn promoted_functions_build_and_resolve_expressions() {
    // The functions promote (threshold 2) and their Exprs and lazy
    // Series cross the tier boundary as the same values.
    yes(r#"
        use ods { col }
        fn feature(f) = ods.with(f, { z: ods.sigmoid(col("x") / 2.0), w: ods.where(col("x") > 1.0, col("x"), 0.0) })
        fn score(s) = ods.sum(ods.exp(s) * 2.0 + 1.0)
        let f = ods.frame([["x", [0.0, 2.0, 4.0]]])
        let a = feature(f)
        let b = feature(f)
        let c = feature(f)
        let s = ods.series([0.0, 1.0])
        let t1 = score(s)
        let t2 = score(s)
        let t3 = score(s)
        ods.to_list(c["w"]) == [0.0, 2.0, 4.0] && ods.get(c["z"], 0) == 0.5
            && t3 == 2.0 * (1.0 + math.exp(1.0)) + 2.0
    "#);
}
