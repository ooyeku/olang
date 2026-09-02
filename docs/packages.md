# Packages and dependencies

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Language reference](language.md) · [Standard library reference](stdlib.md)

olang has a source-based package manager. A package is a directory of `.ol`
files with an `olang.toml` manifest. There is no build step and no compiled
artifact: dependencies are fetched as source and resolved by the same `use`
mechanism as local modules, so a dependency is used the same way as code
written in the same project.

Projects are managed by the companion tool `otc`, whose division of labor
with the `olang` binary is one sentence: everything that touches a *file*
lives in `olang` (run, repl, test, fmt, check, bench, profile);
everything that touches a *project* lives in `otc`. The workflow is
local-first — your own libraries, on your own machine, by name — with
git and registry sources available underneath for when a dependency
lives elsewhere.

## Table of contents

- [Starting a package](#starting-a-package)
- [The manifest](#the-manifest)
- [Commands](#commands)
- [The library shelf](#the-library-shelf)
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
SDK](web-sdk.md), so it is two files: `main.ol` (migrations, rpc
routes, `serve`) and `client.ol` (`mount`, `action`, `call`). The
manifest depends on the shelf's `web` and `validate`; `otc install`
resolves them. Every seam a real app grows along appears exactly once.
`--web-bare` keeps the previous shape — the raw stdlib with the router,
static routes, escaping, and schema hand-rolled — for studying what the
SDK packages. Its README covers the one artifact the scaffold cannot
write from source (the wasm runtime; the scaffold copies one in when it
can find it, and the API works without it).

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

Nothing forces the split — a one-file library that `share`s directly from
`index.ol` is still a valid package, and the resolver does not care. The
scaffold starts with `lib/` because a library that grows without one puts
everything in the root module by default, and the seam is harder to
introduce later than to keep.

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

Git dependencies take one of `tag`, `rev`, `branch` — or none, meaning the
default branch's head. Registry requirements accept the usual semver
operator syntax (`^1.0`, `>=1.2, <2.0`, an exact `1.4.0`).

Then use a dependency like any module — a `use` whose first segment is a
dependency name resolves inside that package:

```olang no-run
use geometry { area, ORIGIN }
println(to_string(area(5, 5)))
```

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

The whole surface is seven verbs:

```bash
otc new NAME [--lib|--web]   # scaffold a project
otc add ../my-lib            # add a dependency by path
otc add my-lib               # add a dependency by name, from your shelf
otc remove my-lib            # drop a dependency
otc list                     # what this project depends on, and where each lives
otc install                  # fetch dependencies, honoring olang.lock
otc install --frozen         # fail if resolution would rewrite the lock (CI)
otc install --update         # re-resolve everything, rewrite the lock
otc lib add|list|remove      # your library shelf (next section)
otc bench                    # the project benchmark harness (below)
```

Every command finds the project root by walking up to the nearest
`olang.toml` — and `otc add` in a directory with none **creates one**,
so a project starts at the moment you first need a dependency, with no
separate init step to know about. `add` takes one argument with two
readings: anything with a path separator (or a leading `.`) is a
directory, recorded relative to the project root no matter where the
command ran; a bare name is looked up on your shelf. Either way the
dependency is validated at `add` time and resolution runs immediately,
so the lockfile is never left behind the manifest, and changing an
existing dependency's source requires `--force`.

Dependencies that live elsewhere still work — write them into
`olang.toml` directly (`{ git = "URL", tag = "v1.0" }`, or a registry
requirement with `OLANG_REGISTRY` set) and `otc install` resolves them
with the same lockfile discipline. The local forms are the ones with
first-class commands because they are the daily workflow.

You rarely need `otc install` day to day: **running a file inside a
package resolves dependencies automatically**. `olang main.ol` reads
`olang.toml`, installs (replaying the lockfile — see below), and runs.

## The library shelf

The shelf answers "how do I use my own library from another project
without remembering where it lives". Register a library once, per user:

```bash
otc lib add ~/code/geometry     # registers under its package name
otc lib list                    # what is shelved, and where each points
otc lib remove geometry
```

From then on, any project anywhere:

```bash
otc add geometry
```

The manifest records only the name — `geometry = { shelf = "geometry" }`
— so `olang.toml` stays free of machine-specific paths. The lockfile
pins the directory the shelf resolved to, plus a content checksum, so a
library that moves or disappears is noticed at the next install rather
than silently drifted past (editing a shelved library is normal
development and reported informationally, never as tampering). The
shelf itself is one small TOML file at `~/.olang/shelf.toml`
(`OLANG_SHELF` overrides the location).

A fresh shelf arrives stocked with the curated starter libraries —
`textkit`, `validate`, `markdown` — removable and restorable like
anything else (`otc lib restore`). The shelf has its own chapter:
[The library shelf](shelf.md) covers the model, the commands, the
starter libraries' full APIs, and pinning in detail.

## The home directory

Everything the toolchain writes under the user's home lives in
`~/.olang`, whose layout is a stated contract (defined once in the
runtime, `src/home.rs`):

```text
~/.olang/
  bin/          shims pointing at the default toolchain
  toolchains/   <version>/bin/{olang, otc} — managed by `otc update`
  shelf/        registered library copies
  shelf.toml    the shelf manifest
  cache/        git dependency clones (prunable)
  state/        warm hints, REPL history (regenerable)
```

`OLANG_HOME` relocates the whole tree (tests use this). Three commands
manage it:

- **`otc doctor`** audits an installation: every `olang`/`otc` on
  PATH with its version and origin (cargo, Homebrew, olang-home) and
  which copy wins; version agreement between `olang` and `otc`; legacy
  files and directories from older layouts; shelf entries whose paths
  no longer resolve; and the sizes of the prunable parts. The default
  run is read-only; `otc doctor --fix` applies the safe repairs —
  removing legacy scaffolding, migrating the REPL history — and never
  touches installations owned by other package managers.
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
invocations, explicitly.

Multiple releases install side by side:

```bash
otc toolchain list              # installed versions, default marked
otc toolchain install 0.78.0    # add a specific release
otc toolchain default 0.78.0    # switch the shims
otc toolchain remove 0.78.0     # remove (the default refuses)
```

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
otc bench sums --runs 7          # filter by name, more repetitions
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
Every real performance bug this codebase has hunted appeared as a curve
before it was a number, which is why the harness fits curves.

The two output conventions are contracts, not decoration. A `CHECKSUM`
line must agree across every repetition or the measurement is refused —
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
everything the package `share`s.

After importing, the module name is a namespace you can inspect and call:
`geometry.area(...)`, or `:help geometry` in the REPL to list its functions.
This works for the embedded `colx` collections module too (`:help colx`).

## The lockfile

`olang.lock` records, for every package in the dependency graph, exactly
which source produced it: the path for a path dependency; the git URL,
the resolved commit, and the ref the manifest asked for; the resolved
version for a registry dependency — plus a checksum of the source tree
and the package's own direct dependencies. Commit it.

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

Registry distribution is currently **dormant surface**: the resolution
machinery below works and is exercised by tests, but the CLI leads with
the local workflow, and no public registry exists yet. This section
documents the mechanism for when distribution matters.

A registry is just a git repository of TOML index files — one `<name>.toml`
per package, listing each published version with its git source, commit, and
checksum. There is no hosted service required to start.

Publish a release:

```text
(publishing is part of the dormant registry surface; the index format
below is what a release writes)
```

The published entry records the release's *registry* dependencies so MVS
can resolve through it; a library meant for the registry should therefore
depend on registry versions itself, not on paths or git branches.

Point installs at a registry with the `OLANG_REGISTRY` environment variable
(a local directory or a checkout of the index repo).

## Trust model

The first fetch of a git or registry dependency is trust-on-first-use.
From then on the checksums bite: the lockfile records a sha256 of each
fetched source tree, and **every later install re-verifies fetched
(git/registry) content against it** — a mismatch is a hard error, not a
warning. A registry release's published checksum is likewise enforced at
fetch time, and the registry index itself is append-only: publishing
refuses to rewrite an already-published version. Path and shelf
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
`proc` module, `os.exec`, and `os.exit`. Terminating the host is a
larger power than spawning a child, so a dependency refused the second
does not get the first.

```toml
[capabilities]
fs = "read"        # false | "read" | true
net = false
proc = false
db = true
env = true
```

Absent, capabilities are wide open — a program with no manifest, or a
manifest with no `[capabilities]`, runs unrestricted, so this is
opt-in and never breaks existing code. A denied call is a runtime
error naming the capability, the call, and the grant that refused it.

**Per-dependency attenuation** grants a dependency fewer capabilities than
the application, never more:

```toml
[capabilities.dependencies.leftpad]
fs = false         # this dependency cannot touch the filesystem,
net = false        # even if a future version tries to
```

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
That split is deliberate: a data module that could be handed a path would
be a filesystem capability under another name.

### Asking what you were granted

A denial stops the program. That is deliberate: a call that needed a
capability it did not have is a mistake in how the program was deployed,
and continuing past it would run a program the manifest does not
describe.

But a program that can degrade should be able to look before it leaps,
and until now it could only attempt the call and be killed by it — so
degradation had to be written as recovery from an error, which the
language does not offer. The `caps` module closes that gap:

```olang
let plan = if caps.allowed("fs") => "cache to disk" else => "in memory"
println(caps.level("fs"))        // one of: none, read, full
let all = caps.granted()         // the whole grant, as a map
println(show(map_get(all, "net")))
```

`caps.allowed` answers for the **caller**, not the application: code in an
attenuated dependency sees what *that dependency* was given, which is the
same set the gate would enforce a moment later. One classification, asked
two ways — the same relationship `check` and `required` already have.

None of the three is itself gated. Asking what you hold reaches nothing
and reveals nothing a caller could not learn by making the call and
reading the error, so the escape hatch stays available under the
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
olang run --trace-caps program.ol
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
value, so applying the generated block can only reduce access. The
profiler observes every effect on either tier, so a promoted function's
demands appear in the profile alongside an interpreted one's.

Add `--write` to fold the block directly into the package's `olang.toml`:

```bash
olang run --trace-caps --write program.ol
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

Capabilities are enforced on **both** tiers, at the one dispatch point
every builtin passes through. A compiled function carries the file it was
declared in, the bytecode tier keeps a stack of those files as it
executes, and the gate reads the innermost one — the same attribution the
interpreter takes from its own call stack. A promoted function in an
attenuated dependency is therefore judged by *that dependency's* grant,
exactly as the interpreted one was — and this holds across threads: a
dependency's code reached through `spawn`, `par_map`, `par for`, or an
`http.serve` handler runs on a worker with its own bytecode tier, and
that tier carries the same gate, so a worker cannot exercise a
capability the main thread would have been refused.

A capability-restricted run keeps the full tier and the full speed. This
was not always true: a manifest used to switch the bytecode tier off,
because the tier reaches some builtins through a bridge interpreter that
carried no grant table and no idea which function was running, and a
partial gate is worse than none. Turning on the security feature cost
roughly two orders of magnitude on hot numeric code.

## The transparent binary

An `olang build` executable embeds its complete source, its `olang.toml`
and `olang.lock`, its capability manifest, and a checksum over all of
them. `olang inspect` reads and verifies these files without external
context:

```bash
olang inspect ./tool             # Print a summary: version, checksum, capabilities.
olang inspect ./tool --source    # Print the embedded source.
olang inspect ./tool --manifest  # Print the embedded olang.toml.
olang inspect ./tool --caps      # Print the resolved capability grant.
olang inspect ./tool --verify    # Verify the checksum.
olang inspect ./tool --against . # Compare the binary to a source tree.
olang inspect ./tool -o dir/     # Extract source, manifest, and lockfile.
```

The `--verify` checksum covers the source, the compiled **AST** (the bytes
that actually run), the manifest, and the lockfile — so verification fails
if the executed program or the capability grant is changed, not only the
source text. Both `--verify` and `--against` additionally confirm that the
embedded source **parses to the AST that runs**, so `--source` is honest: a
binary whose AST was swapped while its source was left clean is reported as
`DIVERGES`. `--against <dir>` compares the embedded source, manifest, and
lockfile to a checkout and reports whether the binary was built from that
source tree.

A transparent binary serves two additional purposes. It is a software bill
of materials, because it records its exact sources and dependency
versions. It also enforces the capability manifest it carries, so a
restricted tool remains restricted wherever it runs.
