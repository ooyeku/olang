# Packages

Part of [the olang book](README.md) ·
[Tour](tour.md) · [Language](language.md) · [Standard Library](stdlib.md)

olang has a source-based package manager: a package is a directory of `.ol`
files plus an `olang.toml` manifest. There is no build step and no compiled
artifact — dependencies are fetched as source and resolved by the same `use`
mechanism as local modules.

The package commands live in the companion tool, `otc`.

## A package

```
mathlib/
  olang.toml
  index.ol      # the public API surface (re-exports what's shared)
```

```toml
# olang.toml
[package]
name = "mathlib"
version = "1.0.0"
```

```olang
# index.ol
share fn square(x) = x * x
share fn cube(x) = x * x * x
```

`share` marks a binding public. A package's root module is `index.ol` (or
`mod.ol`, or `<name>.ol`) — that is what `use <name>` resolves to.

## Depending on a package

```toml
[package]
name = "app"
version = "0.1.0"

[dependencies]
# three kinds of dependency:
mathlib = { path = "../mathlib" }                         # local directory
http    = { git = "https://github.com/u/http", tag = "v1.2.0" }   # git
json    = "^1.0"                                          # registry version
```

Then use it like any module — a `use` whose first segment is a dependency
name resolves inside that dependency:

```olang
use mathlib { square, cube }
println(square(5))     // 25
```

Dependency names win over local files, so `use mathlib` always means the
package, never a sibling `mathlib.ol`.

## Commands

```bash
otc pkg init                       # scaffold olang.toml
otc pkg add lib --path ../lib      # add a path dependency
otc pkg add http --git URL --tag v1.0.0   # add a git dependency
otc pkg add json --version "^1.0"  # add a registry dependency
otc pkg remove lib                 # drop a dependency
otc pkg install                    # resolve + fetch, write olang.lock
otc pkg install --frozen           # fail if the lock would change (CI)
otc pkg tree                       # show the resolved dependency graph
```

Running a file inside a package resolves dependencies automatically — `olang
main.ol` reads `olang.toml`, installs, and runs — so `otc pkg install` is
mainly for pre-fetching and inspecting the lock.

## In the REPL

Start `olang` from a directory inside a package (one containing an
`olang.toml`, or any subdirectory of it) and the REPL resolves its
dependencies automatically:

```
$ cd examples/packages/demo
$ olang
Olang v0.41.0
Package 'demo' loaded — its dependencies are available via `use`

olang> use geometry { circle, area }
olang> area(circle(2.0))
12.5663706
```

A `use` still binds the names — the package context just makes it
resolvable. `:cd` into another package re-resolves; `:pkg` re-resolves the
current one (after editing `olang.toml`) or reports that there is no
package here.

A package is also referable by its own name from within itself, so you can
test a package in its own REPL — `use geometry { circle }` works from
inside the `geometry` package, not only from a package that depends on it.

To use a package **without cd-ing into it**, load it by path:

```
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

`olang.lock` pins every dependency exactly — a git commit SHA, a resolved
registry version, or a path — plus a sha256 checksum of the source tree.
Commit it: a fresh `otc pkg install` on another machine reproduces
byte-identical code, and `--frozen` makes CI fail if resolution drifts.

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

Point installs at a registry with the `OLANG_REGISTRY` environment variable
(a local directory or a checkout of the index repo). Fetched sources are
cached, content-addressed by commit, under `~/.olang/cache` (override with
`OLANG_CACHE`), so a revision is downloaded once and shared across projects.

## Trust model

Checksums in the lockfile detect tampering when a dependency is re-fetched.
The first fetch of a git or registry dependency is trust-on-first-use; pin
git dependencies by `rev` (not just `tag`) when you need the source to be
immutable.
