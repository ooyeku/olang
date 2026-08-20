//! The `meta fn` / `@` macro system (docs/macros.md), end to end.
//!
//! Expansion runs inside `Parser::parse`, so these tests exercise the
//! same path every runner uses: parse (which expands) and evaluate. The
//! laws under test: every site says `@`; the parse is total (arguments
//! are ordinary olang, `@` in a string is data); expansion is pure
//! (effectful modules refuse in meta mode); meta fns never reach the
//! runtime; errors name the macro and the site; and the tiers see only
//! the expanded program.

use olang::ast::Value;
use olang::{Interpreter, Parser};
use std::process::Command;

fn eval(source: &str) -> Result<Value, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    Interpreter::new()
        .eval_program(program)
        .map_err(|e| e.to_string())
}

// ── the invocation forms ──────────────────────────────────────────────

#[test]
fn an_expression_macro_expands_at_its_call_site() {
    let out = eval(
        "meta fn unless(cond, body) = `if !(${cond}) => ${body} else => ()`\n\
         let mut hits = 0\n\
         @unless(1 > 5, { hits = hits + 1 })\n\
         @unless(1 > 0, { hits = hits + 100 })\n\
         hits\n",
    )
    .expect("expansion");
    assert_eq!(out, Value::Integer(1));
}

#[test]
fn a_decorator_transforms_a_type_declaration() {
    // The derive shape: the macro reads the declaration's fields through
    // meta.parse and returns the declaration plus generated functions.
    let out = eval(
        "meta fn getters(decl) = {\n\
             let t = head(unwrap(meta.parse(decl)))\n\
             let fields = map_get(t, \"fields\") |> map((f) => map_get(f, \"name\"))\n\
             let fns = fields |> map((f) => `fn get_${f}(v) = v.${f}`) |> join(\"\\n\")\n\
             `${decl}\n${fns}`\n\
         }\n\
         @getters\n\
         type P = struct { x: Int, y: Int }\n\
         get_x(P { x: 7, y: 9 }) + get_y(P { x: 7, y: 9 })\n",
    )
    .expect("derive");
    assert_eq!(out, Value::Integer(16));
}

#[test]
fn stacked_decorators_apply_nearest_first() {
    // @outer sees what @inner produced: the innermost decorator runs
    // first, its output becomes the next one's input.
    let out = eval(
        "meta fn inner(decl) = `${decl}\nfn tag() = \"inner\"`\n\
         meta fn outer(decl) = `${decl}\nfn count() = ${show(len(decl))}`\n\
         @outer\n\
         @inner\n\
         type T = struct { a: Int }\n\
         tag()\n",
    )
    .expect("stacked");
    assert_eq!(out, Value::String("inner".to_string().into()));
}

// ── composition and termination ───────────────────────────────────────

#[test]
fn a_macro_may_produce_macro_calls_for_the_next_round() {
    let out = eval(
        "meta fn twice(e) = `(${e}) + (${e})`\n\
         meta fn quad(e) = `@twice(@twice(${e}))`\n\
         @quad(3)\n",
    )
    .expect("composition");
    assert_eq!(out, Value::Integer(12));
}

#[test]
fn nested_sites_expand_innermost_first() {
    let out = eval(
        "meta fn neg(e) = `(0 - (${e}))`\n\
         @neg(@neg(5))\n",
    )
    .expect("nesting");
    assert_eq!(out, Value::Integer(5));
}

#[test]
fn runaway_expansion_is_stopped_by_fuel_with_the_site_named() {
    let err = eval(
        "meta fn forever(e) = `@forever(${e})`\n\
         @forever(1)\n",
    )
    .expect_err("must not loop");
    assert!(
        err.contains("did not terminate") && err.contains("@forever"),
        "{err}"
    );
}

// ── the laws ──────────────────────────────────────────────────────────

#[test]
fn an_at_sign_inside_a_string_is_data_not_a_macro() {
    let out = eval("let s = \"user@example.com\"\nlen(s)\n").expect("string data");
    assert_eq!(out, Value::Integer(16));
}

#[test]
fn a_meta_fn_never_exists_at_runtime() {
    // Calling a meta fn as an ordinary function must fail: it was
    // stripped with the expansion. (`@`-less call, runtime resolution.)
    let err = eval(
        "meta fn helper(x) = `${x}`\n\
         let y = @helper(1)\n\
         helper(2)\n",
    )
    .expect_err("meta fns are expansion-only");
    assert!(err.contains("helper"), "{err}");
}

#[test]
fn meta_mode_refuses_the_effectful_world() {
    for (call, what) in [
        ("unwrap(fs.read_file(\"/etc/hosts\"))", "fs."),
        ("show(time.monotonic_ms())", "time."),
        ("show(random.random())", "random."),
    ] {
        let err = eval(&format!(
            "meta fn evil(x) = {{ let v = {call}; `1` }}\nlet y = @evil(0)\n"
        ))
        .expect_err("purity");
        assert!(
            err.contains("not available at expansion time"),
            "{what} must refuse at expansion time: {err}"
        );
    }
}

