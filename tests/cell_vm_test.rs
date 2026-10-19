//! A cell the bytecode VM writes keeps the VM's own value (src/stdlib/cell.rs
//! `vm_call`): read back by that VM unconverted, by the tree-walker (and any
//! other VM) converted, as a value crossing the tiers is. These pin that both
//! sides see the same values whichever tier wrote them.

use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn run(source: &str) -> Result<String, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    interpreter
        .eval_program(program)
        .map(|v| match v {
            olang::ast::Value::String(ref s) => s.to_string(),
            other => other.to_string(),
        })
        .map_err(|e| e.to_string())
}

#[test]
fn a_table_grown_on_the_bytecode_tier_reads_the_same_on_the_interpreter() {
    // `put` and `look` run hot (compiled); the last lines run on the
    // tree-walker: the nested values come back whole
    let out = run("let memo = cell(#{})\n\
                   fn put(k, i) = cell.set(memo, map_set(cell.take(memo), k, (#{ \"role\": \"row\", \"kids\": [#{ \"text\": \"t\" + to_string(i) }] }, [i, i + 1], #{ \"nodes\": [i] })))\n\
                   fn look(k) = map_get(cell.get(memo), k)\n\
                   for i in 0..400 { put(\"k\" + to_string(i), i) }\n\
                   let mut n = 0\n\
                   for i in 0..400 { if look(\"k\" + to_string(i))[1][0] == i => { n = n + 1 } }\n\
                   let e = map_get(cell.get(memo), \"k7\")\n\
                   [n, map_len(cell.get(memo)), e[0].kids[0].text, e[1], e[2].nodes]\n")
        .unwrap();
    assert_eq!(out, r#"[400, 400, "t7", [7, 8], [7]]"#);
}

#[test]
fn a_value_the_interpreter_set_is_read_on_the_bytecode_tier_and_back() {
    let out = run("let c = cell([1, #{ \"a\": [2, 3] }])\n\
                   fn grow(i) = cell.set(c, cell.take(c) + [i])\n\
                   fn size() = len(cell.get(c))\n\
                   for i in 0..300 { grow(i) }\n\
                   let mut s = 0\n\
                   for i in 0..300 { s = size() }\n\
                   let v = cell.get(c)\n\
                   [s, v[1].a, v[301]]\n")
    .unwrap();
    assert_eq!(out, "[302, [2, 3], 299]");
}

#[test]
fn a_function_kept_by_the_bytecode_tier_is_called_by_both() {
    let out = run("let c = cell(())\n\
                   fn keep(k) = cell.set(c, #{ \"f\": (x) => x * k, \"k\": k })\n\
                   fn call(x) = (cell.get(c).f)(x)\n\
                   for i in 0..300 { keep(3) }\n\
                   let mut a = 0\n\
                   for i in 0..300 { a = call(i) }\n\
                   let g = cell.get(c).f\n\
                   [a, g(5), cell.update(c, (m) => map_set(m, \"k\", m.k + 1)).k, call(2)]\n")
    .unwrap();
    assert_eq!(out, "[897, 15, 4, 6]");
}

#[test]
fn take_and_show_a_cell_the_bytecode_tier_wrote() {
    let out = run("let c = cell(0)\n\
                   fn put(i) = cell.set(c, [i, \"x\"])\n\
                   fn grab() = cell.take(c)\n\
                   for i in 0..300 { put(i) }\n\
                   let shown = to_string(c)\n\
                   let mut got = ()\n\
                   for i in 0..300 { put(i); got = grab() }\n\
                   [shown, got, cell.get(c)]\n")
    .unwrap();
    assert_eq!(out, r#"["cell([299, \"x\"])", [299, "x"], ()]"#);
}

#[test]
fn a_cell_made_on_the_bytecode_tier_holds_its_value() {
    let out = run("fn make(i) = cell.new([i])\n\
                   fn read(c) = cell.get(c)\n\
                   let mut last = ()\n\
                   for i in 0..300 { last = make(i) }\n\
                   let mut v = ()\n\
                   for i in 0..300 { v = read(last) }\n\
                   [v, cell.get(last)]\n")
    .unwrap();
    assert_eq!(out, "[[299], [299]]");
}
