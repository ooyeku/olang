# Tooling

The developer tools shipped inside the `olang` binary: the test runner,
the formatter, the static checker, the benchmark harness, the reference
generator (`olang doc`), and the bundler (`olang build`). (Package
commands live in `otc` —
see [Packages](packages.md); the language server has
[its own chapter](editors.md).)

Part of [the olang book](README.md) ·
[Language](language.md) · [Standard Library](stdlib.md) ·
[Internals](internals.md)

## `olang test`

Discovers and runs [`test` blocks](language.md#testing) across a directory:

```bash
olang test              # everything under the current directory
olang test examples/    # or a specific directory / single file
```

A *test file* is any `.ol` file containing a top-level `test "name" { ... }`
block. Each test file runs in a fresh interpreter, top to bottom — helper
functions, fixtures, and imports work exactly as they do under
`olang <file>`, and files inside a package get the package's dependencies.
Files without test blocks are not executed at all.

Under the runner, a failing block **records its failure and execution
continues**, so one red test doesn't hide the rest (in a normal
`olang <file>` run, a failing assertion still aborts — the runner is what
changes the policy). Programs see a bare `os.args()`, so a file that
branches on arguments takes its no-argument path.

```text
examples/markdown/main.ol
  ✓ markdown conversion
tests/math_test.ol
  ✓ doubling works
  ✗ deliberately red
      Runtime error: wrong on purpose
────────────────────────────────────────
  2 passed, 1 failed  (2 test files, 35ms)
```

The exit code is non-zero when anything fails — `olang test` slots directly
into CI. An error *outside* any test block (setup code failed) is reported
as a file error and counts as a failure.

Conventions that work well: name dedicated suites `*_test.ol`, or keep a
`test` block next to the code it guards (the markdown example self-checks
its conversion contract this way).

### Coverage

`--coverage` adds a line-coverage report after the run — which lines your
tests actually executed, per file:

```bash
olang test --coverage           # summary per file + overall
olang test --coverage-lines     # also list each file's uncovered lines
```

```text
────────────────────────────────────────
  coverage
    calc.ol         71%  5/7
    calc_test.ol   100%  3/3
    total           80%  8/10
```

Coverage is a **report, not a gate**: it never changes the exit code, so a
green suite with thin coverage still passes. It is attributed to the file
the code *lives in* — a helper defined in one file and exercised by a test
in another is credited to the helper's file, not the test's. A line counts
as executable when it carries a statement (the same thing the runtime
records when it runs), so a fully-exercised file reads exactly 100%.

Under `--coverage` the runner executes on the interpreter tier rather than
the bytecode tier, so a coverage run is slower than a plain `olang test` —
run it when you want the report, not on every save.

## `olang fmt`

A conservative formatter for whitespace hygiene:

```bash
olang fmt                 # format the current directory, in place
olang fmt src/ lib/       # specific paths
olang fmt --check .       # report what would change; non-zero exit (CI)
```

What it does — only ever *outside* multi-line string literals:

- CRLF line endings become LF
- trailing spaces and tabs are stripped
- leading tabs become 4 spaces (the book's indentation unit)
- runs of blank lines collapse to one
- the file ends with exactly one newline

What it deliberately does **not** do: re-indent, re-wrap, or reflow code.
Comments and layout are the author's; template-string and raw-string
interiors are content and are never touched.

**The safety property is absolute**: before writing, the formatted source
is re-parsed and must produce an AST identical to the original's. A file
whose formatting would change its meaning — or that doesn't parse — is
skipped and reported, never modified. Formatting is idempotent: a formatted
tree passes `--check`.

## `olang check`

The static side of gradual typing ([the Types chapter](types.md) tells
the full story) — reports provable type-annotation violations before
the program runs:

```bash
olang check               # every .ol file under the current directory
olang check src/ app/     # specific paths or a single file
```

```text
  × parameter 'a' of add expects Int, got String
   ╭─[src/main.ol:2:1]
 1 │ fn add(a: Int, b: Int) -> Int = a + b
 2 │ let total: Int = add("one", 2)
   · ▲
   · ╰── this would fail at runtime
   ╰────
```

Its discipline is **no false positives**: a diagnostic appears only when
an annotation is provably dishonest — a literal argument against an
annotated parameter, an annotated binding initialized with a known-type
value, a declared return contradicted by what the body provably produces,
a call to a known function with the wrong number of arguments. Anything
it cannot prove stays silent; unannotated, dynamic code is never judged.

Checking is module-aware: a file's `use`d modules are resolved (the
runtime's local conventions: `a/b.ol`, `index.ol`, `mod.ol`, same-dir)
and their `share`d signatures feed the analysis, so a wrong argument to
an imported function is flagged in the importing file.

The checker also goes where the runtime deliberately doesn't: **element
types**. The runtime checks `List<Int>` shallowly ("a List", O(1)), but
the checker decomposes literals element by element — `[1, "a", 3]`
against `List<Int>` reports `element 1 of let binding 'xs' expects Int,
got String`. The same applies to `Map<K, V>` entries, tuple arity and
elements, struct-literal fields, and nested combinations, with the path
spelled out. Deep types flow through annotated bindings and known return
types, so `f(ys)` is flagged when `f` wants `List<Int>` and `ys` is
declared `List<String>`.

The two kinds of finding are labeled distinctly: violations the
runtime's own checks would also catch say *"this would fail at runtime"*
and carry the runtime's exact error text, word for word; element-level
breaks the shallow runtime checks let pass say *"the annotation's
promise is broken here"* — the program may run, but an annotation in the
flagged chain is provably false.

The same checker runs in the language server, so editors surface these
as error squiggles while you type. Exit is non-zero when a violation or
parse error is found, so `olang check` slots directly into CI.

### Project rules

Run project-specific lint rules, written in olang over the [meta
AST](#meta--the-program-as-data-the-open-ast), alongside the built-in
type checks:

```bash
olang check --rules rules.ol .
```

Define a rule as a top-level function whose name begins with `rule_`. The
function takes one argument: the file's AST, flattened to a list of nodes.
Each node is a map with at least a `kind` field and a `line` field. The
function returns a list of findings. A finding is a message string or a
map with `message` and `line` fields.

```olang
// rules.ol
share fn rule_no_bare_unwrap(nodes) =
    nodes
      |> filter((n) => map_get(n, "kind") == "call" && map_get(n, "target") == "unwrap")
      |> map((n) => #{ "message": "bare unwrap(): use match or ?", "line": map_get(n, "line") })
```

`olang check` runs each rule against every checked file except the rules
file itself. It reports each finding with the file name, line number, and
rule name. Findings count as problems, so any finding produces a non-zero
exit.

The command applies the following behaviors:

- A rule that raises an error is reported with its name.
- A rules file that defines no `rule_*` functions is an error.

## `olang bench`

Reproducible timings for `.ol` programs, and a regression guard:

```bash
olang bench kernels/                     # every .ol file in the directory
olang bench fib.ol --runs 10             # more timed runs (default 7)
olang bench kernels/ --save base.json    # store medians as a baseline
olang bench kernels/ --against base.json # compare to a stored baseline
```

Each file runs as its own subprocess — a fresh VM and JIT every run,
measuring the wall-clock a user actually experiences: one discarded
warmup, then the timed runs (7 by default; 3 when a run exceeds two
seconds, where long runs are stable). Every row reports the median,
min, max, and coefficient of variation, and warns when runs disagree
on their output — a benchmark that prints unstable output is measuring
something else.

Comparisons only call a row changed when it moves more than
max(5%, 2×CV) against the baseline: below that threshold it's noise,
not news, and prints as `~`. Add `--fail-on-regress` to turn any red
row into exit code 1 — that flag is what makes a saved baseline a
standing guard for performance work.

## `olang build`

Bundle a program into a **standalone executable** — a single file that
runs on a machine with no olang installed:

```bash
olang build greet.ol              # → ./greet
olang build greet.ol -o mytool    # choose the output name
./greet Ada --loud                # run it; its argv reaches cli.args()
```

The mechanism needs no C compiler or linker: `build` copies the `olang`
runtime and appends the program — as its already-**parsed AST** — with a
small trailing marker. At startup the binary notices that marker,
deserializes the AST, and runs it with the full process argv, so a
bundled tool's own flags and arguments behave exactly as they would
under `olang program.ol`. Embedding the parsed form means a built tool
never re-parses at startup: deserializing is roughly 20× faster than the
parser, which trims cold-start latency for a large program (the original
source rides along too, so runtime error messages still show a code
snippet). The program is parse-checked as it is built, so a broken
program never produces a binary.

Because the runtime is baked in, the whole standard library and the
embedded packages (`cli`, `term`, `ui`, `viz`, `dash`, `colx`,
`mathx`) travel with the executable. A program that `use`s local
`.ol` files is the one limitation — `build` bundles a single source
file, so keep a shippable tool to stdlib and the embedded packages
(or inline its helpers). The trade for zero-dependency distribution is
size: the binary carries the runtime, so it is tens of megabytes (much
smaller from a `--release` olang than a debug one). On macOS the
appended data invalidates any code signature; re-sign the output if you
distribute it.

[`examples/greet.ol`](../examples/greet.ol) is a self-contained tool
built exactly this way — `cli` for its arguments, `term` for color, and
nothing external.

A built binary also embeds its exact source, its `olang.toml` and
`olang.lock`, its capability manifest, and a checksum over all of them.
Use `olang inspect` to read and verify these files. `--source` prints the
source, `--caps` prints the capability grant, and `-o dir/` extracts all
embedded files. Two flags perform checks:

```bash
olang inspect ./tool --verify      # Verify the binary's checksum.
olang inspect ./tool --against .   # Compare the binary to a source tree.
```

`--verify` recomputes the checksum. The checksum covers the source, the
compiled AST (the bytes that execute), the manifest, and the lockfile, so
verification fails if the running program or the capability grant is
changed, not only the source text. `--verify` also checks that the source
parses to the embedded AST, so `--source` reflects what actually runs.
`--against <dir>` compares the embedded source, manifest, and lockfile to a
checkout, reports each file as match/differ/missing, and confirms the
embedded AST is what that source parses to — exiting non-zero on any
mismatch. Use `--against` to confirm a binary was built from a specific
source tree. For the full model, see [Capabilities and the transparent
binary](packages.md#capabilities).

## `olang doc`

Generate an API reference from doc comments:

```bash
olang doc src/                    # → doc.html (themed, browsable)
olang doc lib/ -o api.html        # choose the output file
olang doc lib/ --md > API.md      # Markdown to stdout instead
```

The convention is source-level, and every doc comment is already a
valid olang comment: `//!` at the top of a file documents the module,
and `///` lines directly above a declaration document it. `olang doc`
scans the `.ol` files at or below each path, pairs each `///` block
with the declaration that follows (`fn`, `type`, `error`, `trait`,
`let` — `share` items carry a badge), and renders them. Only documented
declarations appear, so the reference reports exactly what has been
written down and nothing else.

```olang
/// Greet someone by name, returning the greeting.
share fn greet(name) = "Hello, " + name
```

The embedded packages carry these comments, so `olang doc
src/stdlib/embedded/cli.ol src/stdlib/embedded/term.ol` regenerates
their reference from source rather than by hand.

## `olang --watch`

The edit-run loop as a flag:

```bash
olang --watch script.ol       # rerun whenever any .ol file nearby changes
```

Each run is a child process, so a crash or an `os.exit` ends the run,
never the watcher. Changes are detected by polling every `.ol` file at
or below the script's directory (module edits trigger reruns too); a
save landing mid-run queues an immediate rerun. Ctrl+C stops both.

## `olang --record` and `olang replay` — the Open Timeline

Record a run's nondeterministic inputs to a portable trace, then replay
the run bit-for-bit — anywhere, any time:

```bash
olang --record bug.olt program.ol   # run, logging every nondeterministic input
olang replay bug.olt                # re-run: identical, from the trace alone
```

A program can only observe nondeterminism through a small, explicit set
of stdlib calls — `random.*`, the clocks in `time`, the
environment/stdin/`exec` surface of `os`, filesystem reads, `http`, and
the seeded-random parts of `crypto`. `--record` performs each such call
for real and logs its result in order; `replay` intercepts the same
calls and returns the logged results instead. Because olang programs are
deterministic given their inputs (immutable values, capture-by-value
closures, a seeded RNG), reproducing the inputs reproduces the entire
run — the same random rolls, the same timestamps, the same environment,
down to the last digit.

The `.olt` trace **embeds the program source**, so it is self-contained:
replay works from a directory where the program does not exist, on
another machine, months later. A bug report becomes a file. And a
*crashed* run records too — the trace is written on the way down — so the
failure replays exactly, as many times as you need to understand it.

Replay is honest about drift. If the program's sequence of
nondeterministic calls no longer matches the trace — a *different* call,
or the *same* call with *different arguments* — replay stops at the exact
point and says so, rather than silently producing a different run. The
trace records a fingerprint of each call's arguments for that check, and
map iteration order is deterministic (sorted by key), so the two never
disagree by accident. A clean replay is a proof that the recorded inputs
fully determined the run.

Two boundaries worth knowing. Record/replay runs on the interpreter tier
(the one dispatch point that sees every builtin), so a recorded run
forgoes the bytecode tier — a debugging tool, not a hot path. And v1
records a single thread of effects: a program using `spawn`/`par` for
observable concurrency is outside the model. (Roadmap: `replay --why`,
which carries value provenance during replay to answer "where did this
number come from?" — a chain back to the recorded inputs.)

## `meta` — the program as data (the Open AST)

Not a subcommand but a stdlib module, and the reason tools like `check`
and `deps` need not be the only ones: `meta.parse(source)` returns a
parsed olang program as ordinary olang values, so a project can write
its own linters, codemods, and code generators *in olang* rather than
as compiler changes. `otc deps` — list a file's imports — is four lines
over it. See [the `meta` reference](stdlib.md#meta--the-program-as-data-the-open-ast)
and [`examples/metatool`](../examples/metatool/main.ol).

## The examples harness

`examples/run_all.ol` — a test harness written *in olang* — runs every
example program (standalone scripts and packages) in its own subprocess and
reports a pass/fail summary. It complements `olang test`: the harness
checks that whole programs run; the test runner checks `test`-block
assertions. See [examples/README.md](../examples/README.md).