#[test]
fn meta_parse_shows_source_as_written_including_macro_sites() {
    // The Open AST is the pre-expansion view: a macro call is a node, not
    // its expansion.
    let out = eval(
        "let nodes = unwrap(meta.parse(\"let x = @foo(1)\"))\n\
         let stmt = head(nodes)\n\
         map_get(map_get(stmt, \"value\"), \"kind\")\n",
    )
    .expect("meta.parse raw view");
    assert_eq!(out, Value::String("macro_call".to_string().into()));
}

// ── errors name the macro and the site ────────────────────────────────

#[test]
fn an_unknown_macro_names_itself_and_its_line() {
    let err = eval("let x = @nope(1)\n").expect_err("unknown macro");
    assert!(err.contains("@nope") && err.contains("line 1"), "{err}");
}

#[test]
fn a_raising_meta_fn_is_blamed_at_the_call_site() {
    let err = eval(
        "meta fn boom(x) = head([])\n\
         let y = @boom(1)\n",
    )
    .expect_err("raise");
    assert!(err.contains("@boom") && err.contains("line 2"), "{err}");
}

#[test]
fn generated_source_that_does_not_parse_is_blamed_with_the_text_shown() {
    let err = eval(
        "meta fn bad(x) = `let let let`\n\
         let y = @bad(1)\n",
    )
    .expect_err("bad generation");
    assert!(
        err.contains("@bad") && err.contains("generated") && err.contains("let let let"),
        "{err}"
    );
}

#[test]
fn a_meta_fn_must_return_source_text() {
    let err = eval(
        "meta fn wrong(x) = 42\n\
         let y = @wrong(1)\n",
    )
    .expect_err("non-string return");
    assert!(err.contains("must return source text"), "{err}");
}

// ── the new meta builtins ─────────────────────────────────────────────

#[test]
fn meta_eval_computes_at_expansion_time_and_meta_lit_splices_it() {
    // @bake in userland: the whole comptime feature in one meta fn.
    let out = eval(
        "meta fn bake(expr) = meta.lit(unwrap(meta.eval(expr)))\n\
         @bake([1, 2, 3] |> map((n) => n * n) |> fold(0, (a, b) => a + b))\n",
    )
    .expect("bake");
    assert_eq!(out, Value::Integer(14));
}

#[test]
fn meta_lit_round_trips_every_literal_shape() {
    let out = eval(
        "let v = [1, 2.5, true, (), \"a\\\"b\", (3, \"x\"), #{ \"k\": [1] }]\n\
         unwrap(meta.eval(meta.lit(v))) == v\n",
    )
    .expect("round trip");
    assert_eq!(out, Value::Boolean(true));
}

#[test]
fn meta_fresh_yields_distinct_names() {
    let out = eval("meta.fresh(\"tmp\") != meta.fresh(\"tmp\")\n").expect("fresh");
    assert_eq!(out, Value::Boolean(true));
}

// ── tiers and tooling ─────────────────────────────────────────────────

#[test]
fn the_tiers_agree_on_an_expanded_program() {
    let source = "meta fn twice(e) = `(${e}) + (${e})`\n\
                  fn hot(n) = if n <= 0 => 0 else => @twice(n) + hot(n - 1)\n\
                  hot(50)\n";
    let interpreted = {
        let program = Parser::new().parse(source).expect("parse");
        Interpreter::new().eval_program(program).expect("interp")
    };
    let tiered = {
        let program = Parser::new().parse(source).expect("parse");
        let mut interp = Interpreter::new();
        interp.enable_bytecode_tier(1, false);
        interp.eval_program(program).expect("tiered")
    };
    assert_eq!(interpreted, tiered);
}

#[test]
fn olang_expand_prints_the_expanded_program_with_lines_preserved() {
    let dir = std::env::temp_dir().join(format!("olang_expand_cli_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let file = dir.join("m.ol");
    std::fs::write(
        &file,
        "meta fn unless(c, b) = `if !(${c}) => ${b} else => ()`\n\
         let x = 1\n\
         @unless(x > 5, println(\"small\"))\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("expand")
        .arg(&file)
        .output()
        .expect("expand runs");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    // The site is expanded, the meta fn is gone, and the line count is
    // unchanged — stripping blanks the declaration without shifting
    // anything below it.
    assert!(text.contains("if !(x > 5) => println(\"small\") else => ()"));
    assert!(!text.contains("meta fn"));
    assert_eq!(text.lines().count(), 3, "line count must be preserved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_macro_free_file_is_untouched_by_expansion() {
    let source = "let x = 1\nprintln(to_string(x))\n";
    let program = Parser::new().parse(source).expect("parse");
    assert!(!olang::expand::program_uses_macros(&program));
    assert_eq!(olang::expand::expand_source(source).expect("no-op"), source);
}
