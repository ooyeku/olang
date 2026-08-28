# Command-line tooling

Part of [the olang book](README.md) · [Language reference](language.md) ·
[Standard library reference](stdlib.md) ·
[Architecture and internals](internals.md)

This chapter documents the developer tools built into the `olang` binary: the
test runner, the formatter, the static checker, the documentation generator,
the standalone-executable builder, the benchmark runner, and the record and
replay commands. Everything that touches a *project* — scaffolding (`otc new`, with
`--lib` and `--web` shapes), local dependencies and the library shelf
(`otc add`, `otc lib`), and the project benchmark harness (`otc bench`,
with scaling curves and complexity detection) — is the companion tool
`otc`, described in [Packages and dependencies](packages.md); the
language server has [its own chapter](editors.md).

## Command overview

`olang` is file-first: `olang <file> [args]` runs a program, and bare
`olang` starts the REPL. Everything else is a named command. `olang
--help` lists them all, and `olang <command> --help` documents any one.

| Command | Purpose |
|---|---|
| `olang <file> [args]` | Run a program (`olang run <file>` is the explicit form) |
| `olang` | Start the REPL (`olang repl`) |
| `olang check [path]` | Type-check without running; `--rules FILE` adds project lints |
| `olang fmt [path]` | Format sources in place; `--check` reports instead of writing |
| `olang test [path]` | Discover and run `test` blocks; `--coverage` reports coverage |
| `olang build <file>` | Compile to a self-contained executable; `-o OUT` names it |
| `olang inspect <binary>` | Read a built binary's source, manifest, capabilities, provenance |
| `olang caps [path]` | Show the capability grant a program or binary carries |
| `olang record <file>` | Run a program and record its inputs to a `.olt` trace |
| `olang replay <trace>` | Re-run a recorded `.olt` timeline bit-for-bit |
| `olang doc [path]` | Generate HTML (or `--markdown` Markdown) API reference |
| `olang expand FILE` | Print a file after [macro expansion](macros.md) — the program the runtime actually receives |
| `olang bench` | Run benchmarks |
| `olang profile <file>` | Run under the sampling profiler; report time per function and tier |
| `olang lsp` | Start the language server (LSP over stdio) |

Because the tool is file-first, a word that is neither a known command
nor a flag is taken as a file to run — so `olang report.ol` and `olang
./build` still run those files even though `build` is a command.

**Run options** (before the file: `olang --watch app.ol`) shape how a
program runs — `--watch`, `--deny CAPS`, `--record TRACE.olt`,
`--trace-caps`, `--ovm-tier`, `--max-depth N`, and more. See `olang --help` for the full
set. Every command also honors the `OLANG_DENY` environment variable.

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

A conservative, safety-verified formatter:

```bash
olang fmt                 # format the current directory, in place
olang fmt src/ lib/       # specific paths
olang fmt --check .       # report what would change; non-zero exit (CI)
```

Two layers, applied only *outside* string literals and comments.

**Line hygiene**: CRLF line endings become LF, trailing whitespace is
stripped, leading tabs become 4 spaces, runs of blank lines collapse
to one, and the file ends with exactly one newline.

**Token respacing**: each code line is re-emitted with canonical
spacing —

- one space around binary operators, `=`, `=>`, `->`, and `|>`
- `f(x, y)`, not `f( x ,y )`: commas glue left and breathe right,
  parentheses hug their contents
- calls and indexing glue to their value (`f(x)`, `xs[i]`); a keyword
  keeps its space (`if (x)`)
- unary `-`, `+`, and `!` glue to their operand; ranges glue (`1..5`)
- `:` glues left and breathes right (`x: Int`, `#{ "a": 1 }`)
- braces breathe (`{ ok: true }`, `#{ "a": 1 }`); empty pairs glue
  (`{}`, `#{}`)

The rules were calibrated against every `.ol` file in the repository —
the shipped style is the specification.

What it deliberately does **not** do: re-indent, re-wrap, or reflow
lines — leading whitespace and line structure are the author's. Two
alignments are recognized as intentional and preserved: two or more
spaces before `=>` (match-arm tables) and before a trailing comment.
Template-string, raw-string, and comment interiors are content and are
never touched.

**The safety property is absolute**: before writing, the formatted
source is re-parsed and must produce an AST identical to the
original's (source positions aside). A file whose formatting would
change its meaning — or that doesn't parse — is skipped and reported,
never modified. Macro-using files are compared on the raw
pre-expansion tree, so a file importing its meta fns still formats.
Formatting is idempotent: a formatted tree passes `--check`.

