# Packages and dependencies

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Language reference](language.md) · [Standard library reference](stdlib.md)

olang has a source-based package manager. A package is a directory of `.ol`
files with an `olang.toml` manifest. There is no build step and no compiled
artifact: dependencies are fetched as source and resolved by the same `use`
mechanism as local modules, so a dependency is used the same way as code
written in the same project.

Projects are managed by the companion tool `otc`. Everything that
touches a *file* lives in `olang` (run, repl, test, fmt, check, bench,
profile; see [Command-line tooling](tooling.md)); everything that touches
a *project* lives in `otc`. The workflow is local-first: your own
libraries, on your own machine, by name, with git and registry sources
for a dependency that lives elsewhere.

This chapter covers scaffolding a project, the manifest, dependencies
and the library shelf, the home directory and toolchains, the project
benchmark harness, the lockfile, capabilities, and the transparent
binary `olang build` writes.

## Table of contents

- [Starting a package](#starting-a-package)
- [The manifest](#the-manifest)
- [Commands](#commands)
- [The library shelf](#the-library-shelf)
  - [The starter libraries](#the-starter-libraries)
  - [Pinning and drift](#pinning-and-drift)
- [The home directory](#the-home-directory)
- [Updating and toolchains](#updating-and-toolchains)
- [Benchmarks](#benchmarks)
- [In the REPL](#in-the-repl)
- [The lockfile](#the-lockfile)
- [Version resolution (MVS)](#version-resolution-mvs)
- [The registry](#the-registry)
- [Trust model](#trust-model)
- [Capabilities](#capabilities)
- [The transparent binary](#the-transparent-binary)

## Starting a package

`otc new` scaffolds a project — an application by default, a library with
`--lib`, a full-stack web app with `--web`:

```bash
otc new myapp             # application: olang.toml + src/main.ol
otc new geometry --lib    # library: olang.toml + index.ol + lib/
otc new dashboard --web   # web app: JSON API + sqlite + wasm frontend
```

The `--web` shape is one process serving a SQLite-backed JSON API, the
page, and the frontend's own olang source, which the browser runs
against the DOM through the wasm runtime — built on [the web
SDK](web-sdk.md), so it is three files: `main.ol` (migrations, rpc
routes, `serve` with the server-rendered first paint), `lib/pages.ol`
(the view, rendered by the server into the shell and by the browser on
every change), and `client.ol` (`mount`, `action`, `call`). The
manifest depends on the shelf's `web` and `validate`; `otc install`
resolves them. `--web-bare` writes the same application on the raw
stdlib, with the router, static routes, escaping and schema written by
hand, for studying what the SDK provides. Its README covers the one
artifact the scaffold cannot write from source: the wasm runtime, which
the scaffold copies in when it can find one (`OLANG_WASM`, or beside
`otc`); the API works without it.

A name may be hyphenated (`otc new open-track-query --lib`): the
directory keeps it, and the package is imported by the identifier form
(`use open_track_query`), which the manifest records and the usage hint
prints. Outside a git repository the scaffold ends with a one-line
reminder to `git init`.

All shapes come with a README, a `.gitignore`, and a working `test`
block, and the generated source is parse-checked before it is written.
The difference is the entry point. An application's life is
`olang src/main.ol`; a library's public API lives in `index.ol`, over
implementation modules in `lib/`:

```text
geometry/
  olang.toml
  index.ol        the public API — what `use geometry` finds
  lib/area.ol     implementation
```

`index.ol` has to sit at the package root, because that is where the
resolver looks. Keeping it to imports and re-exports means the package's
whole surface reads in one screen:

```olang no-run
// geometry/index.ol
use lib.area { area }

share fn rectangle(w, h) = area(w, h)
share let ORIGIN = (0, 0)
```

```olang no-run
// geometry/lib/area.ol — `share` here makes a name visible to the rest
// of the package; only what index.ol re-exports is public.
share fn area(w, h) = w * h

fn helper(x) = x + 1        // private even within the package

test "area works" {
    assert_eq(area(3, 4), 12)
}
```

Nothing forces the split: a one-file library that `share`s directly from
`index.ol` is a valid package too.

That is all a consumable library is: an `olang.toml` (so the resolver can
find the package and name it) plus a root module with `share`d bindings.
`share` is the package boundary — there is no separate export manifest to
maintain.

## The manifest

```toml
[package]
name = "app"
version = "0.1.0"        # strict semver: three components
description = "..."      # optional, as are authors and license

[dependencies]
geometry = { path = "../geometry" }                        # local directory
httplib  = { git = "https://github.com/u/httplib", tag = "v1.2.0" }
json     = "^1.0"                                          # registry version
```

**The olang it is written for.** `olang = "1.0"` under `[package]` names
the oldest olang the package runs on, as Cargo's `rust-version` names the
oldest Rust: `"1"` and `"1.0"` are 1.0.0 or newer (every 1.x satisfies
them), `"1.2.1"` that release or newer. It is a version of one to three
numbers and nothing else; `olang check` reports any other value. An olang older than the pin still runs the
project, and says so on standard error first. `otc
install` resolves it to the newest installed toolchain that satisfies it
(see [Updating and toolchains](#updating-and-toolchains)) and records that
version in `olang.lock`; `otc toolchain which` prints the binary a
project's pin resolves to. A pin nothing installed satisfies is said, with
the command that installs one (`otc toolchain install 1.0.0`), which is
never run for you. An editor runs the project's children with the pinned
toolchain when it is installed.

A path dependency's own path dependencies are followed, at any depth,
each resolved against the manifest that names it: an application that
names `loom = { path = "../loom" }` gets the `heddle` Loom names in its
own manifest without naming it too. A name the application (or a nearer
package) already maps keeps that mapping.

Git dependencies take one of `tag`, `rev`, `branch` — or none, meaning the
default branch's head. Registry requirements accept the usual semver
operator syntax (`^1.0`, `>=1.2, <2.0`, an exact `1.4.0`).

Then use a dependency like any module — a `use` whose first segment is a
dependency name resolves inside that package:

```olang no-run
use geometry { area, ORIGIN }
println(to_string(area(5, 5)))
```

### Other sections

| Section | Holds | Described in |
|---|---|---|
| `[package] lock` | `"auto"` or `"never"`: whether incidental runs may write `olang.lock` | [The lockfile](#the-lockfile) |
| `[package] assets` | files and folders a built application carries | [`olang build`](tooling.md#olang-build) |
| `[capabilities]` | what the package's code may touch, and each dependency's attenuation | [Capabilities](#capabilities) |
| `[check] rules` | rules files (`rules = ["tools/rules.ol"]`, relative to the project root) that `olang check` runs with no `--rules` | [Project rules](tooling.md#project-rules) |
| `[check] promote` | advisory classes `olang check` treats as errors | [Promoting advisories](tooling.md#promoting-advisories) |
| `[check] native` | functions pinned to native code | [What the tier refuses](tooling.md#what-the-tier-refuses) |

```toml
[check]
rules = ["tools/rules.ol"]
promote = ["exhaustiveness", "shape"]
```

`olang check` and `olang doctor` warn about a key or section they do
not read (a near miss such as `[dependancies]`), and leave other tools'
sections alone.

### How `use` finds a package

Dependency names win over local files, so `use geometry` always means the
package, never a sibling `geometry.ol`. Within the dependency's
directory, a bare `use geometry` resolves the root module — the first of
`index.ol`, `mod.ol`, `geometry.ol`, `src/index.ol` that exists — and a
dotted path like `use geometry.shapes.circle` resolves below it
(`shapes/circle.ol`, or `shapes/circle/index.ol`, `shapes/circle/mod.ol`,
`src/shapes/circle.ol`). A package can also `use` itself by name, which
is what lets its own tests import its public surface.

## Commands

The project commands:

```bash
otc new NAME [--lib|--web]   # scaffold a project
otc add ../my-lib            # add a dependency by path
otc add my-lib               # add a dependency by name, from your shelf
otc remove my-lib            # drop a dependency
otc list                     # what this project depends on, and where each lives
otc list --format json       # the same as data: #{ format, kind: "deps", package, version, root, dependencies }
otc install                  # fetch dependencies, honoring olang.lock
otc install --frozen         # fail if resolution would rewrite the lock (CI)
otc install --update         # re-resolve everything, rewrite the lock
otc lib add|list|remove|restore   # your library shelf (next section)
otc bench                    # the project benchmark harness (below)
```

Every command finds the project root by walking up to the nearest
`olang.toml`, and `otc add` in a directory with none **creates one**:
there is no separate init step. `add` takes one argument with two
readings: anything with a path separator (or a leading `.`) is a
directory, recorded relative to the project root no matter where the
command ran; a bare name is looked up on your shelf. Either way the
dependency is validated at `add` time and resolution runs immediately,
so the lockfile is never left behind the manifest, and changing an
existing dependency's source requires `--force`.

Dependencies that live elsewhere still work — write them into
`olang.toml` directly (`{ git = "URL", tag = "v1.0" }`, or a registry
requirement with `OLANG_REGISTRY` set) and `otc install` resolves them
with the same lockfile discipline.

You rarely need `otc install` day to day: **running a file inside a
package resolves dependencies automatically**. `olang main.ol` reads
`olang.toml`, installs (replaying the lockfile — see below), and runs.

## The library shelf

The shelf is a per-user catalog of libraries kept by name, so a project
can depend on a local library without recording where it lives. It needs
no server, account or network: the shelf is one small TOML file, and the
libraries are ordinary directories of olang source. A shelvable library
is a directory with an `index.ol` (the entry point `use` resolves) or an
`olang.toml` naming it; `otc new <name> --lib` scaffolds that shape.

Register a library once, per user, from anywhere:

```bash
otc lib add ~/code/geometry            # registers under its package name
otc lib add . --name geo               # the current directory, under a chosen name
otc lib add ~/code/sdk --rev 4f2a91c   # the library as it stands at that commit
otc lib list                           # every shelved library, and where it points
otc lib list --format json             # #{ format, kind: "shelf", shelf, libraries }, each #{ name, dir, rev, starter, status }
otc lib remove geometry                # take it off the shelf
otc lib restore textkit                # re-shelve a starter (or refresh it)
otc lib restore                        # re-shelve every starter
```

Then, from any project:

```bash
otc add geometry       # the manifest gains geometry = { shelf = "geometry" }
```

The manifest records only the name, so `olang.toml` stays free of
machine-specific paths and means the same thing on every machine that
has a `geometry` on its shelf. The lockfile pins what the name resolved
to — the directory and a checksum of its contents — so a library that
moves or disappears is noticed at the next install (see
[Pinning and drift](#pinning-and-drift)).

`otc lib list` marks each entry: `(starter)` for a shipped library,
`pinned at <sha>` for one shelved at a commit, and `(missing — directory
is gone; re-register, remove, or restore)` when a registered directory
has disappeared.

Removing a user library forgets the registration only: the source
directory is untouched, and projects that depend on it keep their
lockfile pin until their next install. Removing a starter also deletes
its materialized directory, since the shelf owns that copy; `otc lib
restore <name>` writes it back. Restoring a starter that is present
overwrites it with the running toolchain's copy, which is how starters
pick up a newer toolchain's changes. `otc install` says when a shelved
starter is behind the toolchain's copy and names the `otc lib restore`
that refreshes it.

| Path or variable | Holds |
|---|---|
| `~/.olang/shelf.toml` | the shelf: name → directory. Its first line, `version = 1`, is the format version; every installed toolchain shares the file, and one that finds a newer version refuses it by name rather than rewriting it. |
| `~/.olang/shelf/<name>/` | the materialized starter libraries |
| `~/.olang/shelf/pins/<name>-<sha>/` | libraries shelved at a commit |
| `OLANG_SHELF` | the shelf file's location (starters materialize into `shelf/` beside it) |
| `OLANG_HOME` | the whole `~/.olang` root, the shelf included ([the home directory](#the-home-directory)) |

A shelf is seeded with the starter libraries once, when its file is
first created. Starters removed on purpose are not reseeded.

### The starter libraries

Three libraries ship with every shelf, so a new installation can `otc
add textkit` at once. Each is pure olang, with no capabilities and no
effects, documented with `///` comments and tested by its own `test`
blocks. Their sources are compiled into the binary, so seeding works
offline, and the materialized copies are versioned with the toolchain
that wrote them.

#### textkit — text layout and humane formatting

| Function | Does |
|---|---|
| `rpad(s, w)` / `lpad(s, w)` | pad right/left with spaces to width `w`; longer strings pass through |
| `center(s, w)` | center in width `w`, extra space to the right |
| `rule(w)` | a horizontal rule of `w` box-drawing dashes |
| `wrap(text, width)` | greedy word wrap, returned as a list of lines |
| `columns(rows, gap)` | align rows of cells into columns, `gap` spaces apart |
| `table(headers, rows)` | a finished table: headers, rule, aligned body |
| `money(cents)` | integer cents as currency: exact, comma-grouped, negative-safe |
| `human_bytes(n)` | a byte count at a readable scale: `"1.5 KB"` |
| `human_duration(ms)` | milliseconds as people say it: `"3.2s"`, `"5m 07s"` |
| `slugify(s)` | a URL- and filename-safe slug: `"hello-world"` |

```olang no-run
use textkit { money, table, human_duration }

println(money(123456789))     // $1,234,567.89
println(money(-150))          // -$1.50
println(table(["op", "time"],
    [["parse", human_duration(3200)], ["run", human_duration(412)]]))
// op     time
// ────────────
// parse  3.2s
// run    412ms
```

Amounts kept in integer cents stay exact; `money` is the display edge.

#### validate — declarative checks for map-shaped input

Data from a form, a JSON body, a CSV row or a config file is a map of
unknowns. `validate.check` holds it against a rule list and reports
every problem, each naming its field.

| Function | Does |
|---|---|
| `check(m, rules)` | `Ok(m)`, or `Err(problems)`: one `"field: problem"` string per failure |
| `ok(m, rules)` | the verdict alone, as a Bool |

A rule is `[field, kind]` or `[field, kind, opts]`. Kinds: `"str"`,
`"int"`, `"float"`, `"num"`, `"bool"`, `"list"`, `"map"`, `"any"`, and
`"date"` (an ISO day the value must parse as). Options: `required: false`
(absent is fine; present still checks), `min`/`max` (numeric bounds,
length bounds for strings and lists, ISO days for a date), `one_of` (an
allowed-values list), `pattern` (a regex the whole string must match),
and `where` (a function of the value answering a `Result`, whose `Err`
message becomes the field's problem). The input is a map or a parsed
JSON object.

```olang no-run
use validate { check, ok }

let rules = [
    ["date",   "str", #{ "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$" }],
    ["amount", "int", #{ "min": 1 }],
    ["kind",   "str", #{ "one_of": ["expense", "income"] }],
    ["note",   "str", #{ "required": false, "max": 200 }],
]

match check(#{ "date": "2026-8-25", "amount": 0, "kind": "loan" }, rules) {
    Ok(row) => println("valid"),
    Err(problems) => for p in problems { println(p) }
}
// date: does not match ^[0-9]{4}-[0-9]{2}-[0-9]{2}$
// amount: below minimum 1
// kind: must be one of ["expense", "income"]

let even = [["n", "int", #{ "where": (v) => if v % 2 == 0 => Ok(v) else => Err("must be even") }]]
println(show(check(#{ "n": 3 }, even)))   // Err(["n: must be even"])
```

#### markdown — a Markdown renderer

`markdown.to_html(text)` turns a Markdown document into HTML: headings,
paragraphs, `` `code` `` spans and fenced blocks, **strong**,
*emphasis*, links, bulleted and numbered lists, block quotes and
horizontal rules. Inline markers nest; an unmatched marker is literal
text; everything else is escaped, so untrusted input renders as text
rather than markup. GitHub-style task lists render as disabled
checkboxes (`<li class="task">`, `class="task done"` for `[x]`), in
source order, so a click in the page maps back to the n-th task line.

| Function | Does |
|---|---|
| `to_html(md)` | a whole Markdown document, as HTML |
| `render_inline(s)` | one line's inline spans, for a caller with its own block structure |
| `escape(s)` | HTML-escape text: `&`, `<`, `>` become entities |

```olang no-run
use markdown { to_html }

println(to_html("# Notes\n\nShip the **shelf** first."))
// <h1>Notes</h1>
// <p>Ship the <strong>shelf</strong> first.</p>
```

#### Documentation in the REPL

Shelved libraries are olang source, so their `///` comments answer
`:help` in the REPL once a library is imported, and `olang doc` renders
them as an API page:

```text
olang> use textkit { money }
olang> :help money

═══ money ═══

  share fn money(cents)

  Integer cents as currency, negative-safe and comma-grouped:
  `money(123456789)` is "$1,234,567.89", `money(-150)` is "-$1.50".
  Keeping amounts in cents end to end is what makes them exact.
```

### Pinning and drift

`otc add` and `otc install` write `olang.lock` with the directory the
shelf resolved and a checksum of its contents. From there:

- **A moved or deleted library** fails the next install, naming the
  library and the path that stopped existing.
- **An edited library** is normal development: the change is reported
  at install, never treated as tampering, and the lock re-pins.
- **`otc install --frozen`** replays the lockfile exactly and fails on
  any drift.
- **A library shelved at a commit** does not follow its checkout. `otc
  lib add <path> --rev <commit>` writes the repository's tree at that
  commit into the shelf's own directory (`shelf/pins/<name>-<sha>`),
  registers the name there, and records the full commit id and the
  repository it came from. The lock of every project that depends on
  the name then carries `rev`, and the lock is the source of truth: a
  run, `olang test`, `olang check`, the language server and the REPL use
  that commit's snapshot whatever commit the shelf holds the name at
  now. The shelf keeps every snapshot it made under `pins/`; one that is
  gone is made again from the repository at that commit. The snapshot's
  checksum is verified as a fetched dependency's is, and a mismatch is
  refused. A commit this machine cannot produce fails, naming the
  library, the locked commit, the shelf's commit and the fix:

  ```text
  the lock pins shelf library 'sdk' at 4f2a91c03e1b, which this machine
  does not hold (…/pins/sdk-4f2a91c03e1b is not there, and the shelf
  knows no repository to make it from); the shelf has it at 9b03e7d5a210.
  Run `otc install` to move the lock to the shelf's pin, or shelve that
  commit: `otc lib add <path> --rev 4f2a91c03e1b`
  ```

  Re-registering the name without `--rev` unpins it. Two machines
  resolve the same library this way from checkouts each keeps moving.
- **A dependency's own lock pins its shelf libraries.** A library that
  depends on another by shelf name, and ships an `olang.lock` pinning
  it, gets the commit its lock names in every project that uses it,
  resolved and verified the same way. The project's lock pins the
  library, and the library's lock pins the rest.
- **Re-shelving is picked up by `otc install`.** After `otc lib add
  <path> --rev <new-sha>`, or `otc lib add <path>` to follow the
  directory again, the next `otc install` reports the move (`shelf
  library 'sdk' moved: the lock had it at 4f2a91c…, the shelf has it at
  9b03e7d… — re-pinning`) and re-resolves that entry; `--frozen` refuses
  the drift. Until then the project runs the commit its lock pins, and
  `otc doctor`, run in the project, says so as a note, not a problem
  (`note: olang.lock has shelf library 'sdk' at 4f2a91c03e1b, the shelf
  has it at 9b03e7d5a210; its snapshot is here and is what runs`). A
  library that follows its directory has no directory in the lock, so
  re-registering it at another path needs no re-pin.

A lockfile pin on a starter behaves like any other: after `otc lib
restore`, the next `otc install` in each project re-pins it, visibly.

## The home directory

Everything the toolchain writes under the user's home lives in
`~/.olang`, whose layout is part of the stability promise:

```text
~/.olang/
  bin/          shims pointing at the default toolchain
  toolchains/   <version>/bin/{olang, otc} — managed by `otc update`
  shelf/        starter libraries and libraries shelved at a commit
  shelf.toml    the shelf manifest
  cache/        git dependency clones (prunable)
  state/        warm hints, REPL history (regenerable)
```

`OLANG_HOME` relocates the whole tree (tests use this). Three commands
manage it:

- **`otc doctor`** audits an installation: every `olang`/`otc` on
  PATH with its version and origin (cargo, Homebrew, olang-home) and
  which copy wins; version agreement between `olang` and `otc`; files
  and directories the layout does not hold; shelf entries whose paths do
  not resolve; a project lock (the one the working directory is in)
  behind the shelf, as a note; and the sizes of the prunable parts. The default
  run is read-only; `otc doctor --fix` applies the safe repairs (stray
  files removed, the REPL history moved into `state/`) and never touches
  installations owned by other package managers.
- **`otc clean`** reclaims the prunable parts: the dependency cache by
  default, `--state` for regenerable runtime state, `--all` for both.
  Sizes are reported as they are reclaimed. The shelf and the
  toolchains are never clean's business.
- **`otc update`** installs the latest release. See the next section.

## Updating and toolchains

`otc update` downloads the latest release for the platform, verifies
it against the release's `SHA256SUMS`, installs it under
`~/.olang/toolchains/<version>/`, and atomically repoints the shims in
`~/.olang/bin` — the running binaries are never overwritten in place.
`otc update --check` reports the current and latest versions without
installing anything. Nothing contacts the network except these two
invocations, explicitly. Release builds target macOS, the supported
platform; Linux builds are an unsupported preview
([Stability](stability.md#platforms)).

Multiple releases install side by side:

```bash
otc toolchain list              # installed versions, default marked
otc toolchain list --format json  # #{ format, kind: "toolchains", dir, default, toolchains }
otc toolchain install 1.0.0     # add a specific release
otc toolchain default 1.0.0     # switch the shims
otc toolchain remove 1.0.0      # remove (the default refuses)
otc toolchain which             # the olang this project's pin resolves to
otc toolchain link 1.1.0 ../olang    # a local build as a toolchain (a binary, or a checkout's target/release/olang)
```

A toolchain is a folder `~/.olang/toolchains/<version>/` holding
`bin/olang`; its version is the folder's name. A project's `[package]
olang` pin picks among them (the lock's exact version first, while the
manifest still allows it; else the newest that satisfies the
requirement; else the olang running when it satisfies it).

Installations from Homebrew or cargo are independent of this
machinery; `otc doctor` shows how the copies on PATH shadow one
another, and `olang --version --verbose` reports which binary answered
and from where.

Shell completion scripts for both tools come from the tools
themselves: `otc completions zsh` and `olang completions zsh` (also
`bash`, `fish`, `elvish`) print a script to install in the shell's
completion directory.

## Benchmarks

`otc bench` runs the project's benches — ordinary olang programs in
`bench/`, each a fresh subprocess — and adds what a stopwatch cannot:

```bash
otc bench                        # every bench in bench/
otc bench sums --runs 7          # filter by name, more repetitions (default 5)
otc bench --sizes 1000,4000      # override every bench's sizes
otc bench --save base.json       # store medians as a baseline
otc bench --against base.json --fail-on-regress   # the CI gate
otc bench --profile              # rerun the slowest point under `olang profile`
```

A bench declares a **scaling curve** with a comment directive, and the
harness runs it once per size (the size arrives as `os.args()[1]`),
fits the growth on the medians, and names it:

```olang no-run
// bench: sizes = 200000, 800000, 3200000
let n = unwrap(str.parse_int(os.args()[1]))
fn total(n) = {
    let mut acc = 0
    let mut i = 0
    while i < n { acc = (acc + i * 3) % 1000003 i = i + 1 }
    acc
}
let t0 = time.monotonic_ms()
let out = total(n)
println(`TIME ${time.monotonic_ms() - t0}`)
println(`CHECKSUM ${out}`)
```

```text
  sums
    n=200000         1.0 ms  (cv  0.0%, rss    13 MB)  checksum 520003
    n=800000         2.0 ms  (cv  0.0%, rss    13 MB)  checksum 920015
    n=3200000        8.0 ms  (cv  0.7%, rss    13 MB)  checksum 120153
    growth: ~O(n) (exponent 1.00)
```

An accidentally quadratic bench announces itself:
`⚠ growth: ~O(n²) — check for a copy-per-iteration (exponent 1.99)`.

The two output lines are part of the harness's contract. A `CHECKSUM`
line must agree across every repetition or the measurement is refused:
a timing whose answer wobbles is measuring something else. A `TIME`
line (milliseconds) makes the bench self-timed, excluding interpreter
startup and setup; without one the harness uses wall clock and
subtracts a measured startup baseline from the growth fit. Each point
reports the median, the coefficient of variation, and peak memory (the
child's actual RSS). Baselines record a machine fingerprint, and
comparing against another machine's baseline warns instead of
pretending; a point only counts as changed when it moves more than
max(5%, 2×CV) — beneath that is noise, not news.

`// bench: setup = gen_data.ol` names a program to run once before
timing (dataset generation), receiving the largest size as its
argument.

## In the REPL

Start `olang` from a directory inside a package (one containing an
`olang.toml`, or any subdirectory of it) and the REPL resolves its
dependencies automatically:

```text
$ cd examples/language/packages/demo
$ olang
Package 'demo' loaded — its dependencies are available via `use`

olang> use geometry { circle, area }
olang> area(circle(2.0))
12.5663706
```

A `use` still binds the names — the package context just makes it
resolvable. `:cd` into another package re-resolves; `:pkg` re-resolves the
current one (after editing `olang.toml`) or reports that there is no
package here.

To use a package **without cd-ing into it**, load it by path:

```text
olang> :pkg load ../geometry          # or an absolute path
Loaded package 'geometry' — use it with `use geometry`
olang> use geometry { * }             # import everything it shares
olang> area(circle(2.0))
12.5663706
```

`:pkg load` is additive — load several packages by path and they are all
available. (Running a *file* needs no such step: `olang path/to/main.ol`
resolves that file's package from anywhere, regardless of your directory.)

An import lists what it binds: `use geometry { circle, area }` brings in
only those two names; `use geometry { * }` (or bare `use geometry`) imports
everything the package `share`s, and `use geometry {}` only the
namespace. A wildcard never replaces a name the program already binds
nor a stdlib module's name — those exports stay reachable as
`geometry.name` — and a listed name the program already binds is an
error; [the language guide](language.md#importing-with-use) has the
rules.

After importing, the module name is a namespace you can inspect and call:
`geometry.area(...)`, or `:help geometry` in the REPL to list its functions.
This works for the embedded `colx` collections module too (`:help colx`).

## The lockfile

`olang.lock` records, for every package in the dependency graph, exactly
which source produced it: the path for a path dependency (relative to
the manifest, a dependency's own path dependency too); the shelf name for a shelf dependency, so another
machine resolves it through its own shelf rather than a path that only
existed here; the git URL, the resolved commit, and the ref the
manifest asked for; the resolved version for a registry dependency —
plus a checksum of the source tree and the package's own direct
dependencies — and `olang`, the toolchain version a `[package] olang`
pin resolved to. An install keeps a lock's `olang` when the manifest names
none, and re-resolves when the manifest's pin no longer allows it.
Commit it. A lock that names a path this machine does not
have is not an error: `otc install` says which entry it is re-resolving
and rewrites the lock.

The lock's first line is its format version (`version = 1`). A lock
whose version is newer than this olang reads was written by a newer
toolchain and may hold what this one cannot represent, so it is refused
by name — `olang.lock is format version 2, written by a newer olang` —
and left untouched, rather than replayed in part and rewritten without
what it held. Upgrade olang (`otc update`), or delete the lock to
resolve afresh.

One dependency that cannot be resolved does not stop the others: a
program's modules that never import the missing package run, and the
`use` that needs it is told which package is missing and why
(`dependency 'web' is declared in olang.toml but could not be resolved:
… not on your shelf`).

The mental model has one moving part: **a lockfile either *covers* the
manifest or it doesn't**, and every install starts by asking which.

- **Covered → replay.** `otc install` (and every implicit install —
  `olang main.ol`, the REPL) fetches exactly what the lock pins and
  leaves the file byte-for-byte untouched. No resolution runs, nothing
  is consulted over the network that isn't already needed for the
  pinned sources, and with a warm cache the whole operation is offline.
  Repeated runs are reproducible by construction.
- **Not covered → re-resolve.** Resolution runs against the manifest,
  the graph is fetched, and `olang.lock` is rewritten. This is the
  *only* path that writes the lock.

"Covers" is checked dependency by dependency:

- a **path** dependency is covered when the lock has a path entry with
  the same path;
- a **git** dependency is covered when the URL matches *and* the lock's
  recorded ref matches what the manifest now requests — so retagging or
  repointing a git dependency in the manifest re-resolves it, but a
  branch dependency stays on its pinned commit until you ask to move
  (`otc install --update` is that ask);
- a **registry** dependency is covered when the locked version satisfies
  the manifest's requirement — so loosening a requirement changes
  nothing, and tightening it past the locked version re-resolves;
- a lock entry for a package the manifest no longer needs (and that no
  other locked package depends on) breaks coverage, so removals clean
  the lock up rather than leaving fossils.

`otc install --update` skips the coverage question entirely: it always
re-resolves and rewrites the lock — the explicit "move everything
forward" action. `--frozen` guards the other direction in CI: if
resolution runs and would change an existing lock, the command fails
with `lockfile is out of date` instead of writing. (With no lockfile at
all, `--frozen` writes the first one — pair it with a committed lock.)
`OLANG_LOCK_READONLY=1` resolves as usual but never writes the lock: an
editor sets it for the children it runs on its own (a language server,
a REPL, `olang check`), so opening a project without a lockfile leaves
the project as it was.

A project can say the same of itself in its manifest — for a test
fixture, whose committed lock (or deliberate lack of one) must not drift
when someone runs `olang test` or `olang main.ol` in it by hand:

```toml
[package]
name = "fixture"
version = "0.1.0"
lock = "never"   # "auto" (the default) or "never"
```

With `lock = "never"` every *incidental* install — a run, `olang test`,
the REPL, the language server, `olang check --tier` — treats the project
as `OLANG_LOCK_READONLY=1` does: a lock that covers the manifest is
replayed, a stale one is resolved past in memory and left byte-for-byte
as it was, and a project without one gets none. The *explicit* commands
`otc install`, `otc add` and `otc remove` still write the lock, and say
that they did so on purpose — the flag is about writes nobody asked for,
not about forbidding the lock. `olang check` warns about any other
value, which reads as `"auto"`.

Fetched sources are cached content-addressed by commit under
`~/.olang/cache` (override with `OLANG_CACHE`), so any given revision is
downloaded once and shared across every project on the machine. Path
dependencies are used in place, never copied.

## Version resolution (MVS)

Registry dependencies resolve by **Minimal Version Selection**: for each
package, olang picks the *lowest* version satisfying every requirement across
the whole graph — the maximum of all the minimum-version floors. This is the
Go-modules algorithm. It is reproducible by construction (no "latest
compatible" drift), needs no backtracking solver, and makes upgrades
explicit: to move to a newer version, raise the requirement in `olang.toml`.

An unsatisfiable requirement is a clear error naming the package and the
constraint; there is no silent duplication of incompatible versions.

## The registry

There is no public registry, and `otc` has no publish command; the
local workflow (paths and the shelf) and git sources cover distribution.
Registry resolution works against any index you point it at.

A registry is a git repository of TOML index files: one `<name>.toml`
per package, listing each published version with its git source, commit
and checksum. An entry records the release's *registry* dependencies so
MVS can resolve through it, so a library meant for a registry depends on
registry versions itself, not on paths or git branches.

Point installs at a registry with the `OLANG_REGISTRY` environment
variable (a local directory or a checkout of the index repository).

## Trust model

The first fetch of a git or registry dependency is trust-on-first-use.
From then on the checksums bite: the lockfile records a sha256 of each
fetched source tree, and **every later install re-verifies fetched
(git/registry) content against it** — a mismatch is a hard error, not a
warning. A registry release's published checksum is likewise enforced at
fetch time, and an already-published version in an index is never
rewritten. Path and shelf
dependencies report drift informationally — editing one is normal
development. Pin
git dependencies by `rev` (not just `tag`) when you need the source to be
immutable, since a tag can be moved and a rev cannot.

## Capabilities

A package declares what it is allowed to touch. The `[capabilities]`
block in `olang.toml` gates the effectful stdlib surface — `fs`,
`http` (as `net`), `db`, `proc`, and the environment functions of
`os` — at the module boundary; pure computation is never gated.

`proc` means "may affect processes", including this one: it covers the
`proc` module, `os.exec`, and `os.exit`, so a dependency refused `proc`
can neither start a child nor end the host.

```toml
[capabilities]
fs = "read"        # false | "read" | true
net = false
proc = false
db = true
env = true
```

Absent, capabilities are wide open: a program with no manifest, or a
manifest with no `[capabilities]`, runs unrestricted. A denied call is a runtime
error naming the capability, the call, and the grant that refused it.

A grant that cannot be honoured is refused, never ignored. A key
`[capabilities]` does not know (a misspelling, or a capability a later
olang added), a value the gate does not take, or an `olang.toml` that
does not parse while it declares `[capabilities]`, stops the program
before it runs, and `olang check` fails on it. The same holds for the
manifest a built binary carries.

**Per-dependency attenuation** grants a dependency fewer capabilities than
the application, never more:

```toml
[capabilities.dependencies.leftpad]
fs = false         # this dependency cannot touch the filesystem,
net = false        # even if a future version tries to
```

The grant governs every way the package's code runs: `olang main.ol`,
`olang test` (a test runs under the capabilities the program runs under,
`OLANG_DENY` on top), and a binary `olang build --app` makes, whose
carried manifest keeps each dependency's attenuation with its sources
gone. The REPL and the language server run code with full capability:
they are the author's own tools, not the program.

File access that happens *through* another module is confined under `fs`
too, so `db`, `net`, and `ods` are not latent filesystem capabilities:
opening a file-backed database (`db.open` on a path, or a `db` query that
runs `ATTACH`) requires `fs`, and an `http.serve` handler that returns a
`body_file` is refused unless the program may read files. An in-memory
database (`:memory:`) needs no `fs`.

The data stack is gated the same way and at the level it actually uses.
`ods.read_csv_file`, `ods.read_jsonl_file`, `ods.open_csv`, and
`ods.open_jsonl` require `fs` at read level; `ods.write_csv` and
`ods.write_jsonl` require it at write level. Every other function in
`ods`, `stats`, and `plot` — including the parsers that take text,
`ods.read_csv` and `ods.read_jsonl` — reaches nothing and needs no grant.

### Asking what you were granted

A denial stops the program: it is not a `Result` to recover from. A
program that can run with less access checks its grant before making the
call. The `caps` module reports it:

```olang
let plan = if caps.allowed("fs") => "cache to disk" else => "in memory"
println(caps.level("fs"))        // the grant's word: false, "read" or true
let all = caps.granted()         // the whole grant, as a map
println(show(all.net))
```

`caps.allowed` answers for the **caller**, not the application: code in an
attenuated dependency sees what *that dependency* was given, which is the
same set the gate would enforce a moment later.

None of the three is itself gated, so a program can ask under the
tightest restriction.

Enforcement is by *attribution*: when code that lives in `leftpad`'s
directory calls a gated builtin, `leftpad`'s grant applies — the
intersection of the app's capabilities and the attenuation. A
supply-chain compromise that adds `fs` or `net` behavior to a dependency
that was never granted those capabilities is stopped at the boundary rather
than taking effect. A named attenuation for a package that is not actually a
dependency is
an error, so a typo can never silently grant nothing to the wrong
name.

For a runnable demonstration — the same app run twice against a malicious
dependency, blocked in one variant and not the other — see
[`examples/language/capabilities`](../examples/language/capabilities/).

`--deny` restricts any run further, on top of any manifest, from the
command line (also read from `OLANG_DENY`):

```bash
olang --deny net,fs-write program.ol   # remove network + write access
```

To determine the minimal manifest for a program, run it with
`--trace-caps`. This flag reports the capabilities the program used and
prints a corresponding least-privilege `[capabilities]` block:

```bash
olang --trace-caps program.ol
# ── capability profile (--trace-caps) ──
# exercised: fs (read), net
#
# suggested least-privilege manifest:
#
#   [capabilities]
#   fs = "read"
#   net = true
#   db = false
#   proc = false
#   env = false
```

Capabilities the program did not use are set to their most restrictive
value, so applying the generated block can only reduce access. The trace
sees every effect on every tier, so a compiled function's demands appear
beside an interpreted one's.

Add `--write` to fold the block directly into the package's `olang.toml`:

```bash
olang --trace-caps --write program.ol
```

`--write` never overwrites an existing `[capabilities]` block, so a
hand-tuned grant is left alone; if the program is not in a package it
prints the block instead. To read the grant a package *declares* (the
static counterpart to the observed profile), use `olang caps`:

```bash
olang caps                 # the current package's grant, and each dependency's
olang caps path/to/pkg     # or a specific package directory
olang caps ./tool          # a built binary (defers to inspect --caps)
```

Capabilities are enforced on every tier, at the one dispatch point
every builtin passes through. A compiled function carries the file it was
declared in, the bytecode VM keeps a stack of those files as it
executes, and the gate reads the innermost one, the same attribution the
interpreter takes from its own call stack. A compiled function in an
attenuated dependency is therefore judged by that dependency's grant,
and this holds across threads: a dependency's code reached through
`spawn`, `par_map`, `par for`, or an `http.serve` handler runs on a
worker whose tiers carry the same gate. A capability-restricted run
keeps every tier and its full speed.

### Modules loaded at run time

A host can load code it did not write into its own running program — a
plugin — and hold that code to a grant of its own:

```olang no-run
let grant = #{ "name": "tier-map", "fs": "read", "fs_roots": ["/work/project"], "proc": ["olang"] }
match runtime.load_module("/work/plugins/tier-map/plugin.ol", grant) {
    Ok(m) => {
        let r = runtime.call_budget(m.on, [state, event], 250)
        if r.ok => r.value else => println(r.error)
    },
    Err(e) => println("did not load: " + e)
}
```

- **The grant is attributed by folder**, as a dependency's attenuation
  is: everything defined under the loaded file's folder — its functions,
  the lambdas they make, the files it imports from there — answers to it;
  the host's own functions (the API it hands the module) answer to the
  host's. A key left out of the grant is denied. Within an app that has
  a manifest, a module gets no more than the app.
- **Finer than a dependency's.** `fs_roots` holds the filesystem grant
  under those folders only (a path's `..` resolved, its real path
  compared); `proc` may be a list of the programs it may run, matched by
  file name or whole path.
- **Never the host's.** `gui`, `tty`, loading and unloading modules and
  the process's own profiler are refused a loaded module (capability
  `host`): its code cannot draw, take the terminal, or grant itself more.
- **Afresh, and safely replaced.** A load forgets the modules cached from
  that folder first, so an edited file is read again; a load that fails
  keeps the grant before it, and the module value the host already holds
  still runs. `runtime.module_grant` says the grant; `runtime.module_permits(path, builtin,
  args)` asks the same gate before the host performs an effect on the
  module's behalf (a process it asked for).
- **Unloaded is refused, not the host's.** `runtime.unload_module`
  leaves a tombstone for the folder: code defined under it that still
  runs — a closure the host kept, the module's functions, a thread it
  started, a `par_map` over its closure — is refused every capability
  ("module <dir> was unloaded"), on every tier, rather than running with
  the host's authority. Its pure computation still answers. A later
  `load_module` of the folder replaces the tombstone with the new grant.
- **A time budget per call.** `runtime.call_budget(f, args, ms)` stops
  the call past `ms` — an interrupt aimed at the calling thread only, so
  the program's other threads run on — catches its failure, and answers
  `#{ ok, value | error, frames, over, us }`. Native code compiled after
  the first budget polls for it too. A builtin that blocks (a long
  `os.exec`, a sleep) is not interrupted until it returns, and a thread
  the module starts is not under the budget.
- **Where a failure happened.** A caught error's frames — each `#{ file,
  line, fn }`, innermost first — come with `call_budget`'s answer, and
  `runtime.last_error()` keeps the last one any boundary on the thread
  caught (`attempt` too). `runtime.shape(value)` describes a value's
  structure, for a host deciding whether a reloaded module's state still
  fits it.

## The transparent binary

An executable written by [`olang build`](tooling.md#olang-build) embeds
its complete source, its `olang.toml` and `olang.lock`, its capability
manifest, and a checksum over all of them. `olang inspect` reads and
verifies these files without any other context:

```bash
olang inspect ./tool             # a summary: version, checksum, capabilities
olang inspect ./tool --source    # the embedded source
olang inspect ./tool --manifest  # the embedded olang.toml
olang inspect ./tool --lockfile  # the embedded olang.lock
olang inspect ./tool --caps      # the resolved capability grant
olang inspect ./tool --format json  # the summary as data, every check's verdict
olang inspect ./tool --verify    # verify the checksum
olang inspect ./tool --against . # compare the binary to a source tree
olang inspect ./tool -o dir/     # extract source, manifest, and lockfile
```

The `--verify` checksum covers the source, the compiled **AST** (the bytes
that run), the manifest, and the lockfile, so verification fails if the
executed program or the capability grant is changed, not only the source
text. Both `--verify` and `--against` also confirm that the embedded
source **parses to the AST that runs**, so `--source` shows what runs: a
binary whose AST was swapped while its source was left clean is reported
as `DIVERGES`. `--against <dir>` compares the embedded source, manifest,
and lockfile to a checkout, reports each file as match, differ or
missing, and exits non-zero on any mismatch: it answers whether the
binary was built from that source tree.

A transparent binary is also a software bill of materials, since it
records its exact sources and dependency versions, and it enforces the
capability manifest it carries, so a restricted tool stays restricted
wherever it runs. `olang caps ./tool` prints that grant.
