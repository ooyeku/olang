//! The bundled collections (Campaign 6): the olang-written data
//! structures and algorithms, their mutation primitives, and the
//! machinery that makes both fast.
//!
//! Three layers under test. The Rust primitives: `col.set`/`col.swap`/
//! `col.filled` and the two assignment fusions (indexed write, move
//! call), which must be invisible — identical answers on both tiers,
//! aliased handles snapshotting instead of corrupting. The modules:
//! every embedded `test` block runs here, so the suite fails if a
//! bundled module's own tests fail. And the properties: differential
//! checks against the builtin map, sorting invariants against a Rust
//! reference, heap drain order — on seeded pseudo-random inputs big
//! enough to cross growth boundaries.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier: bool) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let parser = Parser::new();
            let program = parser.parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .expect("spawn")
        .join()
        .expect("join")
}

/// Same result on both tiers, and that result returned.
fn eval_identical(source: &str) -> Value {
    let interp = eval(source, false).unwrap_or_else(|e| panic!("interpreted: {}", e));
    let tiered = eval(source, true).unwrap_or_else(|e| panic!("tiered: {}", e));
    assert_eq!(interp, tiered, "tiers disagree\n  source: {}", source);
    interp
}

fn shows(source: &str) -> String {
    match eval_identical(&format!("to_string({})", source.trim_end())) {
        Value::String(s) => s.as_ref().clone(),
        other => panic!("expected a string, got {:?}", other),
    }
}

// ── the mutation primitives ───────────────────────────────────────────

#[test]
fn col_set_swap_filled_semantics() {
    assert_eq!(shows("col.set([1, 2, 3], 1, 9)"), "[1, 9, 3]");
    assert_eq!(shows("col.set([1, 2, 3], -1, 9)"), "[1, 2, 9]");
    assert_eq!(shows("col.swap([1, 2, 3], 0, 2)"), "[3, 2, 1]");
    assert_eq!(shows("col.filled(3, 7)"), "[7, 7, 7]");
    assert_eq!(shows("col.filled(0, 7)"), "[]");
}

#[test]
fn col_set_errors_are_identical_on_both_tiers() {
    for bad in [
        "let mut x = [1]\nx = col.set(x, 5, 0)",
        "let mut x = [1]\nx = col.swap(x, 0, 9)",
        "col.filled(-1, 0)",
    ] {
        let a = eval(bad, false).expect_err("must raise");
        let b = eval(bad, true).expect_err("must raise");
        assert_eq!(a, b, "error text diverged for {}", bad);
        assert!(a.contains("col."), "{}", a);
    }
}

#[test]
fn fused_writes_agree_with_copies_and_snapshots_hold() {
    // The fused in-place path and the aliased copy path must produce
    // the same answer, and the alias must keep its snapshot.
    let src = "\
let mut a = col.filled(50, 0)\n\
let snapshot = a\n\
for i in range(0, 200) {\n\
    a = col.set(a, i % 50, i)\n\
    a = col.swap(a, i % 50, (i + 13) % 50)\n\
}\n\
to_string([a[0], a[25], a[49], snapshot[0], snapshot[25], len(snapshot)])";
    let got = eval_identical(src);
    let Value::String(s) = got else { panic!() };
    // The snapshot stayed all zeros regardless of 400 fused writes.
    assert!(s.ends_with("0, 0, 50]"), "{}", s);
}

#[test]
fn a_shadowed_col_is_not_fused() {
    // A user's own `col` value must win: the fusion checks the binding
    // resolves to the real builtin before touching anything.
    let src = "\
let col = #{ \"set\": 1 }\n\
map_get(col, \"set\")";
    assert_eq!(eval_identical(src), Value::Integer(1));
}

#[test]
fn the_move_call_keeps_a_held_handle_intact() {
    // `h = f(h)` moves the handle — but a second binding taken before
    // the call is a snapshot, not a casualty.
    let src = "\
fn bump(xs) = col.set(xs, 0, 99)\n\
let mut h = [1, 2]\n\
let held = h\n\
h = bump(h)\n\
to_string([h[0], held[0]])";
    assert_eq!(shows_of(eval_identical(src)), "[99, 1]");
}

fn shows_of(v: Value) -> String {
    match v {
        Value::String(s) => s.as_ref().clone(),
        other => panic!("expected string, got {:?}", other),
    }
}

// ── automatic availability ────────────────────────────────────────────

#[test]
fn bundled_modules_load_with_no_use() {
    assert_eq!(
        eval_identical("let mut h = heap.new()\nh = heap.push(h, 1, \"x\")\nheap.top_item(h)"),
        Value::String(std::sync::Arc::new("x".to_string()))
    );
}

#[test]
fn a_user_binding_of_a_module_name_wins() {
    assert_eq!(eval_identical("let heap = 7\nheap + 1"), Value::Integer(8));
}

#[test]
fn explicit_use_remains_equivalent() {
    assert_eq!(
        eval_identical("use dsu\nlet d = dsu.new(3)\ndsu.groups(d)"),
        Value::Integer(3)
    );
}

// ── the embedded test blocks run here ─────────────────────────────────