## `olang check`

Everything the compiler can tell you before the program runs. It began
as the static side of gradual typing ([the Types chapter](types.md)
tells that story) and now reports three distinct classes of finding.

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

### Scope and mutability

The second class is not about types at all. The rules settled in
0.61–0.62 — every block scopes its bindings, `let` is required for a
first binding, `let mut` is required to reassign, and a closure cannot
assign to a binding it captured — are validated by one pass that runs
*before* execution. `olang check` reports exactly what `olang run`
would, in the same words and at the same position, because both call
the same validator:

```text
  × cannot assign to 'total': it is captured from an enclosing scope, and
  │ functions capture by value — the outer 'total' would not change. Return
  │ the new value, or hold the state in a cell
   ╭─[src/report.ol:12:20]
   · ╰── the program is refused before it runs
```

These gate. A program with one cannot run at all, on any tier — which is
what makes the three execution tiers agree about them by construction
rather than by testing.

### Advisory warnings

The third class is reported but does not gate: findings that are a
judgement about intent rather than a provable contradiction.

- **A discarded `Result`.** A fallible call in statement position drops
  its failure on the floor — a failed write reads exactly like a
  successful one. Bind it, match it, `unwrap` it to fail loudly, or
  write `let _ = ...` to say the failure is deliberately ignored. A
  block's final statement is its *value*, so a function whose body is
  the fallible call is not flagged.
- **A non-exhaustive `match`** over a known literal-type enum, naming
  the members with no arm.

```text
  ⚠ the Result from fs.write_file is discarded, so a failure here is
  │ invisible. Bind it, match it, unwrap it to fail loudly, or write
  │ `let _ = ...` to say the failure is deliberately ignored
   ╭─[src/backup.ol:8:5]
   · ╰── advisory — the program still runs
```

The discarded-`Result` warning reads which functions return `Result`
from the same registry `:help` does, so the diagnostic and the
documentation cannot disagree about what can fail.

The same checker runs in the language server, so editors surface all
three classes while you type — errors as squiggles, advisories as
hints. Exit is non-zero for a violation or parse error but **not** for a
warning alone, so `olang check` slots directly into CI without advisory
findings failing a build.

### Project rules

