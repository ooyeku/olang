//! The tier boundary (Campaign 7 opener): a function value the VM
//! declines runs in the bridge interpreter — which now carries its own
//! compiled tier, the run's capability grant, and the live call depth.
//! Before this, a callback invoked inside a promoted function stranded
//! everything it called on a tree-walk (measured at three orders of
//! magnitude), ran *ungated* by the capability table, and started its
//! depth budget from zero.

use olang::ast::Value;
use olang::{Interpreter, Parser};
use std::path::PathBuf;
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

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

#[test]
fn a_thunk_called_inside_a_function_keeps_compiled_speed() {
    // 20M tail frames through a thunk invoked inside a promoted
    // function. On the old tree-walking bridge this is minutes; with
    // the bridge's tier it is well under the generous bound even in a
    // debug build. The wall-clock check is deliberately loose — the
    // point is the complexity class, not the constant.
    let src = "\
fn sum_down(n, acc) = if n <= 0 => acc else => sum_down(n - 1, acc + n)\n\
fn call_it(thunk) = thunk()\n\
call_it(() => sum_down(20000000, 0))";
    let start = std::time::Instant::now();
    let got = eval(src, true).expect("eval");
    assert_eq!(got, Value::Integer(200_000_010_000_000));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(30),
        "thunk-called recursion fell off the compiled tiers: {:?}",
        start.elapsed()
    );
}

#[test]
fn tiers_agree_on_callback_results() {
    let src = "\
fn apply_twice(f, x) = f(f(x))\n\
fn step(v, bump = 1) = v * 2 + bump\n\
apply_twice(step, 10)";
    // `step` has a default parameter, which the hof compiler refuses —
    // the tiered run must still agree with the pure interpreter.
    let interp = eval(src, false).expect("interpreted");
    let tiered = eval(src, true).expect("tiered");
    assert_eq!(interp, tiered);
    assert_eq!(interp, Value::Integer(43));
}

#[test]
fn the_depth_budget_holds_across_the_boundary() {
    // Non-tail recursion reached through a declined callee: the frames
    // spent on the VM side must count against the same 100k budget, so
    // both runs fail with the same cap in the message.
    let src = "\
fn deep(n, pad = 0) = if n <= 0 => 0 else => 1 + deep(n - 1)\n\
fn run_it(f) = f(200000)\n\
run_it(deep)";
    let interp = eval(src, false).expect_err("must exceed");
    let tiered = eval(src, true).expect_err("must exceed");
    assert!(interp.contains("Maximum call depth (100000)"), "{}", interp);
    assert!(tiered.contains("Maximum call depth (100000)"), "{}", tiered);
}

#[test]
fn a_nested_fn_survives_the_round_trip() {
    // The demo's sort_with shape: a nested fn captured by a fold lambda,
    // pushed through the bridge and rebuilt from its VM closure. This
    // broke three separate ways before it worked — get_slot's missing
    // name fallback, the discarded template name (self-recursion), and
    // the registry outranking a disagreeing closure.
    let src = "fn sort_with(xs, better) = {
    fn insert(sorted, x) = {
        if len(sorted) == 0 => [x]
        else => if better(x, head(sorted)) => concat([x], sorted)
                else => concat([head(sorted)], insert(tail(sorted), x))
    }
    xs |> fold([], (acc, x) => insert(acc, x))
}
fn caller(xs) = sort_with(xs, (a, b) => a < b)
to_string(caller([3, 1, 2]))";
    let interp = eval(src, false).expect("interpreted");
    let tiered = eval(src, true).expect("tiered");
    assert_eq!(interp, tiered);
    assert_eq!(
        interp,
        Value::String(std::sync::Arc::new("[1, 2, 3]".into()))
    );
}

#[test]
fn two_private_fns_with_one_name_stay_distinct() {
    // Two modules-worth of `helper` with different bodies: the second
    // must not hijack the first's call sites through the bridge's
    // registry or seeded environment.
    let src = "fn first(xs) = {
    fn helper(v) = v * 10
    xs |> fold(0, (acc, v) => acc + helper(v))
}
fn second(xs) = {
    fn helper(v) = v + 1000
    xs |> fold(0, (acc, v) => acc + helper(v))
}
fn drive(xs) = first(xs) + second(xs)
drive([1, 2, 3])";
    let interp = eval(src, false).expect("interpreted");
    let tiered = eval(src, true).expect("tiered");
    assert_eq!(interp, tiered);
    // 10+20+30 + 1001+1002+1003
    assert_eq!(interp, Value::Integer(3066));
}

#[test]
fn a_declined_function_value_is_still_capability_gated() {
    // The callee's default parameter forces the bridge; --deny fs must
    // reach it there. Before the fix this path seeded no capability
    // table at all, and the read sailed through.
    let dir = std::env::temp_dir().join(format!("olang_tier_boundary_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script: PathBuf = dir.join("bridge_caps.ol");
    std::fs::write(
        &script,
        "fn reader(path, mode = 0) = fs.read_file(path)\n\
         fn run_it(f, p) = f(p)\n\
         println(run_it(reader, \"/etc/hosts\"))\n",
    )
    .unwrap();

    let denied = Command::new(olang())
        .args(["--deny", "fs", "run"])
        .arg(&script)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&denied.stdout),
        String::from_utf8_lossy(&denied.stderr)
    );
    assert!(
        text.contains("capability 'fs' denied"),
        "the bridge must enforce the grant: {}",
        text
    );
    assert!(!denied.status.success());
}