#[test]
fn every_bundled_module_passes_its_own_tests() {
    for module in ["heap", "deque", "bitset", "dsu", "table", "alg"] {
        let program = olang::stdlib::embedded::parsed(module)
            .expect("parse")
            .unwrap_or_else(|| panic!("{} is not embedded", module));
        // Outside `olang test`, test blocks execute inline as the
        // program evaluates — raises abort, but a failed
        // `testing.assert_eq` only increments the thread-local tally
        // (the runner's contract). Check both: the eval result for
        // raises, the tally delta for quiet assertion failures.
        let (_, failed_before) = olang::stdlib::testing::tally_snapshot();
        let mut interpreter = Interpreter::new();
        interpreter
            .eval_program((*program).clone())
            .unwrap_or_else(|e| panic!("{}: load or test block failed: {:?}", module, e));
        let (_, failed_after) = olang::stdlib::testing::tally_snapshot();
        assert_eq!(
            failed_after - failed_before,
            0,
            "{}: {} assertion(s) failed in its test blocks",
            module,
            failed_after - failed_before
        );
        let mut declared = 0;
        for statement in &program.statements {
            let mut stmt = statement;
            while let olang::ast::Statement::Located { stmt: inner, .. } = stmt {
                stmt = inner;
            }
            if matches!(stmt, olang::ast::Statement::TestDecl(_)) {
                declared += 1;
            }
        }
        assert!(declared >= 3, "{} ships fewer than 3 test blocks", module);
    }
}

// ── properties on seeded inputs ───────────────────────────────────────

#[test]
fn heap_drains_in_sorted_order() {
    let src = "\
let n = 2000\n\
let mut h = heap.new()\n\
for i in range(0, n) {\n\
    h = heap.push(h, (i * 1103515245 + 12345) % 99991, i)\n\
}\n\
let mut prios = []\n\
while !heap.is_empty(h) {\n\
    prios = prios + [heap.top_prio(h)]\n\
    h = heap.pop(h)\n\
}\n\
let mut sorted = true\n\
for i in range(1, len(prios)) {\n\
    if prios[i] < prios[i - 1] => { sorted = false }\n\
}\n\
to_string([len(prios), sorted])";
    assert_eq!(shows_of(eval_identical(src)), "[2000, true]");
}

#[test]
fn table_matches_the_builtin_map() {
    // The same seeded workload of puts, overwrites, and removes against
    // the builtin persistent map: sizes and every surviving key agree.
    let src = "\
let mut t = table.new()\n\
let mut m = #{}\n\
for i in range(0, 600) {\n\
    let k = \"k\" + to_string((i * 31 + 7) % 97)\n\
    if i % 5 == 4 => {\n\
        t = table.remove(t, k)\n\
        m = map_remove(m, k)\n\
    }\n\
    else => {\n\
        t = table.put(t, k, i)\n\
        m = map_set(m, k, i)\n\
    }\n\
}\n\
let mut agree = table.size(t) == len(map_keys(m))\n\
for k in map_keys(m) {\n\
    if table.get(t, k) != map_get(m, k) => { agree = false }\n\
}\n\
to_string(agree)";
    assert_eq!(shows_of(eval_identical(src)), "true");
}

#[test]
fn alg_sort_matches_a_rust_reference() {
    // Seeded values sorted by alg.sort must equal the Rust sort of the
    // same sequence.
    let n: i64 = 1500;
    let mut reference: Vec<i64> = (0..n).map(|i| (i * 1103515245 + 12345) % 10007).collect();
    reference.sort();
    let want = format!(
        "[{}]",
        reference
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let src = format!(
        "let xs = map(range(0, {}), (i) => (i * 1103515245 + 12345) % 10007)\nto_string(alg.sort(xs))",
        n
    );
    assert_eq!(shows_of(eval_identical(&src)), want);
}

#[test]
fn dijkstra_agrees_with_bfs_on_unit_weights() {
    // On a graph whose weights are all 1, shortest path length is hop
    // count: dijkstra and bfs must answer identically.
    let src = "\
let n = 60\n\
let mut unweighted = []\n\
let mut weighted = []\n\
for i in range(0, n) {\n\
    let a = (i * 13 + 5) % n\n\
    let b = (i * 7 + 11) % n\n\
    unweighted = unweighted + [[a, b]]\n\
    weighted = weighted + [[a, b, 1]]\n\
}\n\
to_string(alg.bfs(alg.graph(n, unweighted), 0) == alg.dijkstra(alg.wgraph(n, weighted), 0))";
    assert_eq!(shows_of(eval_identical(src)), "true");
}

#[test]
fn dsu_agrees_with_reachability() {
    // Union a seeded edge set, then check connectivity for every pair
    // against bfs over the same edges (made bidirectional).
    let src = "\
let n = 40\n\
let mut d = dsu.new(n)\n\
let mut edges = []\n\
for i in range(0, 30) {\n\
    let a = (i * 17 + 3) % n\n\
    let b = (i * 23 + 9) % n\n\
    d = dsu.union(d, a, b)\n\
    edges = edges + [[a, b], [b, a]]\n\
}\n\
let g = alg.graph(n, edges)\n\
let mut agree = true\n\
for u in range(0, n) {\n\
    let dist = alg.bfs(g, u)\n\
    for v in range(0, n) {\n\
        if dsu.connected(d, u, v) != (dist[v] != -1) => { agree = false }\n\
    }\n\
}\n\
to_string(agree)";
    assert_eq!(shows_of(eval_identical(src)), "true");
}
