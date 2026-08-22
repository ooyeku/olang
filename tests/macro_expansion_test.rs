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

// ── hardening regressions from the first adversarial probe ────────────

#[test]
fn unicode_in_a_meta_fn_body_survives_stripping() {
    // Stripping blanks whole spans byte-by-byte; multibyte characters in
    // a meta fn's body (π here) must not corrupt the text.
    let out = eval(
        "meta fn greet(name) = `\"héllo π, \" + ${name}`\n\
         @greet(\"wörld\")\n",
    )
    .expect("unicode strip");
    assert_eq!(out, Value::String("héllo π, wörld".to_string().into()));
}

#[test]
fn output_with_a_trailing_comment_is_refused_at_the_site() {
    // `2 + 1 // note` parses alone but swallows the call site's `)` when
    // spliced inline. The paren-probe validation catches it and blames
    // the macro, instead of an unattributed whole-file parse error.
    let err = eval(
        "meta fn f(x) = `${x} + 1  // note`\n\
         println(to_string(@f(2)))\n",
    )
    .expect_err("trailing comment must be refused");
    assert!(
        err.contains("@f") && err.contains("does not splice"),
        "{err}"
    );
}

#[test]
fn multi_line_block_output_splices_into_an_expression() {
    let out = eval(
        "meta fn wrap(e) = `{\n    let v = ${e}\n    v + v\n}`\n\
         @wrap(21)\n",
    )
    .expect("block output");
    assert_eq!(out, Value::Integer(42));
}

// ── M1: placement rules ───────────────────────────────────────────────

#[test]
fn a_meta_fn_inside_a_block_is_refused() {
    let err = eval("fn outer() = {\n    meta fn inner(x) = `${x}`\n    1\n}\nouter()\n")
        .expect_err("nested meta fn");
    assert!(err.contains("top-level declaration"), "{err}");
}

#[test]
fn a_macro_call_inside_a_meta_fn_body_is_refused() {
    let err = eval("meta fn a(x) = `${x}`\nmeta fn b(x) = @a(x)\nprintln(to_string(@b(1)))\n")
        .expect_err("@ inside meta fn");
    assert!(
        err.contains("inside a meta fn body") && err.contains("directly"),
        "{err}"
    );
}

#[test]
fn share_meta_fn_is_an_error_not_a_silent_export() {
    // Not supported: whatever the failure shape, it must be an error —
    // never a silently ignored modifier.
    assert!(eval("share meta fn f(x) = `${x}`\nprintln(to_string(@f(1)))\n").is_err());
}

// ── M1: determinism ───────────────────────────────────────────────────

#[test]
fn expansion_is_deterministic_within_a_process() {
    // meta.fresh resets per expansion, so expanding the same source
    // twice — in one process — yields byte-identical programs.
    let src = "meta fn tmp(e) = {\n    let n = meta.fresh(\"t\")\n    `{ let ${n} = ${e}; ${n} + ${n} }`\n}\nprintln(to_string(@tmp(5)))\n";
    let a = olang::expand::expand_source(src).expect("first");
    let b = olang::expand::expand_source(src).expect("second");
    assert_eq!(a, b, "same source must expand identically every time");
}

// ── M1: the prefilter is precise ──────────────────────────────────────

#[test]
fn mentioning_meta_or_at_in_data_does_not_trigger_expansion() {
    // "meta" in a comment, "@" in a string: neither is a macro construct.
    let out = eval(
        "// meta.parse is discussed here, and meta  fn is mentioned in prose\n\
         let email = \"user@example.com\"\n\
         email\n",
    )
    .expect("no expansion");
    assert_eq!(out, Value::String("user@example.com".to_string().into()));
    assert!(!olang::expand::has_meta_fn_token(
        "// the meta fns are meta.parse"
    ));
    assert!(olang::expand::has_meta_fn_token("meta fn f(x) = `${x}`"));
    assert!(olang::expand::has_meta_fn_token("meta\t fn g() = `1`"));
    assert!(!olang::expand::has_meta_fn_token("metadata fn = 1"));
}

// ── M2: template escapes ──────────────────────────────────────────────

