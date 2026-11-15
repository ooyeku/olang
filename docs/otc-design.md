# otc: one coherent toolchain (design)

This note is the design of epic OL-576: one tool that answers "which olang
runs here, and why" and "which copy of a library does this project get, and
why", with the same verbs for olang versions and for libraries. The user's
guide is [packages.md](packages.md); this note covers the decisions behind it.

## Principles

1. **One answer.** `otc status`, `otc toolchain list` and `otc list` each
   say what is in use and which rule chose it.
2. **The same verbs everywhere.** `list`, `add`, `use`, `upgrade`,
   `outdated`, `remove` and `prune` mean the same thing for olang versions
   (`otc toolchain …`), for the shelf (`otc shelf …`) and for a project
   (`otc …`).
3. **Nothing by surprise.** The network is used only by commands that
   plainly ask for it: `toolchain list --available`, `use`/`toolchain add`
   of a release that is not installed, `update`, `add owner/repo`,
   `shelf add owner/repo`, `outdated`, `upgrade`, and an install whose
   lock names a GitHub commit the shelf does not hold yet. Nothing from a
   library runs when it is installed. A removal says what depends on the
   thing first. Every command that changes something takes `--dry-run`.
4. **Every error names the next command.**
5. **Every command takes `--format json`** and answers in one documented
   shape (below), so OlangStudio's Packages page shows the same thing.

## The verbs

```text
otc status                         what runs here and why; the project; the shelf; next steps
                                   (also plain `otc`)

otc toolchain list [--available]   every olang here (and every release, when asked)
otc use <version|name|->           install if needed, make it the default; `-` is the one before
otc use <v> --project [--local]    pin this project instead of the machine
otc toolchain add <version>        install a release without switching (alias: install)
otc toolchain remove <v> [--yes]   remove one (never the one in use); names the projects pinning it
otc toolchain prune [--yes]        remove those no known project pins and that are not the default
otc toolchain link <checkout>      a local build as a toolchain, named by its branch
otc toolchain unlink <name>
otc toolchain which [dir]          the olang a project runs
otc update                         `otc use latest` (alias: `otc toolchain upgrade`)

otc shelf list [name]              libraries held: versions, sources, size, users
otc shelf info <name>              one library: versions, users, capabilities, docs
otc shelf add <path|owner/repo[@ref]> [--rev <commit>]   hold a library without adding it to a project
otc shelf use <name>@<version|local>   the machine's default for a bare name (rule 3 below)
otc shelf outdated [name…]         newer releases of GitHub libraries
otc shelf upgrade [name…]          fetch them; old copies stay until pruned
otc shelf remove <name>[@version] [--yes]   forget it; never deletes a local folder
otc shelf prune [--yes]            remove copies nothing uses
otc shelf restore [name]           the starter libraries

otc add <path|name[@version]|owner/repo[@ref]>   add a dependency; says which it understood
otc remove <name>
otc list                           each dependency, the copy it got, and which rule chose it
otc install [--frozen] [--update]
otc outdated                       what each dependency has, the newest allowed, the newest there is
otc upgrade [name…] [--major] [--yes]
```

`otc lib` and `otc toolchain default` are gone: olang has no outside users
to migrate, so the old spellings are deleted rather than kept as aliases.

Global flags: `--format text|json` and `--dry-run` are accepted by every
command (a command that changes nothing ignores `--dry-run`). A prune
without `--yes` is a dry run, and so is a removal of something that is in
use: both say what they would remove and the command that does it.

## On disk

```text
~/.olang/                        ($OLANG_HOME)
  bin/olang, bin/otc             shims: symlinks to the default toolchain
  toolchains/<version>/bin/      managed releases, one folder per version
  toolchains.toml                default, previous (for `otc use -`), linked local builds
  shelf.toml                     the shelf's index (format 2)
  shelf/<name>/<version>/        held copies of libraries, read-only once fetched
  cache/git-db/, cache/git/      git mirrors and checkouts (prunable)
  cache/releases.json            the last answer of `--available` (prunable)
  state/projects.json            the projects otc has installed for (regenerable)
  state/warm/, state/history     as before
```

