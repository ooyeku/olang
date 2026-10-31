# Command-line tooling

Part of [the olang book](README.md) · [Language reference](language.md) ·
[Standard library reference](stdlib.md) ·
[Architecture and internals](internals.md)

This chapter is the reference for the `olang` command: running
programs, testing them, formatting and checking them, the REPL, the
profiler, building standalone executables, generating documentation, and
recording runs so you can replay them later. It is written to be looked
things up in, so each command has its own section.

A few jobs live elsewhere. Anything to do with a whole project, such as
creating one, adding dependencies or managing your shelf of libraries,
belongs to the companion tool `otc`, covered in [Packages and
dependencies](packages.md). Notebooks have [their own
chapter](notebooks.md), and so does the language server that editors
talk to ([Editor support](editors.md)).

## Table of contents

- [Command overview](#command-overview)
- [`olang test`](#olang-test)
- [`olang fmt`](#olang-fmt)
- [`olang fix maps`](#olang-fix-maps)
- [`olang fix guards`, `defaults`, `first-last`, `units`](#olang-fix-guards-defaults-first-last-units)
- [`olang check`](#olang-check)
- [`olang eval`](#olang-eval)
- [`olang repl --serve`](#olang-repl---serve)
- [`olang bench`](#olang-bench)
- [`olang profile`](#olang-profile)
- [`olang build`](#olang-build)
- [`olang doc`](#olang-doc)
- [Notebooks: `.olmd`](#notebooks-olmd)
- [The REPL](#the-repl)
- [`olang --watch`](#olang---watch)
- [`olang record` and `olang replay` — the Open Timeline](#olang-record-and-olang-replay--the-open-timeline)
- [Tier statistics (`--ovm-stats=json`)](#tier-statistics---ovm-statsjson)
- [`--verify-tiers` — live tier verification](#--verify-tiers--live-tier-verification)
- [Environment variables](#environment-variables)
- [Writing custom tools with `meta`](#writing-custom-tools-with-meta)
- [The examples harness](#the-examples-harness)

## Command overview

`olang` is file-first: `olang <file> [args]` runs a program, and bare
`olang` starts the REPL. Everything else is a named command. `olang
--help` lists them all, and `olang <command> --help` documents any one.

| Command | Purpose |
|---|---|
| `olang <file> [args]` | Run a program (`olang run <file>` is the explicit form) |
| `olang` | Start the REPL (`olang repl`) |
| `olang check [path]` | Type-check without running; `--rules FILE` adds project lints (or `[check] rules` in olang.toml); `--tier` lists the functions the bytecode tier refuses, with the reason |
| `olang doctor [path]` | Audit a project: the manifest, its dependencies, the lock, the browser runtime the binary carries, stale runtime copies |
| `olang eval SOURCE` | Evaluate one expression and print its value, with the working directory's project libraries in scope |
| `olang fmt [path]` | Format sources in place; `--check` reports instead of writing |
| `olang fix maps [path]` | Rewrite `map_get(x, "k")` as `x.k` and `map_get_or(x, "k", d)` as `x.k ?? d`, layout and comments kept; `--check` reports instead of writing |
| `olang fix guards [path]` | Drop a guard that only kept a field read off `()`: `if x == () => () else => x.f` → `x.f`, `x != () && x.f == v` → `x.f == v`; `--dry-run` prints what would change and what is left, and why |
| `olang fix defaults [path]` | Rewrite `if x == () => d else => x` as `x ?? d` |
| `olang fix first-last [path]` | Rewrite a length-guarded `xs[len(xs) - 1]` as `last(xs)` and `xs[0]` as `first(xs)` |
| `olang fix units [path]` | Rewrite `to_int(math.floor(e))` (and `ceil`, `round`, `trunc`) as `math.floor(e)` |
| `olang test [path]` | Discover and run `test` blocks; `--coverage` reports coverage |
| `olang build <file>` | Compile to a self-contained executable; `-o OUT` names it; `--app` carries the program's modules, its dependencies', and their assets |
| `olang inspect <binary>` | Read a built binary's source, manifest, capabilities, provenance |
| `olang caps [path]` | Show the capability grant a program or binary carries |
| `olang record <file>` | Run a program and record its inputs to a `.olt` trace |
| `olang replay <trace>` | Re-run a recorded `.olt` timeline bit-for-bit |
| `olang doc [path]` | Generate HTML (or `--format markdown` Markdown) API reference |
| `olang expand FILE` | Print a file after [macro expansion](macros.md) — the program the runtime actually receives |
| `olang <notebook.olmd>` | Run a [notebook](#notebooks-olmd): markdown whose `olang` blocks are the program; `run`, `test`, `check`, `fmt` and `expand` take one |
| `olang notebook export\|pin\|app <notebook.olmd>` | A notebook's page, its pinned outputs, or a web app made from it ([Notebooks](notebooks.md#commands)) |
| `olang bench [path]` | Run benchmarks |
| `olang profile <file>` | Run under the sampling profiler; report time per function and tier |
| `olang lsp` | Start the language server (LSP over stdio) |
| `olang completions SHELL` | Print a shell completion script (`zsh`, `bash`, `fish`, `elvish`, `powershell`) |
| `olang repl --serve` | Serve the REPL to an editor: line-delimited JSON over stdio (below) |

Because the tool is file-first, a word that is neither a known command
nor a flag is taken as a file to run — so `olang report.ol` and `olang
./build` still run those files even though `build` is a command. Command
names are reserved: a later olang may add a command, so a script with no
`.ol` extension runs as `olang ./name`, which never reads as a command.
A word that is no file and is close to a command name is not run as
one: `olang chek` says `no such file or command: chek — did you mean
\`olang check\`?` and exits 2.

**Run options** (before the file: `olang --watch app.ol`) shape how a
program runs — `--watch`, `--deny CAPS`, `--record TRACE.olt`,
`--trace-caps`, `--ovm-tier`, `--max-depth N`, and more. See `olang --help` for the full
set. `olang run` takes them after `run` as well (`olang run --deny net
app.ol`). Everything after the program file is the program's own
arguments, read with `os.args()` — `olang app.ol --version` hands
`--version` to app.ol. A run option whose value is optional,
`--ovm-stats` and `--ovm-tier`, takes it only after `=`
(`--ovm-stats=json`, `--ovm-tier=10`): written `--ovm-stats json
app.ol`, `json` would be the program file. Every command also honors
the `OLANG_DENY` environment variable.

### Output flags and JSON documents

Every command that writes a file takes `-o`/`--output`, and every
command with more than one form of output takes `--format` (`text` and
`json`; `olang doc` `html` and `markdown`).
Every JSON output carries `format`, its own format number, and `kind`,
what it is — a line-delimited stream on its first event — and every
path in it is absolute. A format number changes only when a field
changes meaning or goes away; fields are added without one, so a
consumer ignores fields it does not know. The documents:

| Command | `kind` |
|---|---|
| `olang check --format json` | `check` |
| `olang check --tier --format json` | `tier-check` |
| `olang test --format json` | events, the first `run` |
| `olang bench --format json` | events, the first `start` with `kind` `bench` |
| `olang bench --save FILE` | `bench-baseline` (a baseline without `format` reads as 1; a newer format is refused) |
| `olang profile --format json` | `profile` |
| `olang inspect --format json` | `inspect`; with `--caps`, `caps` |
| `olang caps --format json` | `caps`: `#{ path, manifest, package, grant, declared, dependencies }`, a grant `#{ fs, net, proc, db, env }` with `fs` `true`, `"read"` or `false` |
| `--ovm-stats=json` | `ovm-stats` |
| `otc list --format json` | `deps` |
| `otc lib list --format json` | `shelf` |
| `otc toolchain list --format json` | `toolchains` |
| `otc bench --save FILE` | `otc-bench-baseline` |

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success: the program finished, the check is clean (warnings allowed), every test passed |
| `1` | Findings or failures: a runtime error the program did not handle, `olang check` problems, a failed test, a failed build, `olang fmt --check` or `olang fix --check` with something to change, a bench row past its baseline, an `olang doctor` finding, `olang test --fail-if-none` with no test block run |
| `2` | Misuse: an unknown flag or command, a bad flag value, a path that does not exist, a `--deny` or `OLANG_DENY` olang does not know, an output file that cannot be written |
| `3` | The tier guard: a run keeping tier statistics with `OLANG_TIER_GUARD=1` in which a pinned-native function fell back |
| `102` | `--verify-tiers` found a native call whose result differs from the bytecode VM's |
| any | A program's own `os.exit(n)` |

## `olang test`

`olang test` finds the [`test` blocks](language.md#testing) in a folder and runs them:

```bash
olang test              # everything under the current directory
olang test examples/    # or a specific directory / single file
olang test tests/ --only "rate limit"   # the blocks whose name contains the text
olang test tests/ --times               # each block's milliseconds, and the slowest five
olang test --deps       # the dependencies' blocks too
olang test lib/calc.ol tests/calc_test.ol   # several files (or directories) in one run, each once
olang test --format json tests/            # the run as line-delimited events, for an editor
```

A *test file* is any `.ol` file containing a top-level `test "name" { ... }`
block. Each test file runs in a fresh interpreter, top to bottom — helper
functions, fixtures, and imports work exactly as they do under
`olang <file>`, and files inside a package get the package's dependencies.
Each runs with the working directory set to its project's root — the
folder with the `olang.toml` above the file — and set back after, so a
relative path in a test means the same whatever folder `olang test` was
started in: `fs.read_file("tests/fixtures/a.txt")` reads the project's
`tests/fixtures/a.txt`, as the project's own commands do. A file in no
project runs from its own folder, and so does a notebook, which runs in
its own folder wherever it is run ([Notebooks](notebooks.md)). A test
that wants what lies beside it, wherever it is, finds its own folder as
`fs.dirname(testing.snapshot_dir())`. Files without test blocks are not
executed at all.

The walk keeps what a checkout keeps: every `.gitignore` (at any level,
and those above the path) and `.git/info/exclude` are respected, a
repository or not, and hidden folders, `target/`, `node_modules/` and
`dist/` are left out even with no `.gitignore`. Inside a project, a
folder below the walk's start with an `olang.toml` of its own is
another project — a test fixture, a sample, a vendored package — and is
left out too, so a fixture's deliberately failing tests never run (or
rewrite its snapshots) under the project's bare `olang test`. A path
named on the command line is walked whatever ignores it, and a fixture
named runs as its own project: `olang test tests/fixtures/sample`.
Outside any project the walk enters every project below it. `olang
check` and `olang fmt` walk the same way.

`olang test` answers for the project's own files. An imported module's
blocks run when the module is one of them — under the package's root
(anywhere on disk outside a package) and not a dependency — so
`olang test tests/` still runs the blocks of the `lib/` modules the
tests import. A dependency's blocks — a shelf or lock package, a path
dependency (even one vendored below the root, which the walk skips),
the runtime's embedded modules — are its authors' suite and do not run;
`--deps` runs them too, as they are imported. Either way an imported
module's blocks run once per invocation, keyed by the module's file —
a project whose every client module imports the SDK runs the SDK's
suite once under `--deps`, not once per importer. A named *file* runs
its own blocks only: `olang test tests/x.ol` is a question about that
file, and the modules it imports are not part of the answer.

`--only <text>` runs the blocks whose name contains the text, across
every file under the path, and names only the files that had one — the
way to check one test of a suite that takes minutes. `--times` prints
each block's milliseconds beside its name and lists the slowest five at
the end. Both pass through `--watch`, as `--deps` does.

Under the runner, a failing block **records its failure and execution
continues**, so one red test doesn't hide the rest (in a normal
`olang <file>` run, a failing assertion still aborts — the runner is what
changes the policy). Programs see a bare `os.args()`, so a file that
branches on arguments takes its no-argument path, and `OLANG_TEST` is
set in the environment, so a library can choose test-time behavior
(the web SDK's `dispatch` runs handlers on a task thread under it).

In a directory run, a module that is itself one of the files under the
path runs its blocks as that file and not again when another file
imports it; a module outside the path — a package — still runs its blocks once, at
its first import.

`olang test --watch` re-runs the tests whenever a `.ol` file under the
target changes — the edit-test loop, each run a child process so a
crash ends the run and never the watcher.

**Snapshot tests.** `testing.snapshot(name, value)` compares the value
with `__snapshots__/<name>.snap` beside the file under test — a String
as its text, any other value as `show` writes it: the first run writes
the file and passes, a later run that differs fails with both texts,
and `OLANG_UPDATE_SNAPSHOTS=1` (or the generic `UPDATE_SNAPSHOTS=1`)
accepts the new form. A library with an update variable of its own
passes it: `testing.snapshot(name, value, #{ "update": true })`. It is
for the long HTML and SVG strings a test would otherwise inline; commit
the directory.

```text
examples/language/markdown/main.ol
  ✓ markdown conversion
tests/math_test.ol
  ✓ doubling works
  ✗ deliberately red
      Runtime error: wrong on purpose
────────────────────────────────────────
  2 passed, 1 failed  (2 test files, 35ms)
```

The exit code is non-zero when anything fails — `olang test` slots directly
into CI. A run that finds no test block passes; `--fail-if-none` makes
it exit 1 instead (no file has one, or `--only` matched none), so a CI
step pointed at the wrong directory cannot pass over nothing. An error *outside* any test block (setup code failed) is reported
as a file error and counts as a failure.

Conventions that work well: name dedicated suites `*_test.ol`, or keep a
`test` block next to the code it guards (the markdown example self-checks
its conversion contract this way).

### `olang test --format json`

For an editor: the run as it goes, one JSON
object a line on stdout, each with an `event`. The human report is not
printed; everything else about the run — which files and blocks run,
`--only`, several paths, `--verify-tiers`, `--ovm-stats`, the exit
status — is as without the flag. Lines and columns are 1-based; files
are absolute paths.

```json
{"event":"run","format":1,"files":["/p/tests/calc_test.ol"],"count":7,"only":null,"verify_tiers":null}
{"event":"test","file":"/p/tests/calc_test.ol","name":"total is off by one","line":13}
{"event":"failed","file":"/p/tests/calc_test.ol","name":"total is off by one","line":13,"ms":0.21,
 "kind":"assert_eq","message":"Runtime error: Assertion failed: Integer(42) != Integer(41)",
 "at":{"file":"/p/tests/calc_test.ol","line":14,"col":5},"stmt":14,"frames":[],
 "expected":{"type":"Int","show":"41","pretty":"41","value":41},
 "actual":{"type":"Int","show":"42","pretty":"42","value":42}}
{"event":"passed","file":"/p/tests/calc_test.ol","name":"cents of a price","line":29,"ms":0.05}
{"event":"snapshot","file":"/p/tests/calc_test.ol","name":"the card's pixels","line":41,
 "snapshot":{"name":"card","baseline":"/p/tests/__snapshots__/card.png","actual":"/p/tests/__snapshots__/card.actual.png",
             "diff":"/p/tests/__snapshots__/card.diff.png","differing":50,"total":19800,"worst":231,"sizes":null}}
{"event":"file","file":"/p/tests/calc_test.ol","passed":3,"failed":4,"error":null}
{"event":"finished","passed":3,"failed":4,"skipped":0,"files":1,"ms":412,"code":1}
```

| `event` | When, and what it says |
|---|---|
| `run` | First: the files that will run (those with blocks), how many blocks they declare (`only` applied), the `--only` text, the `--verify-tiers` rate. |
| `test` | A block starts: `file`, `name`, `line`. |
| `passed` | It passed: `ms`. |
| `failed` | It failed: `ms`, `message`, `kind`, `at` (the failing assertion or error: `file`, `line`, `col`; `null` for an assertion that tallies instead of raising, `testing.assert_*`), `stmt` (the block's statement it surfaced from — its place in the test file when `at` is in a library), `frames` (innermost first, each `name`, `file`, and the `line` of its `fn` when the file declares one). `kind` is `assert_eq`, `assert_ne`, `snapshot` (a text snapshot), `pixels` (a pixel snapshot: `snapshots` lists them) or `error`; for the first three, `expected` and `actual` are the values: `type`, `show` (the display form), `pretty` (the display form an entry a line, when a list, map, tuple or record is longer than 60 characters — what a line diff reads), `value` (as JSON, where the value has a form there), `string` (a string's own text), `clipped` (past 20,000 characters). |
| `snapshot` | A pixel snapshot differs, said by the library that compares it (`testing.snapshot_failed`, which Loom's `snapshot` calls): `snapshot` holds `name`, `baseline`, `actual`, `diff` (paths, a relative one said beside the test file), `differing`, `total`, `worst`, `sizes`. |
| `skipped` | A block of a named file that `--only` left out. |
| `divergence` | `--verify-tiers` found native code and the VM disagreeing: `function` (`fn name (native call)`, or an on-stack-replacement region), `native`, `vm` (the two results, rendered), and the block under way (`file`, `name`, `line`). A `finished` event with `aborted: "divergence"` and `code` 102 follows, and the process ends. |
| `file` | A file is done: its `passed` and `failed`, and `error` — the file's parse error (`kind` `parse`) or an error outside any block (`kind` `error`, with its `line`, `col` and `frames`). |
| `finished` | Last: the totals, `ms`, and the exit status `code`. |
| `args` | With `--record-args` only, after a file's blocks ran (before its `file` event): the last call its test blocks made to each of the project's top-level functions — `file` (where the function is declared), `fn`, `params`, `test` (the block that made the call), and `args`, each argument as olang source that makes it again (`"200"`, `"\"text\""`, `"[1, 2]"`, `"#{ \"k\": 1 }"`) or `null` for one with no literal (a function, a handle, a record, a Result, a value past 4,000 characters). A dependency's functions and the runtime's are left out. |

**`--record-args`** (with `--format json`) asks for the `args` events: an
editor binds a function's parameters from them when a line inside the
function is evaluated. Every call that enters through
the interpreter is seen — a block's own calls, and the calls of code the
tiers did not compile; the last one of each function is kept. Without
the flag nothing is recorded: one branch a call.

What the program under test prints still goes to stdout, between the
events, as it is printed: a reader takes a line holding `{"event":` as an
event — with any text before it on the line as printed output — and any
other line as the current block's output.

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

`olang fmt` tidies the layout of your source files. It is cautious by
design: it checks that every file it rewrites still means exactly the
same program, and leaves the file alone if not.

```bash
olang fmt                 # format the current directory, in place
olang fmt src/ lib/       # specific paths
olang fmt --check .       # report what would change; non-zero exit (CI)
```

It works in two layers, and never touches the inside of strings or
comments.

**Line hygiene**: CRLF line endings become LF, trailing whitespace is
stripped, leading tabs become 4 spaces, runs of blank lines collapse
to one, and the file ends with exactly one newline.

**Token respacing**: each code line is re-emitted with canonical
spacing —

- one space around binary operators, `=`, `=>`, `->`, and `|>` — the
  matrix product too (`V @ V.t()`); an `@` that begins a macro call or
  a decorator (a line's start, after `(`, `,`, `=` or a keyword) glues
  to its name (`@json`, `@retry(3, f)`)
- `f(x, y)`, not `f( x ,y )`: commas glue left and breathe right,
  parentheses hug their contents
- calls and indexing glue to their value (`f(x)`, `xs[i]`); a keyword
  keeps its space (`if (x)`)
- unary `-`, `+`, and `!` glue to their operand; ranges glue (`1..5`)
- `:` glues left and breathes right (`x: Int`, `#{ "a": 1 }`)
- braces breathe (`{ ok: true }`, `#{ "a": 1 }`); empty pairs glue
  (`{}`, `#{}`)

What it does **not** do: re-indent, re-wrap, or reflow
lines — leading whitespace and line structure are the author's, so a
method chain written a step to a line (`    .filter(…)`) keeps its
lines and their indent, each step respaced as its own line. Two
alignments are recognized as intentional and preserved: two or more
spaces before `=>` (match-arm tables) and before a trailing comment.
Template-string, raw-string, and comment interiors are content and are
never touched.

**The safety property**: before writing, the formatted
source is re-parsed and must produce an AST identical to the
original's (source positions aside). A file whose formatting would
change its meaning — or that doesn't parse — is skipped and reported,
never modified. Macro-using files are compared on the raw
pre-expansion tree, so a file importing its meta fns still formats.
Formatting is idempotent: a formatted tree passes `--check`.

## `olang fix maps`

`olang fix` rewrites a program into a shorter idiom, exactly: the new
program answers what the old one did for every input. Each rewrite is
its own subcommand.

`olang fix maps` rewrites map reads into [`.` and `??`](language.md#maps): every
`map_get(chain, "ident")` becomes `chain.ident`, and every
`map_get_or(x, "k", d)` becomes `x.k ?? d`.

```bash
olang fix maps                  # the current directory, in place
olang fix maps --check src/     # report what would change; non-zero exit
olang fix maps . --skip lib/vendored.ol   # leave a file (or folder) alone
```

Both rewrites are exact: `m.key` *is* `map_get(m, "key")` on a map, an
object and a parsed JSON object, and `x.k ?? d` takes the default
exactly where `map_get_or` does — for an absent key and for a stored
`()`. The one difference is *when* the default is evaluated: `??`
evaluates it only when it is used. So a `map_get_or` whose default may
have an effect — a call to one of the program's own functions, or to a
builtin outside the pure set — is **flagged, never rewritten**
(`flagged file:line: …`), for a person to decide.

The rewrite is made in the source text: parentheses are added only
where precedence needs them (`(m.pad ?? 0) + 1`, `(a + b).k`), and
everything else — layout, comments, blank lines — stays byte for byte.
Left alone, and counted in the summary: a key that is not an identifier
(`"content-type"`) or is a reserved word, a computed key, a call whose
value is called (`map_get(m, "f")(x)` would read as a method call), the
right side of a pipeline (which passes the first argument), a call with
a comment inside it, a macro's arguments (the macro is handed their
text), and the calls of a `map_get` or a `map_get_or` the file declares or
imports itself.

The safety property is `olang fmt`'s: the rewritten file must parse to
the original's tree once both are read back the same way (each `a.k`
as `map_get(a, "k")`, each `a.k ?? d` as `map_get_or(a, "k", d)`), or
the file is left untouched and reported. A second run changes nothing.

## `olang fix guards`, `defaults`, `first-last`, `units`

These four rewrites work the same way as `olang fix maps`. The places
to change come from the grammar's parse tree, the text is changed only
there (layout and comments stay byte for byte), parentheses are
added only where precedence needs them, and a file whose rewrite would
read back as a different program is left untouched and reported. Each
runs to its fixed point, so a second run changes nothing.

```bash
olang fix guards --dry-run src/   # what would change, and each form left with why
olang fix guards src/             # in place
olang fix defaults --check .      # non-zero exit when something would change
olang fix first-last . --skip vendor/
```

Every rewrite is **exact**: the new program answers what the old one
answered for every input — `()`, an empty list, a value of the wrong
kind (a raise stays a raise) — on every tier. A form that cannot be
proven so is left and counted, with its reason, in the summary (and
listed one by one under `--dry-run`).

**`guards`** — a guard that only kept a field read off `()`. A field of
`()` is `()`, so:

| Before | After | Why |
|---|---|---|
| `if x == () => () else => x.f.g` | `x.f.g` | when `x` is `()` both answer `()` |
| `if x == () \|\| x.f == () => () else => x.f.g` | `x.f.g` | each tested path is a prefix of the read |
| `if x != () => x.f` (or `else => ()`) | `x.f` | the same, turned round |
| `x != () && x.f == v` | `x.f == v` | `v` a literal that is not `()`: `() == v` is false, as the guard was |
| `x != () && x.f != ()` | `x.f != ()` | `() != ()` is false |
| `x == () \|\| x.f != v` | `x.f != v` | `() != v` is true, as the guard was |
| `x == () \|\| x.f == ()` | `x.f == ()` | |

Left: a read used as a Bool (`x != () && x.f` — `&&` needs a Bool, and
`()` is not one), a comparison with a value that may itself be `()`
(`x.f == y`), a comparison `()` answers differently (`x.f > 3`, `x != ()
&& x.f == ()`), a call or an index on the guarded path (`x.f(1)`,
`x.xs[0]`), and a comment the rewrite would drop.

**`defaults`** — `if x == () => d else => x` and `if x != () => x else
=> d` become `x ?? d`. `??` evaluates `d` only when `x` is `()`, as the
`if` did, so `d` may be anything — a call, a `raise` — and `x`, a name
or a field chain, is read once where it was read twice, which changes
nothing. An `else if` default is kept whole: `if a != () => a else if b
!= () => b else => 0` is `a ?? (b ?? 0)`. Left: a default that is a
block, or a `return`, `break`, `continue`, loop or assignment; a default
of `()` (`olang fix guards` drops that test); a comment the rewrite
would drop.

**`first-last`** — `first(xs)` and `last(xs)` answer `()` for an empty
list, where `xs[0]` and `xs[len(xs) - 1]` raise, so only a read whose
emptiness the code tests is rewritten:

| Before | After |
|---|---|
| `if len(xs) == 0 => () else => xs[len(xs) - 1]` | `last(xs)` |
| `if len(xs) > 0 => xs[0] else => ()` (or no else) | `first(xs)` |
| `len(xs) > 0 && xs[len(xs) - 1] == v` | `len(xs) > 0 && last(xs) == v` |
| `if len(xs) == 0 \|\| c => d else => xs[0]` | `… else => first(xs)` |

A read is guarded in the branch a length test proves non-empty (`len(xs)
> 0`, `!= 0`, `>= 1`, either way round, a conjunct of the `if`'s
condition or of the `&&` before it; `len(xs) == 0` in the `else`, or
before an `||`), not inside a lambda written there (it may run later),
and not where `xs` is bound again. `len` and `[i]` also take Bytes,
which `first` and `last` refuse, so `xs` must be a name bound to a value
that is never Bytes: a list or string literal, a `+` with one, the
result of `split`, `map`, `filter`, `sort`, `str.split` and the other
list and string makers, or a parameter annotated `[T]` or `String` —
and never assigned anything else. Left: an unguarded `xs[len(xs) - 1]`
(it raises on `[]`), a list not proven never Bytes, and a file or
function that binds `len`, `first` or `last` itself.

**`units`** — `to_int(math.floor(e))` becomes `math.floor(e)`, and the
same for `ceil`, `round` and `trunc`: they answer an Int (or raise
before `to_int` is reached), and `to_int` of an Int is that Int. Left:
a file or function that binds `to_int` or `math` itself.

`OLANG_FIX_DEBUG=<dir>` writes the two trees as read back when a file is
left, as for `olang fix maps`.

## `olang check`

`olang check` tells you everything the compiler can work out before the
program runs. Its findings come in three kinds: type errors that an
annotation provably contradicts (the static side of [gradual
typing](types.md)), scope and mutability errors that would stop the
program from running, and advisory warnings about code that works but
probably isn't what you meant.

```bash
olang check               # every .ol file under the current directory
olang check src/ app/     # specific paths or a single file
olang check --fix         # apply the unambiguous rewrites first (template escapes)
olang check --format json # every problem as data, as the language server publishes it
```

**As data.** `--format json` prints one document: `#{ format: 1, kind:
"check", olang, files, errors, warnings }`, each file `#{ file,
diagnostics }` (`error` too when it could not be read), each diagnostic
exactly what `olang lsp` publishes for that file — the same checker, so
an editor's Problems and a pipeline's agree — as `#{ severity, code,
message, tags, line, column, end_line, end_column, range }`:
`severity` `error`, `warning`, `information` or `hint`; `code` the rule
(`syntax`, `macro`, `scope`, `runtime`, `type`, `advisory`, an
advisory's class — `shadow`, `exhaustiveness`, `shape`, `keys`,
`result`, `copy`, `no_effect` — `unused` for a name nothing reads, `allow` for an
[allow comment](#allowing-one-line) that names no class (a warning), `manifest` for olang.toml:
a `[package] olang` that is not a version is an error, since it names the oldest olang the
project runs on, see [packages.md](packages.md));
`tags` `["unnecessary"]` on an unused name (a hint, faded by an editor,
never a problem); `line`/`column` to `end_line`/`end_column` 1-based in
characters, and `range` the server's own (0-based, UTF-16). The exit
code is the text form's: 1 when any is an error. `--rules` and `--fix`
are text-only; `--tier --format json` is the tier report below.

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

Unannotated code is judged where a failure is provable all the same.
The builtins that take a collection refuse a number or a boolean
(`fold(5, …)`), and the `map_*` family refuses anything that is not a
map or a record; and a function shows what it needs of a parameter on
the path every call takes — handed straight to such a builtin, added to
a string literal, or called with so many arguments — so the call that
cannot work is reported where it is written:

```text
  × argument 1 of `total` is an Int, and `fold` needs a list (`total` passes it to `fold`)
  × argument 1 of `apply_to` is a function of 1 parameter, and `apply_to` calls it with 2
```

Only the unconditional path counts: a use inside a branch, a loop, a
lambda, or the right of `&&`/`||` may stand behind a guard and says
nothing, as does a local that takes a builtin's name.

A name a file uses and nothing in it defines or imports is an error:
natively it is an undefined variable the moment that line runs, and in
a browser bundle too, which keeps each module's scope.
A bare `use m` brings every name of `m`, an imported type brings its
variants, and a module is its own namespace; when a bare `use` cannot be
resolved, what it brings is unknown and nothing is reported.

A call of a global builtin or a stdlib module's function with an
argument count its implementation refuses is an error with the
runtime's words: `sort([1], 2)` is `Arity mismatch: expected 1, got 2`,
and `str.thousands(1234)` is `str.thousands: missing argument 2`. A
module function is judged by the fewest arguments it takes (several take
options after their required ones); a call in a pipeline's right side,
whose first argument the pipeline supplies, is not judged, nor a name
the file binds itself.

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

The second class is not about types at all. The scope rules — every
block scopes its bindings, `let` is required for a first binding, `let
mut` is required to reassign, and a closure cannot assign to a binding
it captured — are validated by one pass that runs *before* execution. `olang check` reports exactly what `olang run`
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
  the members with no arm; and over a value of a declared `enum` type
  (a parameter or `let` annotated with the type, or a constructor call
  or bare unit variant matched directly), naming the constructors with
  no arm: `match over \`Shape\` is not exhaustive: Rect has no arm`. A
  wildcard or a plain binding covers everything; a guarded arm proves
  nothing; a constructor pattern covers its constructor only when its
  sub-patterns are irrefutable.
- **A literal key a declared shape does not carry.** A parameter, `let`,
  or return annotated `{ summary: String, count: Int }` — or with an
  alias of it, `type Summary = { summary: String, count: Int }` —
  declares the keys a map or record carries, and `r.summry`,
  `r["summry"]`, and `map_get(r, "summry")` are then reported with the
  nearest key;
  the runtime answers such a read with Unit. The shape reaches the
  checker from wherever it was written — a macro's output included —
  and the runtime does not enforce it (a map's keys are data).
- **A field a declared struct does not have.** `p.y` where `p` is
  annotated `P = struct { x: Int }` is reported with the struct's
  fields; unlike a map's, the read raises.
- **A key no construction has.** `m.key` on a map without the key is
  `()`, so a misspelt key fails nowhere. The checker follows each map
  and object literal whose keys are written (`#{ "running": true }`,
  `{ id: 1 }`) through `let`s and assignments, `map_set`, `map_set_in`,
  `map_update`, `map_merge`, spreads, `if`/`match` and `??`, into the
  parameters of the module's functions (the join of every call's
  argument) and out of their returns, and reports a `.key` read that
  none of them has, with the keys they do have and the nearest
  (`` `runing` is not a key any construction of `p` has (it has
  `live_id`, `running`) … did you mean `running`? ``). A value it cannot
  follow — a shared function's parameter, a function used as a value, a
  computed key, anything from another module or the outside — is never
  reported, and a read compared with `()` or defaulted with `??` is a
  presence test, not reported either. The language server completes the
  same keys after `name.`.
- **A parameter that shadows a function its body calls.** `fn row(s,
  span) = span(s)` with `span` imported: the call reaches the argument,
  and the runtime can only say so at the call, in the browser, frames
  away from the parameter. Reported at the function.
- **A binding that takes a stdlib module's name.** `let fs = …` turns
  every later `fs.exists(…)` in its scope into a field access on a
  value. The same is reported for a parameter, a lambda parameter, a
  loop variable, and a pattern binding whose name is then read as the
  module (`fn h(fs) = fs.exists(p)`); `use lib.csv` binds `csv` over the
  stdlib module (the path import wins, on purpose) and says so.
- **A value computed and dropped.** An expression statement that is a
  literal, a name, or arithmetic, a comparison or a negation over them
  does nothing but produce a value, so outside a block's last line
  (its value) and the program's last statement it is reported:
  `let a = 1.0 + 2.0` over `-10.0` binds `3.0`, and `-10.0` is a
  statement of its own (a continuation is written `- 10.0`, see [source
  structure](language.md#source-structure)); `x == 3` alone on
  a line was meant as an assertion. `()` is exempt, and so is a
  notebook cell's last statement, the cell's output.
- **An `==` between values of provably different types** — `"yes" ==
  true` — is always false (values of different types never compare
  equal), so the comparison is almost certainly not the one intended.
  Int and Float compare numerically and are not flagged.

Every statement carries its own position, at any depth — inside a
function body, a block, a test block — so a warning or a failure names
its own line, never the enclosing statement's. Expressions are placed by
the statement that contains them.

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

### Allowing one line

An advisory that is deliberate is silenced where it is, by an `// allow:`
comment naming its class — at the end of the line the finding is on, or
on a line of its own directly above it (other comment lines between are
allowed; a blank line or code ends it):

```olang
"logo.png" // allow: no_effect — evaluated by the lens tests
// allow: shadow, no_effect
// the shell's own name, on purpose
let cell = 2
```

The names are the classes `[check] promote` takes (`exhaustiveness`,
`shape`, `keys`, `result`, `shadow`, `copy`, `no_effect`, `tier`), plus
`unused` for the hint of a name nothing reads and `all` for every one of
them; a list is separated by commas, and the text after its last name is
free. An allow applies to its one line — the line a finding is reported
on (a statement's first line, a function's `fn` line) — and only to
advisories: a syntax, scope, type or run-time error is never silenced,
whatever the comment says. Its rules:

- **An allow wins over `[check] promote`.** It is explicit and local,
  so a promoted class's finding on an allowed line is not reported at
  all; the same finding on any other line is still an error.
- **A name that is no class is a warning** (code `allow`, at the name,
  with the nearest class), as is an `// allow:` with no name; that
  warning is never silenced or promoted. An allow that silences nothing
  is not reported.
- **Everywhere findings are made**: `olang check` and its `--format
  json`, the language server (`unused` silences the hint), a notebook's
  cells (`.olmd`: the comment in the cell, on the file's own lines),
  `--tier` (`// allow: tier` on the `fn` line, or above it, leaves a
  refusal unreported: its JSON row says `"allowed": true` and `"error":
  false` — but a function pinned native stays an error, since the pin
  says it must compile), and `--rules`, where a project rule is allowed
  by its own name (`// allow: rule_no_bare_unwrap`; `all` is the
  checker's classes, not a rule's).
- `olang fmt` keeps the comment where it is written, trailing or above.

The same checker runs in the language server, so editors surface all
three classes while you type — errors as squiggles, advisories as
hints. Exit is non-zero for a violation or parse error but **not** for a
warning alone, so `olang check` slots directly into CI without advisory
findings failing a build.

### What the tier refuses

`olang check --tier [path]` compiles every function of each file under
the bytecode tier's rules, without running the program, and lists what
the tier refuses with the compiler's reason:

```text
lib/snapshot.ol
  ⚠ `start_snapshot` stays on the tree-walker: Compilation failed: it contains a `spawn` (a task starts on the interpreter)
lib/weft.ol
  ⚠ `client_problems` stays on the tree-walker: calls 'check_record', which nothing in its scope defines (a missing import? ...)
```

A refused function runs on the tree-walker: correct, and several times
slower. Declarations are evaluated (imports, types, functions, `let`s
of literal values); nothing else a program does at start happens. The
second line above is a finding in its own right — a name a module never
imported, which a concatenated browser bundle resolves and a native run
does not. Refusals are advisories unless `[check] promote` names `tier`.

`--format json` prints every function of each file, not only the
refused ones, for an editor's gutter (its `format` is its own, not
`--ovm-stats`'s):

```json
{"format":1,"kind":"tier-check","olang":"1.0.0","refused":1,"errors":0,
 "files":[{"file":"/p/main.ol","error":null,"functions":[
   {"name":"fib","line":5,"col":4,"end_line":6,"verdict":"bytecode","reason":null,"code":null,
    "docs":null,"ambiguous":false,"pinned":"olang.toml","allowed":false,"error":false},
   {"name":"doubled_later","line":9,"col":4,"end_line":12,"verdict":"refused",
    "reason":"Compilation failed: it contains a `spawn` (a task starts on the interpreter)",
    "code":"task","docs":"docs/tooling.md#tier-task","ambiguous":false,"pinned":null,"allowed":false,"error":false}]}]}
```

`allowed` is a refusal an [`// allow: tier`](#allowing-one-line) on
its `fn` line keeps out of the report (not counted, never an error).
`verdict` is `bytecode` (it compiles; native code is decided per call,
at run time), `refused` (with the compiler's `reason` and its `code`,
below), `unchecked` (its refusal names a top-level binding this pass
does not evaluate) or `unknown` (not reached: a nested or trait
function). Lines are 1-based. A file that does not parse, or whose
declarations fail to evaluate, has its `error` and no functions. Only a
file's own functions are compiled, so a file that imports a large
library is checked in the time its own functions take.

**Pins.** A function can be pinned *must be native*: a `// studio:
native` comment on the line above its `fn` (comment lines between are
allowed) or at the end of that line, or a pattern in the project's
`olang.toml`:

```toml
[check]
promote = ["shape"]
native = ["rt_frame", "ly_*", "lib/engine.ol:rt_*"]   # a name, a * pattern, or path:pattern
```

`olang check --tier` fails (exit 1, `"error": true`) on a pinned
function the tier refuses, whatever `promote` says; `promote = ["tier"]`
still makes every refusal an error. A run decides the rest — whether a
pinned function that compiles stays on native code — through its tier
statistics (`--ovm-stats=json`, below).

### Why a function stays on the tree-walker

Each refusal has a `code`, the anchor of its entry here.

<a id="tier-task"></a>
**`task`** — it starts a task (`spawn`): a task runs on the interpreter.
Move the work the task does into a function of its own; that function
compiles and runs natively inside the task.

<a id="tier-callee"></a>
**`callee`** — it calls a function that cannot compile and that the tier
cannot call through its bridge. The callee's own reason is the one to
fix; once it compiles, so does its caller.

<a id="tier-undefined"></a>
**`undefined`** — it names something nothing in its scope defines: a
missing `use`, usually. A concatenated browser bundle may resolve it; a
native run would not.

<a id="tier-cell"></a>
**`cell`** — it reads or writes a `cell` in a way the tier cannot
represent. Pass the cell's value in and the new value out, and keep the
cell in the caller.

<a id="tier-capture"></a>
**`capture`** — it calls a value it cannot resolve at compile time (a
dynamic call), or captures what the VM has no environment for. Name the
function directly, or pass what it needs as a parameter.

<a id="tier-boundary"></a>
**`boundary`** — a value crossing between the tree-walker and compiled
code cannot be converted: a module passed as a value, a result the VM
cannot hand back. The function compiles, so `olang check --tier`
passes, but each such call runs on the tree-walker: tier statistics
show it as a *fallback*. Pass the data instead of the module, or the
module's function itself. (A builtin passed as a value — `apply(len,
xs)` — crosses: it is called in compiled code through the bridge, by
name.)

<a id="tier-ambiguous"></a>
**`ambiguous`** — two functions share its name (two modules, a trait
default and an override). It is compiled by identity, as a value; when
that is refused, the reason is its own.

<a id="tier-depth"></a>
**`depth`** — its chain of callees to compile is too deep.

<a id="tier-unsupported"></a>
**`unsupported`** — the body uses a form the bytecode compiler does not
compile (the message names it).

### Project rules

Run project-specific lint rules, written in olang over the [meta
AST](stdlib.md#meta--the-program-as-data-the-open-ast), alongside the
built-in type checks:

```bash
olang check --rules rules.ol .
```

A project names its rules files once, in `olang.toml`, and `olang check`
with no `--rules` runs them over the project's files (a path is
relative to the project's root; `--rules` names the one file instead):

```toml
[check]
rules = ["tools/rules.ol"]
```

A listed file that cannot be read, does not parse or defines no rule is
a problem, as it is with `--rules`. The language server does not run
project rules (they are olang, run in a sandbox, once per check); the
editor shows the checker's own findings.

Define a rule as a top-level function whose name begins with `rule_`. The
function takes one argument: the file's AST, flattened to a list of nodes.
Each node is a map with at least a `kind` field and a `line` field; a
`match` arm's `pattern` and a function's `parameters` are nodes too (a
pattern's `form` and `variant`, a parameter's `type` with its `form` and
`fields`), so an exhaustiveness or a record-shape rule is written here
rather than in a tool of its own. The
function returns a list of findings. A finding is a message string or an
object (or map) with `message` and `line` fields.

```olang
// rules.ol
share fn rule_no_bare_unwrap(nodes) =
    nodes
      |> filter((n) => n.kind == "call" && n.target == "unwrap")
      |> map((n) => { message: "bare unwrap(): use match or ?", line: n.line })
```

`olang check` runs each rule against every checked file except the rules
file itself. It reports each finding with the file name, line number, and
rule name. Findings count as problems, so any finding produces a non-zero
exit.

The command applies the following behaviors:

- A rule that raises an error is reported with its name.
- A rules file that defines no `rule_*` functions is an error.
- A finding on a line that an `// allow: <rule name>` comment covers
  (`// allow: rule_no_bare_unwrap`, [as an advisory's](#allowing-one-line))
  is not reported; `// allow: all` does not cover a rule.

### Promoting advisories

A project promotes the checker's own advisories to errors in
`olang.toml`, without writing a rule:

```toml
[check]
promote = ["exhaustiveness", "shape"]
```

The classes are `exhaustiveness` (a `match` over an enum or a literal
union that misses a case), `shape` (a literal key a declared record
shape does not carry, read or written — its read is `()` — and a field
read off a value of a declared struct type that the declaration does
not have, which raises), `keys` (a `.key` read no map or
object built in the module has), `result` (a discarded `Result`),
`shadow` (a `let` that takes a stdlib module's name — `let cell = …`
turns every later `cell.get` in its scope into a run-time failure far
from the `let` — and a parameter, lambda parameter, loop variable or
pattern binding of a module's name that its scope then reads as the
module: `fn table_sort(rows, col) = col.sort_by(…)`), `copy` (a loop that copies the collection it is
building on every pass: a temporary read again after the rebind, or a
prepend — see [the rebind forms](performance.md#the-rebind-forms)),
`no_effect` (an expression statement whose value is dropped and whose
evaluation does nothing else), `tier` (a function `olang check --tier`
finds the bytecode tier refuses), and `all`. A
promoted finding fails `olang check` and shows as an error in the
editor, and its message says which block promoted it.

A class is a field of the finding, set where the checker makes it — the
`code` of `--format json` — never read back from its wording, and the
class names are the contract: a message may be reworded in any release,
its class does not change. `all` means every class the running olang
knows: a later olang may add a class, or a class such as `keys` or
`shape` may learn to find more, so a project that promotes `all` can
see a new failure when it moves to a newer olang —
name the classes instead to fix the set. `unused`, the class of the
hint for a name nothing reads, is a hint and never promoted. One line's
finding of any class is silenced by an [`// allow:`
comment](#allowing-one-line), which wins over `promote`. A name
`promote` does not know is warned about, with the nearest class, by
`olang check` and `olang doctor`, as is any key or section of
olang.toml that olang does not read (a near miss such as
`[dependancies]` or `autors`; other tools' sections, Loom's `[bundle]`
and Studio's `[plugin]`, pass).

## `olang eval`

`olang eval '<source>'` parses and runs the text with the REPL's
semantics — the last expression's value is printed unless it is Unit —
and no file is written. It is the probe for checking a stdlib behavior
or a library call without a scratch file:

```bash
olang eval 'fs.ext("a.PNG")'
olang eval 'use markdown { to_html }
to_html("- [ ] one")'
```

Modules resolve as they would for a file in the working directory's
project, so a probe typed at a project root reaches the shelf libraries
that project depends on. The same rule applies to `olang <file>`: a
script whose own directory has no `olang.toml` falls back to the working
directory's project for dependency and shelf lookups, so a scratch
script kept outside the repository can still `use` what the repository
does.

A project whose `[package] olang` names a newer olang than the one
running is still run, after a warning on standard error naming both
versions (`[package] olang` is a minimum, as [packages.md](packages.md)
says).

## `olang repl --serve`

`olang repl --serve` is the REPL as a protocol, for an editor that keeps
one long-lived REPL per project. It speaks line-delimited JSON over
stdio: one request a line in, one reply a line out, each reply carrying
the request's `id`.

The session is bound to the project the working directory is in, as
`olang eval` is: the project's dependencies and shelf libraries resolve.
What a program prints while the session serves is collected and sent with
the evaluation that printed it (`out`); stdout belongs to the protocol.
While an `eval` runs, what it prints is also streamed as it is printed,
`{"event":"out","id":7,"text":"…","err":false}` (the feature `stream`):
batches at most every 50 ms, the last one before the reply, the first
512 KB of them; the reply's `out` is still the whole, so a client that
draws the stream replaces it with `out` when the reply comes.

**Handshake.** On start the server says
`{"event":"hello","protocol":1,"olang":"1.0.0","root":…,"pid":…,"ops":[…],"features":["bind","pure","cell","caps","profile","distinct"]}`
(`features`: what an `eval` takes beyond the protocol's first version —
`scene`: a gg plot value is said with its scene, below; `daterange`:
a table view's range filter takes a Date or Datetime column, below;
`stream`: an `eval`'s printed lines come as `out` events while it runs;
a notebook's kernel says `notebook` too, below).
A client that does not know the protocol's version stops there. `{"op":"hello"}`
asks again.

**Scopes and lines.** An `eval` with a `file` runs in that file's scope:
the file's `use`s, functions, types, and the top-level `let`s that call
nothing, loaded from disk the first time, and whatever was evaluated
there since. A top-level `let` that calls something (it may read a file
or start a window) is not loaded; naming it says so. An `eval` with no
`file` runs in the session's own scope. A snippet evaluated at `line` (1-based)
is parsed as if it stood there, so every line an error names is the
file's. Code is evaluated as written: a function's parameter named in a
line of its body is not bound, and the error says whose parameter it is
(`kind` `unbound`, with `fn`, its `fn_line` and its `params`).

**A function's parameters.** `"bind": {"amount": "200", "rate": "10"}`
gives them values for one evaluation: each an expression, evaluated in
the file's scope before the code (a value that fails says which:
"the value given for `amount` fails: …"); afterwards each name is taken
back (a binding the scope had is put back).

**Purity.** `"pure": true` evaluates under no capability at all — no
filesystem, network, process, database or environment (the [capability
gate](packages.md#capabilities) with nothing granted, on every tier). Code
that reaches for one is refused: `kind` `impure` and `cap`, the
capability it wanted (`"fs"`). The session's own grant is back for the
next request. An editor re-runs a result on save only when it is pure
this way (and fast); the clock, randomness and printing are not
capabilities and pass.

**A profile of one evaluation.** `"profile": true` samples the evaluation
(every 100 µs) and adds `profile` to its reply: the document `olang
profile --format json` writes ([As data](#as-data-and-live)), its frames
named as the stack says them — a cell's function at the line its cell
declared it, the notebook's text as the kernel ran it. An evaluation
asked while another profile samples the process (`olang profile`'s,
`runtime.profile_start`'s) runs unsampled and says none.

| Request | Reply |
|---|---|
| `{"op":"eval","id":1,"code":"rows(10)","file":"/p/calc.ol","line":12,"bind":{…}?,"pure":true?}` | `{"id":1,"ok":true,"eval":7,"value":V,"binding":"x"?,"ms":0.4,"server_ms":0.5,"tier":"native","calls":{"bytecode":1,"native":1},"out":"…","notes":[…]?}` — or `"ok":false` with `"error":E` |
| `{"op":"expand","id":2,"h":3,"start":0,"count":100}` | more of a held value: a list's `items` (`{"i","v"}`), a map's or record's `entries` (`{"key","v"}`), a string's text (`s`), Bytes as base64 (`b64`) |
| `{"op":"expand","id":3,"h":3,"table":true,"start":0,"count":50,"sort":{"col":2,"desc":true}}` | a table's `rows` (`{"i": index, "c": [cell text…]}`), sorted by a column; `n` and `total` its rows |
| `{"op":"expand","id":3,"h":7,"table":true,"start":0,"count":200,"filters":[…],"sort":{…}?,"summary":true?}` | a Frame's (or a Series') view, filtered, sorted, and summarised (below): `{"ok":true,"n":…,"total":…,"cols":[…],"rows":[…],"sum":[…]?}` |
| `{"op":"expand","id":3,"h":7,"distinct":{"col":1,"contains":"we"?},"start":0,"count":50}` | a Frame's (or a Series') column's distinct values, the commonest first (ties by their text), a page of them: `{"ok":true,"col":"region","n":…,"start":0,"values":[["west",120],[null,3],…],"sampled"?}` — `n` how many there are (`contains` kept: their text holds it, case-sensitive), a null one value (`null`), read over the column's first million rows |
| `{"op":"expand","id":3,"h":12,"chart":{"x0":1.5,"x1":4.0}}` | a chart's description with each xy series cut to x in [`x0`, `x1`] (either `null`: open) and decimated again: `{"ok":true,"chart":C}` |
| `{"op":"release","id":4,"eval":7}` | the evaluation's handles are let go (the last 200 are kept anyway) |
| `{"op":"interrupt","id":5}` | `{"running":true}`; the running evaluation fails with `"kind":"interrupted"` |
| `{"op":"ping","id":8}` | `{"running":…}` — answered even while an evaluation runs, so a client tells a busy session from one not answering |
| `{"op":"reload","id":6,"path":"/p/util.ol"}` | `{"ok":true,"scopes":n,"ms":…}`, or `"ok":false,"kept":true` with the reason |
| `{"op":"render","id":7,"h":9,"dark":true,"scale":2,"width":640,"loom":"/path/to/loom"?}` | `{"ok":true,"png":"<base64>","width":…,"height":…}` (logical pixels) |
| `{"op":"stats","id":9,"reset":true}` | `{"ok":true,"stats":S}` — where each function ran in the evaluations since the last reset ([tier statistics](#tier-statistics---ovm-statsjson)); `reset` starts them again |
| `{"op":"command","id":10,"line":":type xs"}` | a `:` command, run as the terminal REPL runs it (below) |
| `{"op":"commands","id":11,"prefix":":t"?}` | `{"ok":true,"commands":[{"name","aliases","usage","summary","arg","group","client","words"?}…]}` — every command, for completion and help |
| `{"op":"complete","id":12,"line":":cd sr","pos":6}` | `{"ok":true,"start":4,"items":[{"label","kind","detail"}…],"command":{"name","usage","summary","client"}?}` — completion of a prompt line at `pos` (characters); `items` replace `line[start..pos]` |
| `{"op":"eval","id":13,"cell":"c3","code":"let t = total(xs)","line":21}` | as any `eval`, with `defines` (the names the cell defines, in order); what the code defines is cell `c3`'s, and an evaluation of `c3` again drops it first |
| `{"op":"deps","id":14,"cells":[{"cell":"c3","code":"let t = total(xs)","line":21}…]}` | `{"ok":true,"cells":[{"cell":"c3","ok":true,"defines":[{"name":"t","line":21,"col":1}],"uses":[{"name":"total","line":21,"col":9},{"name":"xs","line":21,"col":15}]}…]}` — each cell's names read from its text, never run (below) |
| `{"op":"drop","id":15,"cell":"c3"}` | `{"ok":true,"dropped":["t"]}` — what the cell defined, forgotten (a cell deleted) |
| `{"op":"fields","id":22,"name":"sales","path":[…]?}` | `{"ok":true,"name":"sales","kind":"frame","t":"Frame","fields":[{"name":"region","type":"String"}…]}` — what a value held by name is made of (below); with `code` in place of `name`, what completes at its end |
| `{"op":"vars","id":16}` | `{"ok":true,"eval":9,"vars":[{"name":"sales","cell":"c3","v":V,"bytes":27411},{"name":"region","param":true,…}…]}` — a kernel's names (below) |
| `{"op":"reread","id":17,"text":"…"?}` | `{"ok":true,"params":["region"],"removed":[],"grant":false,"problems":[…]}` — a kernel's front matter read again (below) |
| `{"op":"set","id":18,"cell":"c5","value":"0.12"}` | `{"ok":true,"eval":10,"name":"lift","value":V}` — the value an editor chose for a cell's control (below) |
| `{"op":"record","id":19}` | `{"ok":true,"recording":true}` — from now each cell's evaluation records its inputs (below); `{"op":"record","path":"run.olt"}` writes the trace and stops (`{"ok":true,"path","cells","events","bytes"}`), `"stop": true` stops without writing |
| `{"op":"replay","id":20,"path":"run.olt"}` | `{"ok":true,"replaying":true,"cells":[{"cell","line","events"}…],"notebook":{"params","controls"}}` — from now each cell's evaluation is served its recorded inputs; `"stop": true` ends it |
| `{"op":"page","id":21,"doc":{…},"mode":"app"?,"text":"…"?}` | `{"ok":true,"html":"…"}` — the notebook's page for an outputs document (`olang notebook export`'s), live controls with `mode: "app"` |
| `{"op":"reset"}`, `{"op":"shutdown"}` | the scopes forgotten (and what each cell defined, and every control's value set); the server ends |

**A recorded run.** After `record`, a cell's evaluation (an `eval` with
`cell`, not `pure`) runs with the [Open Timeline](#olang-record-and-olang-replay--the-open-timeline)
attached — the interpreter tier for that evaluation, the tiers back after
— and its reply says `"record": {"events": n}`; a cell evaluated again
replaces its stretch. The trace is an `.olt` whose `cells` say each cell's
id, code, line and its stretch of `events`, whose `source` is the
notebook's program with its params as bound, and whose `notebook` says
the params' and controls' values: a Run All in file order makes it the
program's own run, so `olang replay run.olt` replays it too. After
`replay`, a cell is served the stretch recorded for it (found by its id
and code, else its code, else its id) and its reply says `"replay":
{"events", "recorded", "code_changed"?, "missing"?, "diverged"?}` —
`diverged` the message when its calls stepped off the trace (another
call, other arguments, fewer calls than recorded).

**An output as HTML.** `"html": true` (or `{"markdown": true}`) on an
`eval` adds `html`, the output as `olang notebook export`'s page draws it
with its controls live, and `text`, its value's words: what a published
app swaps in.

**A notebook's kernel.** `olang repl --serve --notebook x.olmd` serves
one notebook to an editor. The
notebook is read with olang's notebook reader ([Notebooks](notebooks.md)):

- its folder is the working directory, wherever the server is started,
  and its project the folder's (`use`, the dependencies);
- its front matter's params are bound in its scope, each on its own
  line, as `olang run` binds them;
- its front matter's grant is the session's, intersected with the
  enclosing project's `[capabilities]` (each dependency's attenuation
  too) and `OLANG_DENY`, as `olang run` takes them; a grant olang cannot
  read (an unknown capability, front matter outside the subset) is no
  capability at all, never a session wide open;
- every evaluation is in the notebook's scope: one with no `file` (a
  cell's, a prompt's) and a `:` command alike, and an error's `file` is
  the notebook. A cell's code is sent with its first line (`line`), so
  every line an error or a frame names is the file's.

The handshake says the notebook:
`"notebook":{"path":…,"title":"Sales","params":["region"],"problems":[{"line","column","message"}…],"refused":false}`
— `problems` are the notebook's own as olang reads it (a bad option, an
unclosed fence), `refused` that its grant could not be read. The
notebook is read as the server starts; `reread` reads its front matter
again (below).

**Cells.** An `eval` with `cell` (any id: a string or a number) is that
cell's evaluation, in any session. Its top-level definitions — `let`s,
functions, types (an enum's variants with them), traits, error types and
the names a `use` binds — are the cell's, and an evaluation of the same
cell drops them before it runs, so a cell edited and run again replaces
what it defined rather than adding to it (a name it no longer defines is
gone). What a cell defines is kept whether its evaluation succeeds or
fails halfway; a cell that does not parse changes nothing. A cell's
value is the language's: its last expression's, `()` when it ends in a
`let`, a function or any other declaration (no `binding`; the prompt's
`let x = …` still answers `x`'s value with `binding`), and its reply
says what it defined (`"defines":["fmt_money","pct"]`). A frame in a
cell's function names the line the cell declared it on. Whether one cell
may define what another does, or use what a later one does, is the
notebook's rules, which `olang check` holds (the kernel runs cells in
the order it is asked).

**What an evaluation reached.** Every `eval` reply says `caps`: the
capabilities the evaluation reached, as the grant names them (`fs`,
`net`, `db`, `proc`, `env`), whether the grant let it through or refused
it; `[]` when it reached none — it is pure, and an editor may run it
again on its own. It is the gate's own classification, on every tier (a
function the VM or native code runs included). The clock, randomness
and printing are no capabilities.

**A cell's names, without running it.** `deps` reads each cell's code
(its first line `line`, so every place is the file's) as `olang check`
reads a notebook's: `defines`, the top-level names it defines in order
(`"use": true` on a name a `use` binds), and `uses`, the names it reads
that none of its own statements defines, at their first use as a word —
a builtin is none, a name read in a function the cell declares is one.
A cell that does not parse answers `"ok":false` with its parse `error`.
`deps` is answered as it is read, like `ping`: an editor asks as a
person types, while a cell may be running.

**A kernel's names.** `vars` lists the notebook's names: its params
(`"param": true`) and what each cell defined (`cell`), each with its value
as an evaluation's (its handle held under `eval`, a container's items
left to `expand`) and `bytes`, its size in memory roughly (a long
container measured on its first thousand items and scaled; a Frame's
columns). `drop` forgets what a cell defined, as a cell evaluated again
does first.

**A value's fields.** `fields` says what a value held by `name` is
made of, for an editor's completion: a Frame's columns with their types
(`kind` `frame`), a map's keys and a record's fields with their values'
types, in the order they were set (`map`, `record`), a list of maps'
columns, each typed by the first item holding it (`table`); a Series, a
list, a module (a map of functions: `ods`, a `use`d package; `kind`
`module`) and any other value have none (`fields` empty). `path` walks keys
into it (`m.inner` is `"name": "m", "path": ["inner"]`). Nothing is
evaluated, so it answers at once and under no capability; a name that is
not bound is an `unbound` error. At most 500 fields are said (`more` how
many past that).

Given `code` — the text before an editor's caret, a cell's — in place of
`name`, `fields` answers what completes there, as the language server
reads the place: inside a string where a column is named (`t["`,
`ods.select(t, ["`, `ods.group_by(t, "`, a chain's `|> ods.sort_by("`,
an aggregation's column, `ods.pivot`'s and `ods.join`'s) or a key
(`map_get(m, "`), or after `m.`. Its receiver's chain is followed from
the value the name it starts from holds — a `group_by` makes its keys and
aggregates, `select`, `drop`, `with_column`, `rename` and `join` theirs, a
filter, a sort or a head keeps them — and a name not held is taken from
`known` (an editor's cached copy: `{"sales": {"kind": "frame", "fields":
[{"name", "type"}…]}}`), else from what `code` itself binds:

`{"ok":true,"context":true,"receiver":"sales","want":"columns","typed":"re","quoted":true,"live":true,"kind":"frame","fields":[…]}`

— `want` is `columns` (a Frame's), `keys` (a map's or a record's) or
`any` (after `t["`), `typed` what is typed of the field so far, `live`
whether a value held answered; `"context": false` where no field
completes.

**A kernel's front matter, again.** `reread` reads the notebook's front
matter from `text` (an editor's unsaved buffer) or the file: each param
whose value changed (or is new) is bound again in the notebook's scope
on its line, one gone is forgotten, the grant is applied anew. The reply
names the params that changed (`params`), those gone (`removed`), and
whether the grant changed (`grant`); what cells defined stays, so an
editor runs again only what reads a changed param.

**A cell's control.** A cell whose evaluation binds a control (`ui.slider`,
`ui.select`, `ui.date`, `ui.text`, `ui.toggle`, `ui.brush`: each
answers `runtime.bind`) says it in its reply:

`"control":{"kind":"slider","name":"lift","value":V,"default":V,"min":0.0,"max":0.3,"step":0.01}`

— `kind` one of `slider`, `select`, `date`, `text`, `toggle`, `brush`;
`name` the one name the cell defines (the names a `use` binds aside);
`value` what the control answered and `default` what the cell gave, as
values are encoded; `min`, `max` and `step` a slider's (numbers, `step`
absent when it has none), `options` a select's (each a V), `chart` a
brush's chart (C). Only the fields that apply are there. A cell binding
several controls, or defining several names, says its first control
without `name`: an editor draws it read-only, and `set` refuses it.

`set` gives a cell's control a value: `value` an olang literal as text
(`0.12`, `"west"`, `true`, `[2.0, 5.5]`, `()`), evaluated under no
capability, then held to the control — a slider's within [min, max] (an
Int slider's an Int; out of range is refused, never clamped), a select's
one of its options, a date a `"YYYY-MM-DD"` String, a text a String, a
toggle a Bool, a brush `()` or `[x0, x1]` with x0 ≤ x1 (read as Floats).
The cell's name is bound to it in the notebook's scope in place (as
`reread` binds a param), and the value is kept as the cell's: when the
cell runs again its control answers it while it still fits — the same
kind and default as when it was set, within the range or options the
cell gives now — and forgets it otherwise. `drop` of the cell forgets
it; `reset` forgets all. Errors: `kind` `protocol` (no cell, no value, a
cell that bound no control) or `invalid` (a value the control cannot
hold, one that does not parse or fails, a control with no one name).

**Commands.** Every command of the interactive REPL (`:help`, `:type`,
`:time`, `:sh`, … — `commands` lists them) runs through `command`, in the
session's own scope, by the very code the terminal REPL runs: what it
says is the terminal's text. An `eval` with no `file` whose code is a
command line (`:word`, `!shell`, `quit`) is one too. While it runs, what
it says is streamed — `{"event":"out","id":10,"text":"…","err":false}`
for each piece (a `:sh` command's lines as they come) — and an
`interrupt` stops it (a `:sh` command and everything it started are
ended). The reply:

`{"id":10,"ok":true,"command":":type","out":"xs : List<Int>","lines":[[{"s":"xs : List<Int>"}]],"ms":0.4,…}`

- `out` is the text, `lines` the same text as lines of styled runs
  (`{"s", "c": "red"|"green"|"yellow"|"blue"|"magenta"|"cyan"|"white"|"gray", "b": bold, "d": dim}`);
- what the command found, when it found it: `value` (V, the last of
  `values`, each `{"label","value"}` — `:type`, `:time`, `:inspect`,
  `:set`, `:let`, `:run`, `:env`'s `bindings` as a table) with `eval`
  for its handles; `type`; `time_ms`; `bench` (each run's ms); `cwd`
  (`:cd`); `tutorial` (`{name, description, steps: [{title, description,
  code, expected, explanation, hints}]}` — `:tutorial_run` takes no
  answers over the protocol, so an editor takes the steps); `commands`
  (with `:help`); `open` (`"tiers"` for `:ovm` and `:stats`,
  `"profiler"` for `:profile`: a richer place an editor may have);
- `effects`: what an editor does with its own interface — `clear` (the
  screen, `:clear`), `clear_history`, `multiline` (`:ml`), `quit`
  (`:quit` ends nothing: the editor ends or restarts the session);
  the commands marked `client` in `commands` are these;
- an unknown command: `"unknown":true` and the nearest, `suggest`;
- `"ok":false` with `error` when the command failed (`:inspect` of
  nothing), `"interrupted":true` when it was stopped.

A prompt's evaluations (no `file`) are the session's history (`:history`,
`:!n`), and a watched variable they change is said in the eval's reply
(`"watched":[…]`).

**Values (V).** Every value has `k` (its kind), `t` (its type) and `s`
(a one-line form, at most 200 characters; a string's text, up to 2,000):

- `int`, `float`, `bool`, `unit`, `str` (`n` characters; a handle `h`
  past 2,000; `img`, the absolute path, when the string names a picture
  that exists; `svg: {w, h, bg?, text}` when the string is an SVG
  document, as `plot` answers: its size, the background a first `rect`
  as wide as it paints, and its whole text; `chart` (C) when it is a
  chart that carries its description — every `plot` and `viz` chart
  does — and then always a handle `h`), `enum`, `fn`, `decl` (a
  declaration evaluated at the prompt or in a file);
- `result`: `ok` and the inner value `v`;
- `list` and `tuple`: `n`, a handle `h`, the first 20 `items`; a list of
  maps or records also says `table: {cols}`, and a list of numbers (or of
  `(x, number)` pairs) `series: {lo, hi, pts}` (at most 512 points: past
  that, each bucket's least and most);
- `map` and `record`: `n`, `h`, the first 20 `entries`, in the map's order;
- `frame` (an `ods` Frame) and `series` (an `ods` Series, one column
  named `value`): `n` rows, `h`, `table: {cols, types}` (each column's
  name and its type: `Int`, `Float`, `Bool`, `String`) and the first 20
  `rows` (`{"i", "c": [cell…]}`, a cell its text — a float to six
  significant digits — or `null`); `sum` in `table`, each column's summary
  (below); a numeric Series also `stats: {count, nulls, mean, min, max,
  hist}` (`hist` 24 bins over min … max); `expand` with `table` reads
  the rest by range, filtered and sorted (below);
- `bytes`: `n`, `h`, and for a picture `img: {format, w, h}` and the
  picture itself (`png`, base64);
- `view`: a Loom view node (`#{ role, key, props, children }`) — `role`
  and `h`, for `render`.

**A column's summary.** A table's `sum` is a list aligned with its `cols`,
one summary a column, computed by the data stack over the column's
first 1,000,000 rows (a longer one says `"sampled": 1000000` on `table`,
so a ten-million-row Frame stays cheap; a million rows of four columns
take about 10 ms, 70 at worst — a million distinct strings):

- an `Int` or `Float` column: `{"nulls", "distinct", "min", "max",
  "mean", "hist"}` — `hist` 20 equal-width bins over [min, max] (every
  value in the first when min is max), nulls not counted; `min`, `max`
  and `mean` numbers over the finite values, absent when there are none;
- a `Date` or `Datetime` column: `{"nulls", "distinct", "min", "max",
  "hist"}` — `min` and `max` its first and last as ISO text, `hist` 20
  bins between them;
- any other: `{"nulls", "distinct", "top"}` — `top` the six most frequent
  values, most first, ties by first appearance, each `["cell text",
  count]` (the text the table's cells use); a null is never in `top`.

`distinct` counts the values that are not null.

**A table's view.** `expand` with `table` on a Frame or a Series takes
`filters`, a list ANDed together, each on a column by index:

- `{"col": 2, "range": [100.0, null]}` — inclusive bounds, either `null`
  (open), on a numeric column, or a Date or Datetime one with ISO text
  bounds (`["2024-01-01", "2024-03-31"]`; a date alone is its midnight);
  any other is a `protocol` error naming it; a null fails;
- `{"col": 1, "values": ["east", "west"]}` — the cell's text is one of
  them; `null` in the list keeps null cells;
- `{"col": 0, "contains": "gad"}` — the cell's text contains it,
  case-sensitive (the rows `ods.contains` keeps); a null fails;
- `{"col": 3, "nulls": true}` — only nulls (`false`: none).

A `sort` (nulls last, as ever) orders what the filters keep. `n` is the
view's rows, `total` the table's; each row's `i` its index in the whole
table, so `ods.take` reads it. The view's order is cached by handle,
filters and sort, and let go with the handle. `"summary": true` adds
`sum`, the view's column summaries as a table's `sum` has them (`sampled`
past a million rows). Without `filters` a view is the whole table.

**A chart (C).** A chart's description, as `plot.describe` answers it
([the data stack](ods.md#a-charts-description)), with each xy series at
most 1,500 points: a line's or an area's least and most y in each of 750
buckets of x, in x order; a scatter's at an even stride. A series so
decimated says `"n"` (its points) and `"cut": true`; `expand` with
`chart` reads a range of x afresh, for zooming past what the reply
carried.

**A gg plot (scene).** A value that is a plot `use gg` built is said as a
`str` of type `Plot`: its SVG (`svg`, as above) and `chart` its scene —
`{"v":2,"type":"scene","width","height","theme","title","alt","zoomable","n",
"palettes","layers":[{geom,fields}],"panels":[…],"ops":[…],"cut"}`
(`plot.gg_scene`'s JSON). `ops` are drawn in order, in plot pixels:
`rect`, `line`, `path` (`d`: its vertices in tenths of a pixel, flat,
each after the first as its step from the one before; a path no pointer
reads drops the vertices that land where the last did), `points` (`x`, `y`, `r`, `fill` an
index or one each into `pal`, `shape`), `text` (`y` the line's middle,
`align`), `clip`/`unclip`; a frame colour is a role (`@ink`, `@ink2`,
`@grid`, `@axis`, `@surface`, `@strip`, `@panel`), a data colour
`#rrggbb[aa]`. A mark a pointer reads says its layer (`L`) and its
values: `tips` (by field: a list, one per point or vertex; a String, the
same for every one; or `{"lv": [values], "ix": [each one's index]}` for
a field of few values) or `tip` (a bar's). A brush over a gg plot
(`ui.brush(p)`) says this scene as its control's `chart`, and its SVG.
Points are thinned for the screen (one a pixel and colour, then an even
stride; 12,000 at most): `n` and `cut` say so. Each panel says its box
and its axes (`kind` `continuous`, `discrete` or `time`; `trans`; `lo`
and `hi` in the transformed space). `expand` with `chart: {x0, x1,
panel}` renders the plot again with its horizontal axis over that range
(data units) and the other fitted to what shows.

**Errors (E).** `{"kind","message","file","line","col","stack","hint"}`,
`kind` one of `parse`, `runtime`, `unbound` (a parameter — with `fn`,
`fn_line`, `params` — or a top-level binding not loaded), `impure` (a
`pure` evaluation reached for `cap`), `interrupted`, `reload`, `render`,
`gone` (a handle let go). `stack` lists the frames outermost first, each
`{"name","file","line"}` (where the function is declared).

**Interrupt.** Requests are read on a thread of their own, so
`interrupt` reaches a running evaluation. Every tier polls one flag: the
tree-walker at each statement, loop iteration and call, the VM at each
backward jump and self tail call and at one call in 256, native code at its back edges (compiled
with the poll only in a serving process, so a program run any other way
compiles exactly the code it always did). The evaluation fails with
"interrupted" and the session stays as it was; a loop the interpreter had
promoted to the VM keeps the values it had when it was promoted.

**Reload.** `reload` re-reads a saved file. If it does not parse, nothing
changes (`kept`). Otherwise the file's scope gets its declarations again,
every scope that `use`s anything is declared again (its functions then
call the module loaded now; what was evaluated there by hand stays), and
the bytecode and native tiers start afresh (compiled code links its
callees directly).

**Views.** `render` draws a held Loom view headless with Loom's
`render_view` (lib/test.ol), in the theme and scale asked for, at its
natural size within `width`. Loom comes from the project's dependency,
else from the `loom` path given (a project with its own `loom`
dependency keeps it).

## `olang bench`

`olang bench` times `.ol` programs in a repeatable way, and can compare
a run against a saved baseline so you notice when something gets
slower:

```bash
olang bench kernels/                     # every .ol file in the directory
olang bench fib.ol --runs 10             # more timed runs (default 7)
olang bench kernels/ --save base.json    # store medians as a baseline
olang bench kernels/ --against base.json # compare to a stored baseline
olang bench kernels/ --in-task           # every run on a spawned task
```

`--in-task` runs each program on a spawned task instead of the main
thread — the execution context every `http.serve` worker and `spawn`
body has — so a task's speed can be pinned to the main thread's with a
saved baseline. `olang --in-task run file.ol` is the same switch for one
run.

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

**As data.** `--format json` prints line-delimited events instead of the
table, for an editor:

```bash
olang bench fib.ol --format json --against base.json --profile profiles/
```

```
{"event":"start","olang":"1.0.0","runs":7,"in_task":false,"files":["/…/fib.ol"],"against":"base.json"}
{"event":"result","name":"fib","file":"/…/fib.ol","median_s":0.0392,"min_s":0.039,"max_s":0.0413,"cv_pct":3.1,"runs":7,
 "times_s":[…],"stable_output":true,"baseline_s":0.0371,"delta_pct":5.6,"threshold_pct":6.3,"verdict":"same",
 "profile":"profiles/fib.json"}
{"event":"finished","regressed":false,"files":1,"saved":null}
```

`verdict` is `new` (no baseline row), `same` (within the band
`threshold_pct`, max(5%, 2×CV)), `faster` or `slower` — the table's
colours as words; `regressed` is true when any row is `slower`, which
`--fail-on-regress` turns into exit 1. A failed run is
`{"event":"error","name","file","message"}`. `--profile DIR` runs each
file once more, untimed, under `olang profile --format json`, and leaves
that profile in `DIR/<name>.json`: the flame graph that explains a
regression, beside the number that shows it. `--save` writes the
baseline as it does without `--format`.

## `olang profile`

`olang profile` shows where a program spends its time, function by
function, and which tier ran each one:

```bash
olang profile report.ol                  # sample at 1 kHz, list the top 20
olang profile report.ol --interval 200   # finer sampling (microseconds)
olang profile report.ol --top 40         # a longer table
olang profile report.ol -- --input big.csv   # the program's own arguments
```

The run is an ordinary run — same tiers, same capability grant, same
`os.args()` — with a shadow stack sampled by a background thread. A
program that cannot be wrapped in one run — a server — opens the window
itself: [`runtime.profile_start()` and
`runtime.profile_stop()`](stdlib.md#runtime--what-this-binary-carries)
sample the same stacks and answer the counts as data, for a `/profile`
route. Time a thread spends parked — a channel receive, a sleep, a
join, `http.serve` waiting for a connection, an idle http worker — is
counted as blocked and charged to no function; a served app's profile
is its handlers, not its server's idle loop. A program that ends with
`os.exit` — every Loom application does, from its loop or a quit — is
reported all the same (text or JSON), as it exits; a windowed program is
sampled for its whole run, the platform's loop on the first thread and
the program on its own. The report names, for
every function that appeared:

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

**TIME BY TIER** splits the run's samples by where they ran:
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

### As data, and live

```bash
olang profile report.ol --format json -o p.json            # the profile as a document
olang profile report.ol --format json -o p.json --live live/   # and a snapshot of it every 500 ms
olang --profile-live live/ server.ol                           # any run, armed: sampled while live/attach exists
```

`--format json` writes one document (to `-o`/`--output`, else stdout after the
program's own output), the counts behind the report:

| field | |
|---|---|
| `frames` | each function seen: `name`, `file`, `line` (its `fn`; a lambda's, its enclosing function's), `lambda`, `builtin` |
| `stacks` | each distinct path: `f` (frame indices, root first), `t` (each frame's tier: 0 tree-walker, 1 bytecode, 2 native, 3 builtin — a promoted call's frames and recursion folded into one carrying the innermost tier), `n` (samples) |
| `functions` | per frame: `self` and `total` samples and ms, `tiers` (self samples by tier), `tier` (the one that ran it most) |
| `folded` | the stacks as folded lines (`a;b;c 12`), for any flame-graph tool |
| `tiers`, `samples`, `ticks`, `idle`, `blocked`, `worker_samples`, `interval_us`, `elapsed_ms`, `ms_per_sample` | the totals |
| `series` | the process over time, a point a snapshot: `t_ms`, `samples` by tier since the last point, `rss_kb`, `threads`, `cpu_ms` (the kernel's), `heap`, `program`, `values` (the counting allocator's, as `runtime.memory()`), `tasks` (counted apart), `olang_threads`, `parked`; with `--ovm-stats` also `calls` and `deopts` since the last point |
| `live`, `done`, `attached`, `attaches`, `pid`, `file`, `wall_ms`, `exit` | the run |

`--live DIR` writes the document so far (without `folded`) to
`DIR/<pid>.json` every `--live-every` ms while the program runs, each
written beside and renamed, so a reader never sees half of one; the
last is the final document (`done: true`). The series keeps an hour.

`--profile-live DIR` (or `OLANG_PROFILE_LIVE=DIR`, which reaches the
children a run starts: `olang test`'s, `olang bench`'s) arms any run: its
vital signs are written to `DIR/<pid>.json` from the start, and its
stacks are sampled while the file `DIR/attach` exists — an editor
attaches to a program it started by creating the file and detaches by
deleting it (the profile so far is kept). A `DIR/attach` there before
the run starts samples it from its first call; one made or removed while
it runs is noticed within 50 ms, not at the next snapshot. Until then the
shadow stack is off: an armed run costs what an ordinary one does. The
series' first point is the run's start. `OLANG_PROFILE_LIVE_EVERY` sets
its period (ms).

**Overhead**, measured on an Apple M5 Pro (6P+12E, 24 GB), macOS 27.0
(median of 5, load ~3): a JSON
profile with live snapshots +2.7% on a bytecode- and native-heavy run
(1.83 → 1.88 s), +2% on a tree-walker-heavy one (0.90 → 0.92 s); the
text report +0.5% and +6.7%; an armed run not attached, nothing
measurable. A snapshot of a few thousand paths is written in about a
millisecond, on the live thread.

### In the browser

A page has no second thread to sample from, so the web SDK's shim
exposes the same shadow stack in an instrumented form: every frame is
timed on entry and exit, which is exact rather than statistical.

```js
olangProfile.start()      // in the devtools console
// act: click, type, let a repaint happen
olangProfile.table()      // every olang function that ran: tier, calls, self ms, total ms
olangProfile.stop()       // the final report, and the instrumentation off
```

The rows say which view helper a repaint spends its time in and on
which tier it ran — the answer a wasm-level profiler cannot give,
since it sees Rust symbols, not olang functions.

## `olang build`

`olang build` bundles a program into a standalone executable: a single
file that runs on a machine with no olang installed.

```bash
olang build examples/tools/watch/main.ol            # → ./main
olang build examples/tools/watch/main.ol -o watch   # choose the output name
./watch --interval 500 ls                           # run it; its argv reaches cli.args()
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
`mathx`, and `collections` with `heap`, `deque`, `table`, `dsu`,
`bitset`, `alg`) travel with the executable. A plain `build` bundles a single
source file; a program that `use`s local `.ol` files or depends on
packages is built as an application:

```bash
olang build --app main.ol -o myapp   # its modules, its dependencies', and their assets
```

`--app` loads the modules the entry's `use`s name (without running the
entry), and carries every one of them — the project's and each
dependency's — with each package's `olang.toml` and the files it lists
under `assets` (`assets = ["fonts", "data/index.json"]`). The
application runs them from itself, resolving its `use`s exactly as it
did on the build machine, and reads its assets with
`asset.read(package, path)`. The trade for zero-dependency distribution is
size: the binary carries the runtime, so it is tens of megabytes (much
smaller from a `--release` olang than a debug one). On macOS the
appended data invalidates any code signature; re-sign the output if you
distribute it.

[`examples/tools/watch/main.ol`](../examples/tools/watch/main.ol) is a
self-contained tool built exactly this way — `cli` for its arguments,
`term` for color, `proc` and `os` for the commands it runs, and nothing
external.

A built binary also embeds its exact source, its `olang.toml` and
`olang.lock`, its capability manifest, and a checksum over all of them,
which `olang inspect` reads and verifies: see
[The transparent binary](packages.md#the-transparent-binary).

## `olang doc`

`olang doc` turns your doc comments into an API reference:

```bash
olang doc src/                    # → doc.html (themed, browsable)
olang doc lib/ -o api.html        # choose the output file
olang doc lib/ --format markdown > API.md   # Markdown to stdout instead
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

## Notebooks: `.olmd`

A notebook (`.olmd`) is markdown whose `olang` cells are one program.
`olang run`, `test`, `check`, `fmt` and `expand` take a notebook as they
take a `.ol` file, every location they report is a line of the notebook,
and `olang notebook` exports, pins and publishes one. The format, its
params and capabilities, and what each command does with a notebook are
in [Notebooks](notebooks.md).

```bash
olang sales.olmd                          # run it (or `olang run sales.olmd`)
olang run sales.olmd --param region=west  # override a front matter param
olang notebook export sales.olmd          # sales.html: a self-contained page
```

## The REPL

Bare `olang` (or `olang repl`) starts the interactive REPL. Anything
without a leading colon is olang; a line starting with `:` is a command,
and `:help` lists them:

| Command | Does |
|---|---|
| `:help <name>`, `:help <module>` | a function's documentation, a module's functions (`:help list` every function, `:help syntax`, `:help examples`) |
| `:help search <q>`, `:search <q>` | functions found by keyword |
| `:env` | the session's bindings |
| `:type <expr>`, `:time <expr>`, `:benchmark <expr>` | a value's type; an expression's wall-clock time; five timed runs |
| `:inspect <name>`, `:watch`, `:trace`, `:debug`, `:stack` | a variable in detail; a watched variable or traced function; stepping through an expression |
| `:profile <code>`, `:ovm`, `:stats` | the sampling profiler over code; what the bytecode tier compiled and ran |
| `:history`, `:!n`, `:clear` | this session's inputs; input `n` again; the screen, the environment or the history |
| `:cd`, `:pwd`, `:ls` | the working directory |
| `:sh <cmd>`, `!<cmd>` | a shell command |
| `:run <file>` | a program or a notebook, in its folder |
| `:pkg`, `:pkg load <dir>` | the current package's dependencies resolved again; another package by path ([In the REPL](packages.md#in-the-repl)) |
| `:tutorial` | the guided tutorials |
| `:ml`, `:end`, `:cancel` | multi-line input (below) |

`it` holds the last printed result, and TAB completes. The same
commands run over [`olang repl --serve`](#olang-repl---serve).

The doc comment convention of [`olang doc`](#olang-doc) reaches it: `:help
<name>` answers from the builtin registry first; when the name is not a builtin, it answers
from the user's own documented code — a `///` block above a
declaration entered in the session, in a file the session `:run`, or
in any module a program `use`d. `:help <module>` on a loaded file's
name shows its `//!` note and every documented item; `:help
module.name` qualifies the lookup to one file. Docs are re-read from
source at lookup time, so an edited file answers with its current
text.

```text
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

`--watch` reruns a program every time you save:

```bash
olang --watch script.ol       # rerun whenever any .ol or .olmd file nearby changes
```

Each run is a child process, so a crash or an `os.exit` ends the run,
never the watcher. Changes are detected by polling every `.ol` and
`.olmd` file at or below the script's directory (module edits trigger reruns too); a
save landing mid-run queues an immediate rerun. Ctrl+C stops both.

## `olang record` and `olang replay` — the Open Timeline

A bug that depends on a random number, the time of day or a network
reply is hard to catch, because the next run is different. The Open
Timeline is olang's answer: record a run once, and you can replay it
exactly, as many times as you like, anywhere.

```bash
olang record program.ol             # run, logging every nondeterministic input;
                                    # writes program.olt (or -o bug.olt to name it)
olang replay program.olt            # re-run: identical, from the trace alone
```

`olang record <file>` is the command form. If you are already running a
file directly, the `--record <trace>` run option does the same thing:
`olang --record bug.olt program.ol`.

Here is the whole idea on a program that rolls three dice. The trace
file replays the run even after the program is deleted:

```bash
$ olang record dice.ol
rolled [5, 2, 6], total 13
$ rm dice.ol
$ olang replay dice.olt
rolled [5, 2, 6], total 13
olang replay: clean — 3 recorded event(s) reproduced
```

### How it works

A program can only see the unpredictable outside world through a small,
fixed set of standard library calls: `random.*`, the clocks in `time`,
the environment, standard input and `exec` in `os`, file reads, `http`,
and the parts of `crypto` that draw random numbers. When recording, olang
makes each of those calls for real and writes down its result, in order.
When replaying, it catches the same calls and hands back the results it
wrote down.

Everything else in an olang program is already predictable. Values never
change, closures copy what they capture, and the random generator is
seeded. So once the inputs are the same, the whole run is the same: the
same dice, the same timestamps, the same environment, down to the last
digit.

The `.olt` trace carries the program's source inside it, so it is
self-contained. It replays from a folder where the program doesn't
exist, on another machine, or months later. A run that crashes is
recorded too, because the trace is written as the run fails, so you can
re-examine the failure as often as you need.

Replay is honest when things no longer line up. If the program now
makes a different call from the one in the trace, or the same call with
different arguments, replay stops at that exact point and tells you,
instead of quietly producing a different run. To make that check, the
trace stores a fingerprint of each call's arguments. Map iteration order
is also fixed (keys come back in the order they were set), so the two
never disagree by accident. A clean replay is proof that the recorded
inputs fully determined the run.

### Databases and threads

The database and other threads are covered as well. Every `db.` call is
recorded, including the connection handle `db.open` returns, so a
replay never opens the database at all: the handle is replayed, and so
is every query and statement made through it.

The trace is the main thread's view of the program. Whatever it receives
from other threads (a `chan.recv`, `chan.recv_timeout`, `chan.try_recv`
or `chan.ask` answer, or a `task.join` result) is logged in the order it
arrived. `chan.send` is logged too, so a replay fills no channel.

A `spawn`ed task gets a stream of its own. Every event records which
thread made it (0 for the main thread, otherwise the task's spawn
number), and a task's database calls, random draws and channel sends are
logged under its number. During replay the task runs against its own
stream: its effects are served from the log rather than repeated, so a
worker that wrote to a database while recording writes nothing on
replay. A program that spawns its tasks in the same order meets the same
streams.

A message can carry a handle instead of data, such as the reply channel
in `#{ req, reply }`, or a task. A handle is logged as a marker and
revived as a fresh one during replay. Every operation on it replays from
the log, so the revived handle is never really used. If any task drifts
from its stream, the replay fails: the verdict is "clean" only when every
thread reproduced.

### Limits

There are two. Recording and replay run on the interpreter tier, the
one place that sees every built-in call, so a recorded run gives up the
faster tiers. It is a debugging tool, not something for a hot path. And
an `http.serve` worker is not traced: requests arrive in an order no
trace could pin down, so a server's handlers run live, and a recorded
server says so once on stderr.

## Tier statistics (`--ovm-stats=json`)

`olang --ovm-stats=json[:PATH] program.ol` keeps, for the whole run,
where every function ran, and writes it as JSON at exit (to `PATH`, or
`ovm-stats.json` in the working directory) — at a normal end, an error,
or `os.exit`. `OLANG_OVM_STATS=json[:PATH]` does the same for any
command, `olang test` included. `--ovm-stats` alone prints a one-line
summary.

A `PATH` that is a directory — written with a trailing `/`, or one that
exists — gets a file a process, `<pid>.json`, and the processes the run
starts are told the same directory: `olang --ovm-stats=json:stats/
bench prog.ol` leaves the bench's own file and one for each of its runs
(with a single path, the last child to end replaced the others). A
reader adds them up, function by function.

```json
{"format":1,"kind":"ovm-stats","olang":"1.0.0","threshold":1,"interval_us":250,
 "sampled_ms":41.2,"ms_per_sample":0.31,
 "tiers_ms":{"interpreter":3.1,"bytecode":0.9,"native":36.4,"builtin":0.3},
 "functions":[
  {"name":"apply","file":"/p/main.ol","line":17,"col":4,"lambda":false,"tier":"interpreter",
   "calls":{"interpreter":400,"bytecode":0,"native":0},
   "self_ms":{"interpreter":0.6,"bytecode":0,"native":0,"builtin":0},"total_ms":0.9,
   "promoted":false,"deopts":0,"deopt_reasons":[],"refused":null,"native_refused":null,
   "fallbacks":{"count":400,"reason":"an argument the tier boundary cannot convert (a module passed as a value, say)"},
   "pinned":null,"violation":null}],
 "violations":[]}
```

Per function (keyed by `file` and `name`; `line` is where it is
declared, 1-based):

- `calls` — exact counts of the calls each tier served. A call the
  tree-walker hands to a compiled tier counts once, on that tier.
  Calls native code makes to native code push no frame; with
  statistics on, each such call site adds one to its callee's counter
  as it calls (a load, an add and a store baked into the machine code;
  racing threads may lose a count), so a function called only from
  inside native code still has its calls. Its time stays its caller's.
- `self_ms` — time each tier spent in the function itself, and in the
  builtins it called (`map`, `sort_by`, …: `builtin`); `total_ms` with
  its callees. Sampled, as `olang profile` samples, every 250 µs: a
  function that ran for less than a tick may show calls and no time.
- `tier` — the tier that served most of its calls.
- `deopts` and `deopt_reasons` — native attempts that declined: an
  argument of a kind native code does not take, a native compile
  refused, a guard that failed while it ran (the bytecode ran the call
  again), no specialisation for the arguments' kinds.
- `refused` (`reason`, `code`, `docs`) — the bytecode tier refused it,
  at compile time or because a result could not cross back.
- `fallbacks` — calls of a compiled function that ran on the
  tree-walker anyway, and why (code `boundary` above). This is the
  case `olang check --tier` cannot see: the function compiles, and its
  run stays interpreted.
- `native_refused` — why native code does not take a function that
  runs as bytecode.
- `pinned` (`comment`, `olang.toml`) and `violation` — a pinned
  function that fell back: more than `threshold` calls on the
  tree-walker, or every call on bytecode because native code would not
  take it. Each violation is also printed on stderr at exit
  (`tier guard: file:line `name` …`), and with `OLANG_TIER_GUARD=1` a
  run that otherwise succeeded exits with status 3 — a guard a local
  verify script can use without an editor.

**What it costs.** Calls are counted on the shadow-stack push each tier
already makes for `olang profile` (a load and a store on the thread's
own counters, no lock, no clock), and native code's calls of native code
at their call sites (a load, an add and a store, compiled in only when
statistics are on); time is sampled by a thread that wakes every
250 µs. Measured on an Apple M5 Pro (6P+12E, 24 GB), macOS 27.0 (best
of 7 whole runs, start-up included): a native-heavy run (`fib(32)` × 6, 85 ms, its
42 million calls of itself counted) +1–5%, a bytecode-heavy one (record
and string helpers, 10⁶ calls, 1.5 s) +3%, a tree-walker-heavy one (a
module passed as a value, 10⁶ calls, 1.3 s) +8%. Without the flag nothing changes: every push site is
one relaxed load and a predicted-false branch, and native code is
compiled without the counters.

**From the REPL.** `olang repl --serve` keeps the same statistics for
its evaluations and answers them to `{"op":"stats","id":9,"reset":true}`
— `{"ok":true,"stats":{…the object above…}}` — with `reset` starting
them again (the evaluation's own, when asked after each one).

## `--verify-tiers` — live tier verification

The differential suites prove that the execution tiers agree on the
programs the suites contain. `--verify-tiers <rate>` proves it on the
program actually running:

```bash
olang --verify-tiers 0.05 run pipeline.ol   # verify 5% of native results
olang --verify-tiers 1 run pipeline.ol      # verify every native result
```

At the given sampling probability, each native-tier result — a
compiled call, made from the VM or from the tree-walker (a test block's
call, a top-level one), or an on-stack-replacement loop region — is
re-executed on the bytecode VM with the same inputs, and the two
results are compared bit for bit. Re-execution is unobservable because the JIT
compiles only code that is pure with respect to caller-visible state;
the cost is the work re-run (rate `1` roughly doubles native work;
`0.01` is noise). A divergence prints both renderings and exits with
code 102: by the correctness policy, any such difference is an engine
bug, and the report asks for it to be filed. Under `olang test --format
json` it is also a `divergence` event naming the function, both values
and the block under way. The `:ovm` report in the
REPL shows how many calls a session verified.

The flag suits long-running or high-stakes runs where the standing
suites are not evidence enough: a sampled rate provides continuous
spot-checking for a few percent of overhead.

## Environment variables

Every variable olang reads, in one table. **Public** ones are part of
the contract ([stability](stability.md#tools-and-file-formats)): their
names and meanings hold across 1.x, and a command-line flag takes
precedence where both exist. **Internal** ones are development and
diagnostic aids: they may change or go away in any release, and
nothing should depend on them.

| Variable | Public | Effect |
|---|---|---|
| `OLANG_HOME` | yes | Relocate the whole `~/.olang` tree ([the home directory](packages.md#the-home-directory)). |
| `OLANG_SHELF` | yes | Location of the shelf manifest (default `~/.olang/shelf.toml`). |
| `OLANG_CACHE` | yes | Location of the git dependency cache (default `~/.olang/cache`). |
| `OLANG_REGISTRY` | yes | The registry registry-version dependencies resolve against ([packages.md](packages.md)). |
| `OLANG_LOCK_READONLY` | yes | `1`: resolve as usual but never write `olang.lock`. |
| `OLANG_DENY` | yes | Deny capabilities for the run, as `--deny`: a comma list of `fs`, `fs-write`, `net`, `proc`, `db`, `env`. |
| `OLANG_PARALLELISM` | yes | Thread count for `par_map`/`par for` and other parallel work (also `--ovm-parallelism N`). |
| `OLANG_HTTP_WORKERS` | yes | Default worker count for `http.serve` when the options map does not set one. |
| `OLANG_PLOT_THEME` | yes | `dark` or `light`: the theme a `plot` chart takes when its options name none (an editor's notebook kernel follows its appearance). |
| `OLANG_ODS_WORKERS` | yes | Worker count for the ods data stack's parallel operations. |
| `OLANG_STALL_ABORT` | yes | `0` disables the deadlock abort: an all-threads-parked program hangs instead of exiting with the stall report. |
| `OLANG_VERIFY_TIERS` | yes | The `--verify-tiers` sampling rate (`0`–`1`). |
| `OLANG_OVM_STATS` | yes | `json` or `json:PATH`: keep [tier statistics](#tier-statistics---ovm-statsjson) for any command and write them at exit (`json:DIR/`: a file a process in `DIR`). |
| `OLANG_TIER_GUARD` | yes | `1`: a run keeping tier statistics in which a pinned-native function fell back exits with status 3. |
| `OLANG_PROFILE_LIVE` | yes | A live-profile directory, as `--profile-live DIR`; reaches the children a run starts. |
| `OLANG_PROFILE_LIVE_EVERY` | yes | Milliseconds between live-profile snapshots (default 500). |
| `OLANG_UPDATE_SNAPSHOTS` | yes | `1`: `testing.snapshot` rewrites its snapshots instead of comparing (`UPDATE_SNAPSHOTS=1`, the generic spelling, too). |
| `OLANG_TEST` | yes | Set to `1` by `olang test` for the files it runs, for a program to read. |
| `OLANG_COMPILE_CACHE` | yes | `0` turns off the compile cache ([ovm.md](ovm.md)). |
| `OLANG_COMPILE_CACHE_MB` | yes | The compile cache's size cap in MB (default 64). |
| `OLANG_PARSE_CACHE` | no | `0`: parse every module instead of reading it back from `~/.olang/state/parsed`. |
| `OLANG_WARM` | yes | `0` disables the warm tier profile ([ovm.md](ovm.md)), for measurement. |
| `LOOM_RENDERER` | yes | `gpu` (the default) or `software`: the renderer `gui.open` uses when its options name none. |
| `NO_COLOR` / `CLICOLOR_FORCE` | yes | Disable or force `term` styling regardless of whether output is a TTY. |
| `OLANG_LOG_LEVEL`, `OLANG_LOG_TIMESTAMP`, `OLANG_LOG_MODULE` | no | The runtime's own log (`ERROR` … `TRACE`; default `WARN`). |
| `OLANG_TIER_STATS` | no | `1`: machine-readable tier counters on exit. |
| `OLANG_TIER_VERBOSE` | no | The bytecode tier says what it compiles and refuses. |
| `OLANG_BOOT_TRACE` | no | `1`, the same as `--trace`: where a program's start goes — a line on stderr at each landmark (the interpreter's thread started, the entry parsed, the program starting, the platform's loop, a window asked for and made, the first frame presented), stamped from the process's start, each module a `use` found and where, and at the first frame the modules read and loaded and the compiles, with their times. |
| `OLANG_LAZY_COMPILE` | no | `0`: compile every callee with its caller (the eager route) instead of at its first call, for A/B measurement. |
| `OLANG_JIT_DEBUG` | no | `1`: JIT compilation decisions: what compiled, what refused, and the instruction or register that refused it. |
| `OLANG_OSR_DEBUG`, `OLANG_OSR_OFF` | no | On-stack-replacement decisions; `1` disables on-stack replacement. |
| `OLANG_BRIDGE_TIER_OFF` | no | `1`: disable the bridge interpreter's bytecode tier. |
| `OLANG_COLLECTIONS_ON_INTERP` | no | Keep the auto-loaded `collections` on the tree-walker. |
| `OLANG_DUMP_FN` | no | `<name>`: print the compiled instruction stream for one function. |
| `OLANG_ODS_TIMING` | no | `1`: per-phase timings from the ods CSV reader and group-by. |
| `OLANG_FIX_DEBUG` | no | `<dir>`: when `olang fix` leaves a file because its rewrite would read back differently, write the two trees as read back (`before.json`, `after.json`) and the rewrite (`after.ol`) into `dir`. |
| `OLANG_KEYS_STATS` | no | The checker's `.key` read coverage, on stderr. |
| `OLANG_ALLOC_STATS`, `OLANG_ALLOC_TRACE` | no | Allocation counts (an `alloc-count` build). |
| `OLANG_DEBUG_*` | no | Engine tracing for one subsystem each (closures, modules, liveness, higher-order calls, auto-parallelism). |
| `OLANG_VERIFY_SELFTEST` | no | Make `--verify-tiers` see a divergence, to test what reports one. |
| `OLANG_WARM_DIR` | no | Where the warm tier profiles live (tests). |
| `GUI_TRACE` | no | The `gui` platform layer's event trace. |
| `OLANG`, `OLANG_WASM` | no | `otc bench` and `otc new --web`: the olang binary and the browser runtime to use instead of the ones found beside `otc`. |

The build reads `OLANG_BUILD_*` and `OLANG_RUNTIME_WASM*`, set by
olang's own `build.rs`; they are not read at run time.

## Writing custom tools with `meta`

Several of the commands above are built on the `meta` module, which returns a
parsed olang program as ordinary olang values. A project can use it to write
its own linters, code transformations, and code generators in olang rather
than as changes to the compiler; `olang check --rules` runs such tools
alongside the built-in checker. The `meta` module is documented in the
[standard library reference](stdlib.md#meta--the-program-as-data-the-open-ast).
A complete example is [`examples/language/metatool`](../examples/language/metatool/main.ol).

## The examples harness

`examples/run_all.ol`, a test harness written in olang, runs every example
program — standalone scripts and packages — in its own subprocess and reports
a pass/fail summary. It complements `olang test`: the harness checks that
whole programs run, and the test runner checks `test`-block assertions. See
[examples/README.md](../examples/README.md).
