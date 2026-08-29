# watch — the process-story dogfood

`watch` reruns a command on an interval and streams its output — like
the Unix `watch(1)` — or runs a pipeline, until you press Ctrl-C. It is
to olang's **process** story what [`survey`](../survey/) is to its
terminal story: one small program that exercises the whole capability at
once.

- **`proc.spawn` + streaming** — each run spawns the command and reads its
  stdout *a line at a time* (`proc.read_line`), printing lines with a dim
  counter as they arrive, then reads the exit code with `proc.wait`.
- **`proc.pipeline`** — with `--pipe`, the command is split on `|` and run
  as a real pipeline, each stage's stdout wired to the next one's stdin.
  Every stage's exit code is shown, green for `0` and red for failure.
- **`os.on_interrupt` / `os.interrupted`** — Ctrl-C is trapped, not fatal:
  the loop notices between frames (and mid-interval, checking every 50ms)
  and prints a summary instead of dying half-drawn.
- **`cli` + `term`** — one declarative spec for `--interval`, `--count`,
  and `--pipe`; colored headers, counters, and status.

## Run it

```bash
olang examples/tools/watch/main.ol "date +%T"           # stream a command, every 800ms
olang examples/tools/watch/main.ol -n 500 "seq 3"       # every 500ms
olang examples/tools/watch/main.ol -c 5 "uptime"        # stop after 5 runs
olang examples/tools/watch/main.ol --pipe "ls | wc -l"  # run a pipeline
```

Press Ctrl-C to stop; the run count and last exit code are reported.

The command is split on spaces (no shell, no quoting), so `watch "grep
foo"` is the program `grep` with the argument `foo`. Without a terminal —
piped, or under the [example runner](../run_all.ol) — there is no one to
press Ctrl-C, so a `--count 0` ("forever") watch caps itself at two runs
and exits.

Like the other flagship tools it is one self-contained `main.ol` using
only stdlib and the embedded packages, so it bundles into a standalone
binary:

```bash
olang build examples/tools/watch/main.ol -o watch
./watch --pipe "ps ax | wc -l"
```
