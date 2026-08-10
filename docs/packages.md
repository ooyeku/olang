# Packages

Part of [the olang book](README.md) ·
[Tour](tour.md) · [Language](language.md) · [Standard Library](stdlib.md)

olang has a source-based package manager: a package is a directory of `.ol`
files plus an `olang.toml` manifest. There is no build step and no compiled
artifact — dependencies are fetched as source and resolved by the same `use`
mechanism as local modules, so a dependency behaves exactly like code you
wrote, minus the writing.

The package commands live in the companion tool, `otc`.

## Starting a package

`otc new` scaffolds a project — an application by default, a library with
`--lib`:

```bash
otc new myapp             # application: olang.toml + src/main.ol
otc new geometry --lib    # library: olang.toml + index.ol at the root
```

Both shapes come with a README, a `.gitignore`, and a working `test`
block, and the generated source is parse-checked before it is written.
The difference is the entry point. An application's life is
`olang src/main.ol`; a library's public API lives in `index.ol`:

```olang no-run
// geometry/index.ol — `share` marks the public API;
// anything unshared stays private to the package.
share fn area(w, h) = w * h
share let ORIGIN = (0, 0)

fn helper(x) = x + 1        // private: invisible to consumers

test "area works" {
    assert_eq(area(3, 4), 12)
}
```

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

```bash
otc new NAME [--lib]               # scaffold a project or library
otc pkg init [--name NAME]         # write olang.toml in the current directory
otc pkg add lib --path ../lib      # add a path dependency
otc pkg add http --git URL --tag v1.0.0   # add a git dependency
otc pkg add json --version "^1.0"  # add a registry dependency
otc pkg remove lib                 # drop a dependency
otc pkg install                    # fetch, replaying olang.lock when it covers the manifest
otc pkg install --frozen           # fail if resolution would rewrite the lock (CI)
otc pkg update                     # re-resolve everything, rewrite the lock
otc pkg tree                       # show the dependency graph with locked versions
otc pkg publish --registry PATH --git URL --rev COMMIT   # cut a release
```

Every `pkg` command except `init` finds the project root by walking up to
the nearest `olang.toml`; `init` writes into the directory you are in.
`otc pkg add` covers the common dependency forms — a git `rev` or
`branch` pin is written into `olang.toml` by hand.

You rarely need `otc pkg install` day to day: **running a file inside a
package resolves dependencies automatically**. `olang main.ol` reads
`olang.toml`, installs (replaying the lockfile — see below), and runs.
`olang test` does the same for test files, with one caveat: it does not
read `OLANG_REGISTRY`, so packages with registry dependencies should be
tested via a normal run or after an explicit `otc pkg install`.

## In the REPL

Start `olang` from a directory inside a package (one containing an
`olang.toml`, or any subdirectory of it) and the REPL resolves its
dependencies automatically:

```text
$ cd examples/packages/demo
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

- **Covered → replay.** `otc pkg install` (and every implicit install —
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
  (`otc pkg update` is that ask);
- a **registry** dependency is covered when the locked version satisfies
  the manifest's requirement — so loosening a requirement changes
  nothing, and tightening it past the locked version re-resolves;
- a lock entry for a package the manifest no longer needs (and that no
  other locked package depends on) breaks coverage, so removals clean
  the lock up rather than leaving fossils.

`otc pkg update` skips the coverage question entirely: it always
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

A registry is just a git repository of TOML index files — one `<name>.toml`
per package, listing each published version with its git source, commit, and
checksum. There is no hosted service required to start.

Publish a release:

```bash
otc pkg publish --registry PATH --git URL --rev COMMIT
```

The published entry records the release's *registry* dependencies so MVS
can resolve through it; a library meant for the registry should therefore
depend on registry versions itself, not on paths or git branches.

Point installs at a registry with the `OLANG_REGISTRY` environment variable
(a local directory or a checkout of the index repo).

## Trust model

The first fetch of a git or registry dependency is trust-on-first-use.
The lockfile records a sha256 checksum of each fetched source tree;
checksums are recorded for auditability but are not yet re-verified on
later installs — the commit pin is what fixes the content. Pin git
dependencies by `rev` (not just `tag`) when you need the source to be
immutable, since a tag can be moved and a rev cannot.