#[test]
fn template_escapes_backtick_dollar_backslash() {
    let out = eval("`tick \\` dollar \\${x} back \\\\ raw \\n`").expect("escapes");
    assert_eq!(
        out,
        Value::String("tick ` dollar ${x} back \\ raw \\n".to_string().into())
    );
}

#[test]
fn a_macro_can_generate_a_template() {
    // The capability the escapes exist for: macro output containing a
    // template that interpolates at *runtime*, not at expansion.
    let out = eval(
        "meta fn logfmt(tag) = `(m) => \\`[${tag}] \\${m}\\``\n\
         let log = @logfmt(app)\n\
         log(\"started\")\n",
    )
    .expect("template generation");
    assert_eq!(out, Value::String("[app] started".to_string().into()));
}

// ── M2: decorators on fn and let ──────────────────────────────────────

#[test]
fn a_decorator_on_a_fn_can_wrap_it() {
    let out = eval(
        "meta fn noisy(decl) = {\n\
             let node = head(unwrap(meta.parse(decl)))\n\
             let name = map_get(node, \"name\")\n\
             let impl_name = meta.fresh(name)\n\
             let params = map_get(node, \"params\") |> join(\", \")\n\
             let renamed = str.replace(decl, `fn ${name}(`, `fn ${impl_name}(`)\n\
             renamed + `\nfn ${name}(${params}) = ${impl_name}(${params}) * 10`\n\
         }\n\
         @noisy\n\
         fn base(x) = x + 1\n\
         base(4)\n",
    )
    .expect("fn decorator");
    assert_eq!(out, Value::Integer(50));
}

#[test]
fn a_decorator_on_a_let_receives_the_declaration() {
    let out = eval(
        "meta fn doubled(decl) = str.replace(decl, \"= \", \"= 2 * \")\n\
         @doubled\n\
         let x = 21\n\
         x\n",
    )
    .expect("let decorator");
    assert_eq!(out, Value::Integer(42));
}

// ── M2: imported macro libraries ──────────────────────────────────────

