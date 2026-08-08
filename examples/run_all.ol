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
// Run it from the examples/ directory:  olang run_all.ol

let olang = unwrap(os.exe_path())
let root = unwrap(os.cwd())

// ── discover targets: each is { label, dir, file } ──
let entries = sort(unwrap(fs.list_dir(".")))
let mut targets = []

// Standalone scripts: top-level *.ol, minus this runner.
for e in entries {
    if str.ends_with(e, ".ol") && (e != "run_all.ol") =>
        { targets = targets + [{ label: e, dir: root, file: e }] }
}

// Packages: a directory with a main.ol, looking one level deeper for nested
// package roots like packages/demo.
for e in entries {
    if (!str.ends_with(e, ".ol")) && unwrap(fs.is_dir(e)) => {
        if unwrap(fs.exists(e + "/main.ol")) =>
            { targets = targets + [{ label: e + "/", dir: root + "/" + e, file: "main.ol" }] }
        else => {
            for sub in sort(unwrap(fs.list_dir(e))) {
                let rel = e + "/" + sub
                if unwrap(fs.is_dir(rel)) && unwrap(fs.exists(rel + "/main.ol")) =>
                    { targets = targets + [{ label: rel + "/", dir: root + "/" + rel, file: "main.ol" }] }
            }
        }
    }
}

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

let started = unwrap(dates.timestamp(dates.now()))
let mut passed = 0
let mut failed = 0

for t in targets {
    unwrap(os.chdir(t.dir))
    let res = os.exec(olang, [t.file])
    unwrap(os.chdir(root))

    match res {
        Ok(r) => if r.code == 0 => {
            passed = passed + 1
            println("  ✓ pass   " + t.label)
        }
        else => {
            failed = failed + 1
            println("  ✗ FAIL   " + t.label + "  (exit " + to_string(r.code) + ")")
            show_tail(r.stdout + r.stderr)
        },
        Err(e) => {
            failed = failed + 1
            println("  ✗ ERROR  " + t.label + "  (could not launch: " + to_string(e) + ")")
        }
    }
}

let elapsed = unwrap(dates.timestamp(dates.now())) - started

// ── summary ──
println("")
println("────────────────────────────────────────")
println("  " + to_string(passed) + " passed, " + to_string(failed) + " failed"
    + "   (" + to_string(len(targets)) + " total, " + to_string(elapsed) + "s)")

if failed > 0 => { os.exit(1) } else => { println("  all green ✓") }