### Toolchains

A managed release is `toolchains/<version>/bin/{olang,otc}` as before. A
**linked** local build is not a folder there: it is an entry in
`toolchains.toml`

```toml
default = "1.0.0-rc.8"
previous = "0.88.0"

[linked.main]
checkout = "/Users/me/code/olang"
```

and its olang is always `<checkout>/target/release/olang`, read where it is,
never copied: a rebuild is seen at once. Its name is the checkout's branch
(`--name` overrides). Its version is the binary's own `--version`, and
`list` adds the commit, the distance from the last release tag
(`1.0.0-rc.8+39e27324 (main, 41 commits past v1.0.0-rc.8)`) and whether the
build is older than the last commit. A checkout that has gone, or that has
no build, is marked missing with the command that relinks or unlinks it.
The shims of a linked default point into the checkout (`otc` too, when the
checkout has built it; otherwise the otc shim is left as it was, and said).

A linked build never satisfies a numeric pin: a project picks one only by
name, and only in `olang.lock` (`otc use main --project --local`), because
the name means nothing on another machine.

### The shelf

`shelf.toml`, format 2:

```toml
version = 2

[libraries]
greeter = "/Users/me/code/greeter"            # a local folder, and nothing else

[libraries.geometry]
path = "/Users/me/.olang/shelf/geometry/1.2.0" # what a bare name gives (written for readers)
local = "/Users/me/code/geometry"              # a registered local folder (referenced)
default = "1.2.0"                              # `otc shelf use geometry@1.2.0`

[[libraries.geometry.copies]]
version = "1.2.0"
source = "github"                              # github | git | folder | starter
from = "ooyeku/geometry"                       # owner/repo, a git URL, or the folder a snapshot came from
ref = "v1.2.0"                                 # what was asked: a tag, a branch, a commit
commit = "9b03e7d5a2104c…"
checksum = "…"                                 # sha256 of its .ol files
dir = "geometry/1.2.0"                         # under ~/.olang/shelf/
fetched = "2026-11-14"
```

A library with only a local folder stays a plain string, so the simple
case reads as it always did. A copy's folder is `<name>/<version>/`, or
`<name>/<commit12>/` when the version is already taken by another commit
(two commits of one version, a branch head). Copies are made read-only.
Local folders are referenced, never copied.

### Known projects

`state/projects.json` lists the project roots an explicit install (`otc
install`, `add`, `remove`, `upgrade`, `use --project`) has written a lock
for. It is what lets `otc` answer "what uses this": a project whose
`olang.toml` is gone drops out when it is next read. A copy is in use when
a known project's lock, or the lock of another held library, names its
commit, or when it is what its name gives a new project.

## Which olang, and why

The olang that runs is the first `olang` on PATH (**active**); the shims'
target is the **default**; `otc use` changes the default and says when
something else on PATH shadows it, with the fix. A project's olang:

1. its lock's `olang` (an exact version, or a linked build's name), while
   the manifest's `[package] olang` minimum still allows it;
2. else the newest installed release satisfying the minimum;
3. else the running olang, when it satisfies it;
4. else none, with `otc use <pin> --project` named.

`[package] olang` takes a full version too (`"1.0.0-rc.8"`), so a project
can pin a release candidate.

`otc use` resolves its argument: an exact version; a prefix (`1.0`, `0.88`:
the newest release in that series, prereleases included); `latest` (the
newest release), `stable` (the newest non-prerelease), `rc` (the newest
prerelease); a linked build's name; `-`. An installed match answers without
the network; otherwise the releases list is read (cached for an hour).

## Which library copy, and why

For a dependency, in order:

1. **locked**: what the project's `olang.lock` pins (its commit, verified
   by checksum), while the manifest still asks for the same thing;
