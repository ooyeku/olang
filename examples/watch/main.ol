//! watch — rerun a command on an interval and stream its output, or run a
//! pipeline, until you press Ctrl-C. The dogfood for olang's process story:
//! it drives child processes with `proc`, chains them with `proc.pipeline`,
//! reads exit codes, and shuts down gracefully on SIGINT with `os` — all
//! wrapped in the `cli` + `term` toolkit.
//!
//!   olang examples/watch/main.ol "date +%T"          # stream a command
//!   olang examples/watch/main.ol --interval 500 "ls" # every 500ms
//!   olang examples/watch/main.ol --pipe "ls | wc -l" # run a pipeline
//!
//! Press Ctrl-C at any time: the loop notices, stops, and prints a summary
//! instead of the process being killed mid-frame.

use cli
use term

let spec = #{
    "name": "watch",
    "about": "rerun a command on an interval and stream its output",
    "flags": [
        #{ "name": "interval", "short": "n", "type": "int", "default": 800,
           "help": "milliseconds between runs" },
        #{ "name": "count", "short": "c", "type": "int", "default": 0,
           "help": "stop after N runs (0 = until Ctrl-C)" },
        #{ "name": "pipe", "type": "bool",
           "help": "treat the command as a pipeline: \"a | b | c\"" }
    ],
    "args": [
        #{ "name": "cmd", "default": "date",
           "help": "the command line to run (space-separated)" }
    ]
}

// Split a command line into [program, ...args] on runs of spaces, dropping
// the empties so "grep   foo" is still two words.
fn words(s) = filter(str.split(str.trim(s), " "), (w) => w != "")

// A dim, right-aligned run counter header, e.g. "── run 3 · every 800ms ──".
fn header(run, interval) =
    term.dim("── ") + term.bold("run " + show(run))
        + term.dim(" · every " + show(interval) + "ms · Ctrl-C to stop ──")

// ── one streamed run of a single command ────────────────────────────────

// Spawn `argv`, print each stdout line with a dim line number as it
// arrives, and return the exit code. This is the streaming path: we read
// the child a line at a time rather than waiting for all of its output.
fn stream_once(argv) = {
    match proc.spawn(argv[0], skip(argv, 1)) {
        Err(e) => { println(term.red("watch: ") + e); -1 },
        Ok(p) => {
            proc.close_stdin(p)
            let mut n = 0
            let mut running = true
            while running {
                match proc.read_line(p) {
                    Err(_) => { running = false },
                    Ok(line) => {
                        n = n + 1
                        println(term.dim(str.pad_start(show(n), 3, " ") + "  ") + line)
                    }
                }
            }
            let exit = unwrap_or(proc.wait(p), #{ "code": -1 })
            map_get(exit, "code")
        }
    }
}

// ── one run of a pipeline: "a | b | c" ──────────────────────────────────

fn run_pipeline(cmd) = {
    let stages = map(str.split(cmd, "|"), (part) => words(part))
    match proc.pipeline(stages) {
        Err(e) => { println(term.red("watch: ") + e); -1 },
        Ok(r) => {
            let out = str.trim_end(map_get(r, "stdout"))
            if out != "" => { println(out) }
            let err = str.trim_end(map_get(r, "stderr"))
            if err != "" => { println(term.dim(err)) }
            // Colored per-stage exit codes: green 0, red non-zero.
            let codes = map_get(r, "codes")
            let badges = map(codes, (c) =>
                if c == 0 => term.green(show(c)) else => term.red(show(c)))
            println(term.dim("stages: ") + join(badges, term.dim(" | ")))
            map_get(r, "code")
        }
    }
}

// ── the loop ────────────────────────────────────────────────────────────

fn watch(cmd, is_pipe, interval, count) = {
    unwrap_or(os.on_interrupt(), false)
    // Without a terminal (piped, or under the example runner) there is no
    // one to press Ctrl-C, so a "forever" watch would never end: cap it.
    let tty = os.is_tty()
    let limit = if count > 0 => count else => (if tty => 0 else => 2)

    let mut run = 0
    let mut last = 0
    let mut go = true
    while go {
        run = run + 1
        println(header(run, interval))
        last = if is_pipe => run_pipeline(cmd) else => stream_once(words(cmd))
        println(term.dim("exit " + show(last)))

        let hit_limit = limit > 0 && run >= limit
        if os.interrupted() || hit_limit => { go = false }
        else => {
            // Sleep in short slices so Ctrl-C is noticed promptly, not only
            // at the end of a full interval.
            let mut slept = 0
            while slept < interval && os.interrupted() == false {
                time.sleep(50)
                slept = slept + 50
            }
            if os.interrupted() => { go = false }
        }
    }
    println("")
    if os.interrupted() =>
        println(term.yellow("watch stopped") + term.dim(" after " + show(run)
            + " run(s); last exit " + show(last)))
    else =>
        println(term.green("done") + term.dim(" — " + show(run)
            + " run(s); last exit " + show(last)))
}

// ── dispatch ────────────────────────────────────────────────────────────

let argv = cli.args()
match cli.parse(spec, argv) {
    Err(e) => { println(term.red("watch: ") + e); os.exit(2) },
    Ok(a) => {
        if map_get(a, "help") && len(argv) > 0 => println(cli.help(spec))
        else => watch(map_get(a, "cmd"), map_get(a, "pipe"),
                      map_get(a, "interval"), map_get(a, "count"))
    }
}