Run project-specific lint rules, written in olang over the [meta
AST](stdlib.md#meta--the-program-as-data-the-open-ast), alongside the
built-in type checks:

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

## `olang profile`

Where a program spends its time, per function, **and on which tier**:

```bash
olang profile report.ol                  # sample at 1 kHz, list the top 20
olang profile report.ol --interval 200   # finer sampling (microseconds)
olang profile report.ol --top 40         # a longer table
olang profile report.ol -- --input big.csv   # the program's own arguments
```

The run is an ordinary run — same tiers, same capability grant, same
`os.args()` — with a shadow stack sampled by a background thread. The
report names, for every function that appeared:

```
  TIME BY TIER
    native  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0.4%
    vm      ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    1.3%
    interp  █████████████████████████████░░░   90.7%  ← never promoted
    builtin ██░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    7.6%

  FUNCTIONS
    function                         self   total  samples  tier
    <lambda in bench>  ██████████░  91.2%   91.2%   12,824  interp
    map                █░░░░░░░░░░   4.8%    4.8%      672  builtin
    bench              ░░░░░░░░░░░   0.1%    8.8%       20  vm

  HOTTEST CALL PATHS
      4.8%  bench → <lambda in bench> → map
```

**TIME BY TIER** is the headline a tiered language owes its reader:
`interp` is time on the tree-walking interpreter, and a large share
there is the finding — a hot function that never promoted is usually a
shape the bytecode compiler refused, not a slow algorithm. `builtin`
is time inside the standard library, which is Rust: not a tier, and
not something a program can promote, but time that still has to be
accounted for.

In **FUNCTIONS**, **self** is the share of samples where the function
was the one running; **total** is the share where it was anywhere on
the stack, so `total − self` is time in its callees. A function that
ran on two tiers (interpreted before promotion, say) shows both, as
`vm+interp`.

Anonymous functions are named for where they were written —
`<lambda in bench>` — since a profile full of bare `<lambda>` rows
says only that the program uses lambdas. A builtin that runs user code
(`map`, `fold`, `sort_by`) appears in call paths so a lambda always
has a visible caller, but it is never chosen as the name that
qualifies one: the useful answer is the function a reader would go
looking in.

**NOTES** at the end says what the numbers mean when they mean
something — a large interpreted share and which functions carry it, a
run too short to draw conclusions from, or work that ran on parallel
worker threads, where a frame legitimately has no call path above it
because its caller is on another thread.

Sampling is statistical: a short program yields few samples, and a
function that never appears was simply never running at a tick.
Lengthen the workload or shorten `--interval` before reading much into
small percentages.

Two things the report deliberately folds together. Recursion is one
frame, not one per level — a call path names places in the program,
not stack depth. And a function the JIT has **inlined into its caller**
no longer exists as a frame: its time is attributed to the caller,
which is where the machine code actually is. If a function you expected
is missing entirely and its caller shows `native`, inlining is why.

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
olang doc lib/ --markdown > API.md   # Markdown to stdout instead
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

The same convention reaches the REPL. `:help <name>` answers from the
builtin registry first; when the name is not a builtin, it answers
from the user's own documented code — a `///` block above a
declaration entered in the session, in a file the session `:run`, or
in any module a program `use`d. `:help <module>` on a loaded file's
name shows its `//!` note and every documented item; `:help
module.name` qualifies the lookup to one file. Docs are re-read from
source at lookup time, so an edited file answers with its current
text.

```
olang> /// Steps in the collatz orbit of n.
olang> fn collatz(n, s) = if n == 1 => s else => ...
olang> :help collatz

═══ collatz ═══

  fn collatz(n, s)

  Steps in the collatz orbit of n.

  defined this session
```

### Multi-line input

A line that leaves a delimiter or a string open continues onto the
next line. The continuation prompt shows exactly what is open — `((
...> ` for two unclosed parentheses, `" ...> ` inside a string — and
the input evaluates the moment it balances: closing the delimiter is
the exit. Two commands work inside a continuation: `:end` evaluates
the buffer immediately (an unbalanced buffer surfaces the parse
error, which names the unclosed opener's line and column), and
`:cancel` abandons it without evaluating anything, as does Ctrl+C.
Any other command typed mid-continuation — including `quit` — is
refused with a note naming the open delimiter rather than being
swallowed into the buffer.

`:ml` enters multi-line mode deliberately: lines collect without
evaluating, balanced or not, until `:end` runs the whole buffer as
one program. This is the mode for pasting several statements at once.

## `olang --watch`

The edit-run loop as a flag:

```bash
olang --watch script.ol       # rerun whenever any .ol file nearby changes
```

Each run is a child process, so a crash or an `os.exit` ends the run,
never the watcher. Changes are detected by polling every `.ol` file at
or below the script's directory (module edits trigger reruns too); a
save landing mid-run queues an immediate rerun. Ctrl+C stops both.

## `olang record` and `olang replay` — the Open Timeline

Record a run's nondeterministic inputs to a portable trace, then replay
the run bit-for-bit — anywhere, any time:

```bash
olang record program.ol             # run, logging every nondeterministic input;
                                    # writes program.olt (or -o bug.olt to name it)
olang replay program.olt            # re-run: identical, from the trace alone
```

`olang record <file>` is the command form. When you are already invoking
a file directly, the `--record <trace>` run option does the same thing:
`olang --record bug.olt program.ol`.

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

The `.olt` trace embeds the program source, so it is self-contained: replay
works from a directory where the program does not exist, on another machine,
at a later time. A crashed run is also recorded — the trace is written as the
run fails — so the failure replays exactly and can be re-examined as many
times as needed.

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
observable concurrency is outside the model, and a recorded or replayed
run that starts a task or worker thread says so — a one-time warning on
stderr, in both record and replay mode — rather than letting a trace
that silently missed worker effects present itself as a clean,
fully-determined run. (Roadmap: `replay --why`,
which carries value provenance during replay to answer "where did this
number come from?" — a chain back to the recorded inputs.)

## Writing custom tools with `meta`

Several of the commands above are built on the `meta` module, which returns a
parsed olang program as ordinary olang values. A project can use it to write
its own linters, code transformations, and code generators in olang rather
than as changes to the compiler; `olang check --rules` runs such tools
alongside the built-in checker. The `meta` module is documented in the
[standard library reference](stdlib.md#meta--the-program-as-data-the-open-ast),
and the [Openness](openness.md) chapter describes the program-as-data model.
A complete example is [`examples/metatool`](../examples/metatool/main.ol).

## The examples harness

`examples/run_all.ol`, a test harness written in olang, runs every example
program — standalone scripts and packages — in its own subprocess and reports
a pass/fail summary. It complements `olang test`: the harness checks that
whole programs run, and the test runner checks `test`-block assertions. See
[examples/README.md](../examples/README.md).