2. **asked**: what the manifest asks for: a version (`geometry = "1.2"`,
   `{ shelf = "geometry", version = "^1" }`), a tag, a branch or a commit
   (`{ github = "ooyeku/geometry", tag = "v1.2.0" }`): the newest held copy
   that matches, fetched from GitHub when none is held and the dependency
   names its repository;
3. with nothing asked: the machine's **default** (`otc shelf use`), else
   the **local** folder when one is registered (your working copy is the
   most up to date), else the newest **remote** copy held.

`otc list` prints the rule (`locked`, `asked`, `default`, `local`,
`remote`) beside each dependency. A version you write is a prefix: `"1.1"`
is any 1.1.x and `"1"` any 1.x; an operator (`^1.1`, `~1.1.2`, `>=1.0`,
`=1.1.0`) is a semver requirement. A bare version string is a shelf
requirement unless `OLANG_REGISTRY` names a registry. Two projects on two
versions never disturb each other: each lock names its own copy. One
project needing two versions of one name is refused with both requirers
named.

## Libraries from GitHub

`otc add owner/repo[@ref]` (or `https://github.com/owner/repo`):

- no `@`: the newest release tag (a tag that parses as a version, stable
  preferred), else the default branch's head; `@v1.2.0`, `@main`,
  `@<commit>` as asked;
- fetched with the user's own `git` (private repositories work through
  their credentials; otc stores none), into the git cache, then checked:
  the tree must hold an `olang.toml` with a `[package]`, or an `index.ol`,
  or it is refused, nothing kept;
- copied to the shelf with its commit and checksum; the library's
  `[capabilities]` are shown before it is added; nothing in it runs;
- the manifest gains `geometry = { github = "ooyeku/geometry", version =
  "^1.2.0" }` (or `tag`/`branch`/`rev` as asked) and the lock a shelf entry
  with the commit, the checksum and the repository, so another machine can
  fetch the same commit;
- a tag that now names another commit than the copy held is refused by
  `shelf add`, `shelf upgrade` and `upgrade` until accepted with
  `--accept-moved`;
- offline, an install from the lock needs nothing but the shelf.

`OLANG_GITHUB` (default `https://github.com`) and `OLANG_GITHUB_API`
(default `https://api.github.com`) say where GitHub is: the test suite
points both at a local fixture server, and no test reaches the network.
`OLANG_NET_TESTS=1` runs the one test that does.

Release downloads are verified as before (OL-491): `SHA256SUMS` against its
signature by the pinned key, then the tarball against `SHA256SUMS`.

## JSON

Every answer is one object: `#{ format: 1, kind, … }`. Listings keep their
kinds (`toolchains`, `releases`, `shelf`, `library`, `deps`, `outdated`,
`status`). A change answers `#{ format: 1, kind: "change", command,
dry_run, changes: [#{ action, what, detail }], now }`. A failure answers
`#{ format: 1, kind: "error", error, next }` on stdout, with exit code 1.
The text form of a failure is `error: …` and a last line `next: <command>`.

## Migration

The layout above replaces the old one directly; `otc doctor` reports an old
home and `otc doctor --fix` moves it:

- `shelf/pins/<name>-<sha>/` becomes a copy `shelf/<name>/<version>/`
  (source `folder`, `from` the repository it was shelved from, its commit);
- a starter at `shelf/<name>/` becomes `shelf/<name>/<version>/`;
- `shelf.toml`'s `revs` and `repos` tables are folded into copies and the
  file is written as format 2 (a shelf still holding them is refused with
  that command until then);
- a toolchain folder whose `bin/olang` is a symlink into a checkout
  (the old `toolchain link <version> <path>`) becomes a linked entry named
  by the checkout's branch;
- the default is recorded in `toolchains.toml` from where the shims point.

Locks that pinned a shelf library at a commit keep working: the commit is
looked up among the copies.
