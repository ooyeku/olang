// run_all.ol — a test harness that runs every example in this directory.
//
// It discovers two kinds of program:
//   • standalone scripts — every top-level `*.ol` file (except this one)
//   • packages — any directory (or sub-directory, e.g. packages/demo) that
//     contains a `main.ol`
// and runs each one in a fresh `olang` subprocess via `os.exec`, from the
// program's own directory so relative imports and file reads resolve. Output
// is captured and shown only on failure; the run ends with a pass/fail
// summary and a non-zero exit code if anything failed — so it works in CI.
//
// Run it from anywhere — `olang run_all.ol`, `olang run examples/run_all.ol`
// from the repo root, all resolve the same set of examples.

let olang = unwrap(os.exe_path())

// Resolve everything against this script's own directory, not the current
// working directory, so the discovered set (and the per-target labels the
// harness args key off) is the same no matter where the runner is launched.
// The script path is argv[0]; its directory, made absolute, is the base.
let self_path = unwrap_or(os.args(), [""])[0]
let self_dir = fs.dirname(self_path)
let root =
    if str.starts_with(self_dir, "/") => self_dir
    else if self_dir == "" => unwrap(os.cwd())
    else => unwrap(os.cwd()) + "/" + self_dir

// Programs that block forever by design (servers) can't run under the
// harness; list them here so the skip is visible, never silent.
let long_running = ["webserver/", "app/"]

// ── discover targets: each is { label, dir, file } ──
// Labels stay relative to `root` (e.g. "demo/"), which is what harness_args
// keys on; dirs are absolute so os.exec's cwd resolves from anywhere.
let entries = sort(unwrap(fs.list_dir(root)))
let mut targets = []

// Standalone scripts: top-level *.ol, minus this runner.
for e in entries {
    if str.ends_with(e, ".ol") && (e != "run_all.ol") =>
        { targets = targets + [{ label: e, dir: root, file: e }] }
}

// Packages: a directory with a main.ol, looking one level deeper for nested
// package roots like packages/demo.
for e in entries {
    if (!str.ends_with(e, ".ol")) && unwrap(fs.is_dir(root + "/" + e)) => {
        if unwrap(fs.exists(root + "/" + e + "/main.ol")) =>
            { targets = targets + [{ label: e + "/", dir: root + "/" + e, file: "main.ol" }] }
        else => {
            for sub in sort(unwrap(fs.list_dir(root + "/" + e))) {
                let rel = e + "/" + sub
                if unwrap(fs.is_dir(root + "/" + rel)) && unwrap(fs.exists(root + "/" + rel + "/main.ol")) =>
                    { targets = targets + [{ label: rel + "/", dir: root + "/" + rel, file: "main.ol" }] }
            }
        }
    }
}

// Per-target arguments: a long-running-by-design program gets a bounded,
// deterministic invocation under the harness. Everything else runs bare.
let harness_args = #{
    "demo/": ["--ticks", "48", "--fast", "--quiet", "--seed", "7"]
}
fn args_for(label) =
    if map_has_key(harness_args, label) => map_get(harness_args, label) else => []

let runnable = targets |> filter((t) => !contains(long_running, t.label))
for t in targets {
    if contains(long_running, t.label) =>
        { println("  ~ skip   " + t.label + "  (long-running server; covered by tests/http_serve_test.rs)") }
}
let targets = runnable

// Show the tail of captured output, indented, so a failure is diagnosable.
fn show_tail(text) = {
    let lines = str.lines(str.trim(text))
    let n = len(lines)
    let from = if n > 8 => n - 8 else => 0
    let mut i = from
    while i < n { println("        │ " + lines[i]); i = i + 1 }
}

// ── run every target ──
println("running " + to_string(len(targets)) + " example programs")
println("")

let started = time.monotonic_ms()
let mut passed = 0
let mut failed = 0

for t in targets {
    // Each program runs from its own directory (the exec cwd option), so
    // relative imports and file reads resolve.
    match os.exec(olang, concat([t.file], args_for(t.label)), #{ "cwd": t.dir }) {
        Ok(r) => if r.code == 0 => {
            passed = passed + 1
            println("  ✓ pass   " + t.label)
        }
        else => {
            failed = failed + 1
            println("  ✗ FAIL   " + t.label + "  (exit " + show(r.code) + ")")
            show_tail(r.stdout + r.stderr)
        },
        Err(e) => {
            failed = failed + 1
            println("  ✗ ERROR  " + t.label + "  (could not launch: " + show(e) + ")")
        }
    }
}

let elapsed_ms = time.monotonic_ms() - started

// ── summary ──
println("")
println("────────────────────────────────────────")
println(str.fmt("  {} passed, {} failed   ({} total, {}.{}s)",
    passed, failed, len(targets), elapsed_ms / 1000, (elapsed_ms % 1000) / 100))

if failed > 0 => { os.exit(1) } else => { println("  all green ✓") }