#[test]
fn a_use_import_brings_meta_fns_into_expansion() {
    let dir = std::env::temp_dir().join(format!("olang_macro_lib_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    std::fs::write(
        dir.join("mylib.ol"),
        "meta fn twice(e) = `((${e}) + (${e}))`\nshare fn unused() = 1\n",
    )
    .expect("lib");
    std::fs::write(
        dir.join("main.ol"),
        "use mylib\nprintln(to_string(@twice(20 + 1)))\n",
    )
    .expect("main");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg("main.ol")
        .current_dir(&dir)
        .output()
        .expect("runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("42"),
        "imported macro must expand: stdout={stdout} stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    // A local meta fn of the same name shadows the imported one.
    std::fs::write(
        dir.join("main2.ol"),
        "use mylib\nmeta fn twice(e) = `(100 * (${e}))`\nprintln(to_string(@twice(2)))\n",
    )
    .expect("main2");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg("main2.ol")
        .current_dir(&dir)
        .output()
        .expect("runs");
    assert!(String::from_utf8_lossy(&out.stdout).contains("200"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── M2: REPL persistence ──────────────────────────────────────────────

#[test]
fn the_repl_keeps_meta_fns_across_inputs() {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("repl");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"meta fn triple(e) = `(3 * (${e}))`\nprintln(to_string(@triple(14)))\n:quit\n")
        .expect("write");
    let out = child.wait_with_output().expect("repl exits");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("42"),
        "meta fn defined on one line must be usable on a later line: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// ── M3: expand --diff and error alignment ─────────────────────────────

#[test]
fn expand_diff_shows_only_changed_lines() {
    let dir = std::env::temp_dir().join(format!("olang_expand_diff_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let f = dir.join("d.ol");
    std::fs::write(
        &f,
        "meta fn inc(e) = `((${e}) + 1)`\nlet untouched = 1\nprintln(to_string(@inc(41)))\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["expand", f.to_str().unwrap(), "--diff"])
        .output()
        .expect("expand");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("+ println(to_string(((41) + 1)))"), "{text}");
    assert!(
        !text.contains("untouched"),
        "unchanged lines must not appear in the diff: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_runtime_error_in_generated_code_notes_the_expansion() {
    let dir = std::env::temp_dir().join(format!("olang_expand_err_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let f = dir.join("e.ol");
    std::fs::write(
        &f,
        "meta fn boom(e) = `(${e}) + nope_undefined`\nprintln(to_string(@boom(1)))\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg(f.to_str().unwrap())
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), err);
    assert!(
        all.contains("nope_undefined")
            && all.contains("generated by @boom")
            && all.contains("olang expand"),
        "error must be source-mapped to the @ site with the macro named: {all}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── edge shapes ───────────────────────────────────────────────────────

#[test]
fn edge_shapes_expand_correctly() {
    // No-argument macro.
    let out = eval("meta fn answer() = `42`\n@answer()\n").expect("no args");
    assert_eq!(out, Value::Integer(42));
    // Macro call inside a match arm and a lambda body.
    let out = eval(
        "meta fn inc(e) = `((${e}) + 1)`\n\
         let f = (x) => match x { 1 => @inc(40), other => 0 }\n\
         f(1)\n",
    )
    .expect("nested positions");
    assert_eq!(out, Value::Integer(41));
    // Decorators separated by comments.
    let out = eval(
        "meta fn keep(decl) = decl\n\
         @keep\n\
         // a comment between decorator and declaration\n\
         @keep\n\
         type P = struct { x: Int }\n\
         P { x: 7 }.x\n",
    )
    .expect("comment between decorators");
    assert_eq!(out, Value::Integer(7));
    // A macro inside a test block body parses and expands.
    let out = eval(
        "meta fn four() = `4`\n\
         test \"macros in tests\" { assert_eq(@four(), 4) }\n\
         @four()\n",
    )
    .expect("macro in test block");
    assert_eq!(out, Value::Integer(4));
}

// ── source-mapped runtime error spans ─────────────────────────────────

#[test]
fn an_error_in_generated_code_points_at_the_call_site() {
    // The error happens inside @boom's output; the rendered error must
    // show the ORIGINAL file with the caret at the @ site's line, name
    // the macro, and offer `olang expand` for the generated text.
    let dir = std::env::temp_dir().join(format!("olang_srcmap_a_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let f = dir.join("a.ol");
    std::fs::write(
        &f,
        "meta fn boom(e) = `(${e}) + nope_undefined`\nlet x = 1\nprintln(to_string(@boom(x)))\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg(f.to_str().unwrap())
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    // The caret frame shows the original source: the @ call line, not
    // the expanded `(x) + nope_undefined` text.
    assert!(
        err.contains("@boom(x)") && err.contains("generated by @boom"),
        "must display the original file at the @ site: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_error_after_a_line_shifting_decorator_maps_to_the_true_line() {
    // The decorator appends three declarations, shifting every later
    // line of the expanded program. The error in UNTOUCHED code must
    // still point at its original line, showing original content.
    let dir = std::env::temp_dir().join(format!("olang_srcmap_b_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let f = dir.join("b.ol");
    std::fs::write(
        &f,
        "meta fn grow(decl) = decl + `\nfn extra_one() = 1\nfn extra_two() = 2\nfn extra_three() = 3`\n\
         @grow\ntype P = struct { x: Int }\nlet fine = extra_one() + extra_two()\n\
         println(to_string(undefined_after_shift))\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("run")
        .arg(f.to_str().unwrap())
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("undefined_after_shift")
            && err.contains("8 │ println(to_string(undefined_after_shift))"),
        "the caret must sit on original line 8, not the shifted expanded line: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_line_map_tracks_origins_through_rounds() {
    // Library-level check of the map itself: original lines stay
    // Original with their own numbers; a macro-produced line is
    // Generated, rooted at the @ site — even through composition, where
    // one macro's output calls another.
    use olang::expand::{LineOrigin, expand_source_mapped};
    let exp = expand_source_mapped(
        "meta fn a(e) = `((${e}) + 1)`\nmeta fn b(e) = `@a((${e}))`\nlet x = @b(2)\nprintln(to_string(x))\n",
    )
    .expect("expansion");
    // Line 3 (`let x = ...`) carries generated content rooted at line 3.
    match &exp.line_origins[2] {
        LineOrigin::Generated { site_line, .. } => assert_eq!(*site_line, 3),
        other => panic!("line 3 should be generated, got {other:?}"),
    }
    // Line 4 is untouched and still maps to 4.
    assert_eq!(exp.line_origins[3], LineOrigin::Original(4));
}
