//! Tier floors: hot shapes must actually reach the compiled tiers.
//!
//! The differential suites pin that every tier computes the same
//! answer; nothing pinned that the fast tiers *engage*. That gap is
//! real: tail-call elimination once left dead code that failed JIT
//! finalize, silently keeping every self-tail-recursive function off
//! native for weeks — answers right, tier wrong, no test red. These
//! tests close the class. Each runs a representative shape under
//! `OLANG_TIER_STATS=1` and asserts a floor on the deterministic
//! counters the run reports: native calls served, functions promoted,
//! VM dispatch instructions not executed because the work ran native.
//!
//! Floors carry generous slack (typically 4–10× below the calibrated
//! value) so they fail on regressions, not on tuning. Warm start is
//! disabled: every run here must earn its tiers from cold.

use std::collections::HashMap;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

struct TierReport {
    aggregate: HashMap<String, u64>,
    per_fn: HashMap<String, u64>,
}

/// Run a script and parse the `tier-stats:` / `tier-fn:` lines from
/// stderr. Panics if the run fails or the stats lines are missing —
/// a report that silently vanished must not read as "floors met".
fn run(source: &str) -> TierReport {
    let dir = std::env::temp_dir().join(format!(
        "olang_tier_floor_{}_{}",
        std::process::id(),
        source.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("shape.ol");
    std::fs::write(&script, source).unwrap();
    let out = std::process::Command::new(olang())
        .arg("run")
        .arg(&script)
        .env("OLANG_TIER_STATS", "1")
        .env("OLANG_WARM", "0")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut aggregate = HashMap::new();
    let mut per_fn = HashMap::new();
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("tier-stats: ") {
            for pair in rest.split_whitespace() {
                if let Some((k, v)) = pair.split_once('=') {
                    aggregate.insert(k.to_string(), v.parse().unwrap());
                }
            }
        } else if let Some(rest) = line.strip_prefix("tier-fn: ") {
            let mut parts = rest.split_whitespace();
            let name = parts.next().unwrap().to_string();
            for pair in parts {
                if let Some(("native_calls", v)) = pair.split_once('=') {
                    per_fn.insert(name.clone(), v.parse().unwrap());
                }
            }
        }
    }
    assert!(
        !aggregate.is_empty(),
        "no tier-stats line — the report format changed or the tier is off:\n{stderr}"
    );
    TierReport { aggregate, per_fn }
}

#[test]
fn tail_recursion_runs_native() {
    // Two tail shapes, both directly called. `sum_down` is the plain
    // single-branch form; `steps` has the nested else-if merge whose
    // eliminated call once stranded dead code past the back-edge — the
    // exact body the TCE bug kept off native (verified: reintroducing
    // the bug zeroes its count). Calibrated: 200 native calls each,
    // 0 VM instructions.
    let r = run(
        "fn sum_down(n, acc) = if n <= 0 => acc else => sum_down(n - 1, acc + n)\n\
         fn steps(n, s) =\n\
             if n <= 1 => s\n\
             else => if n % 2 == 0 => steps(n / 2, s + 1)\n\
             else => steps(3 * n + 1, s + 1)\n\
         let mut total = 0\n\
         for i in range(0, 200) { total = total + sum_down(2000, 0) + steps(i + 2, 0) }\n\
         println(total)\n",
    );
    let plain = r.per_fn.get("sum_down").copied().unwrap_or(0);
    let nested = r.per_fn.get("steps").copied().unwrap_or(0);
    assert!(
        plain >= 50,
        "sum_down must serve native calls (calibrated 200), got {plain}"
    );
    assert!(
        nested >= 50,
        "steps (nested-branch tail recursion) must serve native calls (calibrated 200), got {nested}"
    );
}

#[test]
fn a_lambda_calling_a_named_recursive_function_compiles() {
    // The collatz gap: the kernel is a lambda, its callee a named
    // tail-recursive function the registry has not seen. The hof
    // dependency channel compiles both; the whole map then runs as
    // native calls, one per element. Calibrated: 19,999 native calls,
    // 1 promotion (the dependency), ~40k VM instructions.
    let r = run("fn collatz_steps(n, steps) =\n\
             if n == 1 => steps\n\
             else => if n % 2 == 0 => collatz_steps(n / 2, steps + 1)\n\
             else => collatz_steps(3 * n + 1, steps + 1)\n\
         let steps = map(range(1, 20000), (n) => collatz_steps(n, 0))\n\
         println(fold(steps, 0, (a, b) => math.max(a, b)))\n");
    let native = r.aggregate["native_calls"];
    let promoted = r.aggregate["promoted"];
    assert!(
        native >= 15_000,
        "the kernel must run native per element (calibrated 19,999), got {native}"
    );
    assert!(
        promoted >= 1,
        "collatz_steps must compile through the dependency channel, got promoted={promoted}"
    );
}

#[test]
fn a_map_filter_fold_pipeline_runs_native() {
    // The whole-loop HOF path: each stage's lambda compiles and the
    // loop runs on the VM with native kernel calls — no interpreter
    // per element. Calibrated: 46,667 native calls (20k map + 20k
    // filter + 6,667 fold), 0 VM instructions. Sized under the 50k
    // auto-parallel threshold so the work stays on the measured tier.
    let r = run(
        "let out = range(0, 20000) |> map((v) => v * v) |> filter((v) => v % 3 == 0) |> fold(0, (a, v) => a + v)\n\
         println(out)\n",
    );
    let native = r.aggregate["native_calls"];
    assert!(
        native >= 40_000,
        "pipeline kernels must run native (calibrated 46,667), got {native}"
    );
}

#[test]
fn a_once_called_big_loop_does_not_live_on_the_vm_dispatch() {
    // The OSR / eager-compile shape: called once, a million
    // iterations. Calibrated: the loop runs native — 0 VM dispatch
    // instructions. Interpreted-in-VM it would execute ~5M; the
    // ceiling sits far below that and far above legitimate overhead.
    let r = run("fn once() = {\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < 1000000 { acc = (acc + i * 7) % 1000003 i = i + 1 }\n\
             acc\n\
         }\n\
         println(once())\n");
    let instructions = r.aggregate["instructions"];
    let native = r.aggregate["native_calls"];
    assert!(
        instructions <= 500_000,
        "the loop must not run on VM dispatch (calibrated 0 instructions), got {instructions}"
    );
    assert!(
        native >= 1,
        "the function must enter native code, got native_calls={native}"
    );
}

#[test]
fn bool_arguments_do_not_keep_a_function_off_native() {
    // The call boundary never marshaled Boolean arguments, so any
    // function taking a Bool silently stayed off native — no refusal
    // logged, nothing red. Calibrated: 200 native calls, kinds
    // Int,Bool, 0 VM instructions.
    let r = run("fn step(n, up) = if up => n + 1 else => n - 1\n\
         let mut acc = 0\n\
         for i in range(0, 200) { acc = acc + step(i, i % 2 == 0) }\n\
         println(acc)\n");
    let own = r.per_fn.get("step").copied().unwrap_or(0);
    assert!(
        own >= 50,
        "a Bool-taking function must serve native calls (calibrated 200), got {own}"
    );
}

#[test]
fn a_nested_loop_enters_native_through_osr() {
    // The println keeps the whole function off the JIT, so the nest
    // must enter through on-stack replacement — which once refused any
    // function with more than one loop head. The region is the
    // outermost enclosing loop; the dispatch enters it at the outer
    // head even though the inner back edge is what got hot.
    // Calibrated: 81k VM instructions (the pre-threshold warmup);
    // without OSR the nest executes 81M.
    let r = run("fn grid() = {\n\
             println(\"start\")\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < 3000 {\n\
                 let mut j = 0\n\
                 while j < 3000 { acc = (acc + i * j) % 1000003 j = j + 1 }\n\
                 i = i + 1\n\
             }\n\
             acc\n\
         }\n\
         println(grid())\n");
    let instructions = r.aggregate["instructions"];
    assert!(
        instructions <= 2_000_000,
        "the nest must not spin on VM dispatch (calibrated 81k instructions), got {instructions}"
    );
    assert!(
        r.aggregate["native_calls"] >= 1,
        "the region must enter native"
    );
}

#[test]
fn a_loop_with_wide_live_out_state_enters_native_through_osr() {
    // Six live-ins, five live-outs — past the old 4-slot tuple cap, so
    // the region refused with "live state past the marshal caps".
    // Calibrated: 164k VM instructions; without OSR, 4M.
    let r = run("fn spread() = {\n\
             println(\"start\")\n\
             let mut a = 0\n\
             let mut b = 1\n\
             let mut c = 2\n\
             let mut d = 3\n\
             let mut e = 4\n\
             let mut i = 0\n\
             while i < 200000 {\n\
                 a = (a + i) % 1000003\n\
                 b = (b + a) % 1000003\n\
                 c = (c + b) % 1000003\n\
                 d = (d + c) % 1000003\n\
                 e = (e + d) % 1000003\n\
                 i = i + 1\n\
             }\n\
             a + b + c + d + e\n\
         }\n\
         println(spread())\n");
    let instructions = r.aggregate["instructions"];
    assert!(
        instructions <= 1_000_000,
        "wide loop state must not stay on VM dispatch (calibrated 164k instructions), got {instructions}"
    );
    assert!(
        r.aggregate["native_calls"] >= 1,
        "the region must enter native"
    );
}

#[test]
fn a_mixed_kind_call_site_keeps_both_shapes_native() {
    // Polymorphic specialization: the first shape used to be the only
    // one — a call with new argument kinds fell off native forever.
    // Now it compiles a variant. Calibrated: 200k native calls (100k
    // Int + 100k Float), 0 VM instructions; before variants, the
    // Float half executed 300k VM instructions.
    let r = run("fn scale(x, k) = x * k + 1\n\
         let mut int_sum = 0\n\
         let mut float_sum = 0.0\n\
         for i in range(0, 100000) {\n\
             int_sum = (int_sum + scale(i, 3)) % 1000003\n\
             float_sum = float_sum + scale(i * 1.0, 0.5)\n\
         }\n\
         println(int_sum)\n\
         println(float_sum)\n");
    let own = r.per_fn.get("scale").copied().unwrap_or(0);
    assert!(
        own >= 150_000,
        "both shapes must serve native calls (calibrated 200k), got {own}"
    );
}

#[test]
fn a_template_string_does_not_poison_its_function() {
    // MakeTemplate was never whitelisted, so one template anywhere kept
    // the whole function off native — the report/CSV-builder shape.
    // Calibrated: 300 native calls, 0 VM instructions; before E3 the
    // same program executed 13.5M VM instructions with 0 native calls.
    let r = run("fn report(n, label) = {\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < n { acc = (acc + i * i) % 1000003 i = i + 1 }\n\
             `${label}: sum=${acc} over n=${n} (${acc * 1.0 / 1000.0})`\n\
         }\n\
         let mut rows = []\n\
         for k in range(0, 300) { rows = rows + [report(5000, \"batch\" + to_string(k))] }\n\
         println(rows[299])\n");
    let own = r.per_fn.get("report").copied().unwrap_or(0);
    let instructions = r.aggregate["instructions"];
    assert!(
        own >= 100,
        "a template-bearing function must serve native calls (calibrated 300), got {own}"
    );
    assert!(
        instructions <= 2_000_000,
        "the loop must not fall to VM dispatch (calibrated 0), got {instructions}"
    );
}
