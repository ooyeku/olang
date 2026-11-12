# Writing idiomatic olang

Part of [the olang book](README.md) · [Language reference](language.md) · [Common pitfalls](pitfalls.md).

Most olang that is hard to read is not hard because of the language.
It is hard because it reaches for the general tool when a better one is
there: a `#{ "k": v }` map where a record would do, `map_set` nested
three deep where one spread says it, a long `if` chain where a `match`
lines the cases up. This chapter is the habits that make olang read
well. Each section says what to write, shows it next to what it
replaces, and says why. [Writing fast olang](performance.md) covers the
habits that make it fast, and the two agree almost everywhere; where
they part, this chapter says so.

## Table of contents

1. [Records for fixed fields, maps for data](#records-for-fixed-fields-maps-for-data)
2. [Name the shapes you pass around](#name-the-shapes-you-pass-around)
3. [Enums for a closed set of cases](#enums-for-a-closed-set-of-cases)
4. [Update with a spread](#update-with-a-spread)
5. [`match` when every branch tests one value](#match-when-every-branch-tests-one-value)
6. [Pipe what would read inside out](#pipe-what-would-read-inside-out)
7. [One idea a line](#one-idea-a-line)
8. [Errors: let them travel](#errors-let-them-travel)
9. [Names, and modules instead of prefixes](#names-and-modules-instead-of-prefixes)
10. [Functions, comments and files](#functions-comments-and-files)
11. [Letting the tools hold you to it](#letting-the-tools-hold-you-to-it)
12. [What 1.0 cannot say yet](#what-10-cannot-say-yet)

## Records for fixed fields, maps for data

A value whose fields you know when you write the code is a record,
`{ name: v }`. A map, `#{ "k": v }`, is for keys that are data: file
paths, user names, words counted, anything that arrives at run time.

Records read with `.field` as maps do, print as `{ name: v }`, compare
equal to a map with the same keys, and turn into the same JSON, so
switching a value from one to the other changes nothing that reads it.
What they add is that the keys are names: no quotes to type, and
`olang check` warns when a read names a field that no construction of
the value has.

```olang
// the general tool
let wait_m = #{ "kind": "wait", "on": "server", "deadline": 20000 }
// what it is
let wait = { kind: "wait", on: "server", deadline: 20000 }
println(wait.on)                              // server
println(show(wait == wait_m))                 // true

// a map, because its keys are data
let mut seen = #{}
for w in str.split("a b a c b a", " ") { seen = map_update(seen, w, (n) => (n ?? 0) + 1) }
println(show(seen.a))                         // 3
```

A record's key may be any string; one that is not a name, or is one of
olang's reserved words, is quoted: `{ "fn": f, "content-type": t, tag: "x" }`.

Every native that takes a map takes a record too: the options of
`os.exec`, `fs.scan` or `md.parse`, the maps inside them (an
`os.exec`'s `env`), a `gui` view, its props and its theme.

**What each costs:** a record is cheaper than a map to build, to read
and to update: a spread, a `map_set` of a field it has and a
`map_merge` copy its slots without looking its shape up again. The one
place a map wins is a loop that rebinds it, `m = map_set(m, k, v)`: a
map that nothing else holds is updated in place (see [Writing fast
olang](performance.md)), and a record is copied each time.

## Name the shapes you pass around

When several functions take the same record, give its shape a name
with `type`. The alias costs nothing at run time; it tells the reader
what a value holds, and `olang check` checks each field read against
it.

```olang
type Task = { title: String, done: Bool }

fn finish(t: Task) -> Task = { ...t, done: true }
fn open_titles(ts: [Task]) = ts |> filter((t) => !t.done) |> map((t) => t.title)

let tasks = [{ title: "write", done: false }, { title: "ship", done: false }]
println(show(open_titles([finish(tasks[0]), tasks[1]])))   // ["ship"]
```

Reach for a `struct` when construction itself should be checked: a
struct literal must name every field, refuses one it does not declare,
and checks each field's type as it is built. That suits a value made in
many places that must all agree. A record shape is lighter and is
enough for a program's own state.

## Enums for a closed set of cases

A string `kind` field and an `if` chain over it is an enum the
language cannot see. Declared as one, each case carries its own data,
a `match` binds it, and `olang check` warns about a case the `match`
forgot.

```olang
type Page = enum { Diff(String, Bool), History(Int), Closed }

fn reload(p) = match p {
    Diff(root, big_ok) => "diff of " + root + (if big_ok => " (large files too)" else => ""),
    History(from) => "history from " + to_string(from),
    Closed => "nothing to reload",
}
println(reload(Diff("src", true)))
println(reload(History(0)))
```

An enum's cases take positional fields only. When a case needs more
than two or three, give it a record or a struct as its one field
(`Diff(DiffPage)`), or match records by their `kind` as the next
section but one shows.

## Update with a spread

A copy of a record or map with some fields changed is a spread:
`{ ...r, k: v }` for a record, `#{ ...m, "k": v }` for a map. One
spread says what a chain of `map_set` says, and it nests.

```olang
let rec = { role: "item", key: "a", selected: false, index: 0 }
// before: every update its own call, read inside out
let a = map_set(map_set(map_set(rec, "role", "listitem"), "selected", true), "index", 3)
// after
let b = { ...rec, role: "listitem", selected: true, index: 3 }
println(show(a == b))                          // true

// a nested update
let model = { sel: 1, view: { top: 9, height: 30 } }
let scrolled = { ...model, view: { ...model.view, top: 0 } }
println(show(scrolled.view.top))               // 0
```

Spread a value into a literal of its own kind: a record into `{ }`, a
map into `#{ }`. Spreading one into the other converts it, which costs
more than either.

Keep `map_set` for a key computed at run time, and for the
`m = map_set(m, k, v)` rebind of a map inside a loop, which is updated
in place.

## `match` when every branch tests one value

An `if … else if` chain that tests the same thing in every branch is a
`match`. The cases line up, a record pattern names the fields it reads,
and a guard says the rest.

```olang
fn describe(msg) = match msg {
    { tag: "open", arg: path } => "open " + path,
    { tag: "close" } => "close",
    { tag: t } if str.starts_with(t, "pane.") => "a pane's " + str.substring(t, 5, str.length(t)),
    _ => "ignored",
}
println(describe({ tag: "open", arg: "main.ol" }))
println(describe({ tag: "pane.o3", arg: () }))
```

Keep `if … =>` for one or two conditions on different things, and for
an early return at the top of a function:

```olang
fn reload(page, shown) = {
    if !shown => return "not shown"
    if page == () => return "no page"
    "reloading " + page
}
println(reload("diff", true))
```

Over an enum, leave out the final `_` so that `olang check` can tell you
about a case you missed.

## Pipe what would read inside out

When a value goes through several steps, a pipeline reads in the order
they happen.

```olang
let files = ["b.ol", "_scratch.ol", "a.ol"]
// before
let kept = filter(map(sort(files), (f) => "src/" + f), (p) => !str.contains(p, "/_"))
// after
let kept2 = files
    |> sort
    |> map((f) => "src/" + f)
    |> filter((p) => !str.contains(p, "/_"))
println(show(kept == kept2))                   // true
```

A pipe of one step (`x |> f`) is no clearer than `f(x)`, and a lambda
stage that only reorders arguments, `|> (m) => map_set(m, "a", 1)`, is
better said as a spread.

When an update function returns `(model, effects)` and runs several
steps, a two-line helper threads the pair so each step is one line:

```olang
fn then((m, fx), step) = {
    let (m2, more) = step(m)
    (m2, fx + more)
}
let (m, fx) = ({ count: 0 }, [])
    |> then((m) => ({ ...m, count: m.count + 1 }, ["saved"]))
    |> then((m) => (m, if m.count > 0 => ["notify"] else => []))
println(show((m.count, fx)))                   // (1, ["saved", "notify"])
```

## One idea a line

`olang fmt` lays code out but does not re-wrap a long line, so keeping
lines short is yours to do. Around a hundred columns is a good limit.
Past it, a line is almost always several ideas, and each can have a
name.

```olang
let rows = []
let regressed = true
let has_baseline = false
// before
let head = if len(rows) == 0 => "No bench ran" else => "olang bench" + (if has_baseline => " against the baseline" else => " (no baseline yet)") + (if regressed => " · SLOWER" else => "")
// after
let against = if has_baseline => " against the baseline" else => " (no baseline yet)"
let slower = if regressed => " · SLOWER" else => ""
let head2 = if len(rows) == 0 => "No bench ran" else => `olang bench${against}${slower}`
println(show(head == head2))                   // true
```

- A long record, list, argument list or `use` list goes one item a line,
  with a trailing comma.
- Text made of parts is a template string, `` `${a} at ${b}` ``. A
  template formats each value itself, so the `to_string` calls go too.
- An `if` arm's `=>` may start the next line, so a long condition wraps
  cleanly.

## Errors: let them travel

A failure the caller can handle is a `Result`. Inside a function that
returns one, `?` hands an `Err` straight back, so an error travels to
the code that can say something about it instead of turning into an
empty string at the first step.

```olang
fn port_of(text) = {
    let n = str.parse_int(str.trim(text))?
    if n < 1 || n > 65535 => return Err("a port is 1 to 65535, got " + to_string(n))
    Ok(n)
}
println(show(port_of(" 8080 ")))               // Ok(8080)
println(show(is_err(port_of("eighty"))))       // true
```

When a default really is the right answer, say so in one call:

```olang
// before
let home_m = match os.get_env("OLANG_IDIOMS_HOME") { Ok(h) => h, Err(e) => "" }
// after
let home = unwrap_or(os.get_env("OLANG_IDIOMS_HOME"), "")
println(show(home == home_m))                  // true
```

- `Err(_)`, not `Err(e)`, when the error is ignored on purpose.
- `attempt(f)` is a boundary, not a `try`: put one around a request
  handler, a plugin's call or one record of a batch, where a bug must
  not end the program — not around ordinary calls.
- `raise(text)` is for a caller who used your code wrongly (a bug), not
  for a condition a caller should handle.
- An `error` declaration gives callers errors they can match on,
  instead of text they would have to parse:

```olang
error StoreError {
    Locked,
    Missing: { path: String }
}
fn open_store(path) = if fs.exists(path) => Ok(path) else => Err(Missing(path))
println(match open_store("/no/such/store.db") {
    Ok(p) => "opened " + p,
    Err(Locked) => "locked",
    Err(Missing(p)) => "no store at " + p,
})
```

## Names, and modules instead of prefixes

A function is named for what it does (a verb), a value for what it is
(a noun). A flag reads as a question (`big_ok`, `is_open`), and a
missing one defaults with `??`: `p.big_ok ?? false`, not
`p.big_ok == true`.

A run of numbered rebinds (`m0`, `m1`, `m2`) tells the reader nothing.
A fresh `let m = …` is allowed and usually clearer, and so is naming
what each step made.

Short name prefixes (`pj_open`, `tz_run`) exist to keep the names that
files share from colliding when they are imported flat. A module's
namespace does that job and reads better:

```olang no-run
// before: every name carries two letters, and the use line lists them all
use core.project { pj_open, pj_close, pj_files }
pj_open(root)

// after: the module's name says where a function comes from
use core.project {}
project.open(root)
```

`use m {}` binds the module's namespace and nothing else. To take a
single function under another name, write `use m { f as g }`. 1.0 has
no alias for a whole module (`use m as x`), and two modules whose paths
end in the same name (`core.search` and `face.search`) bind the same
namespace, the later one winning; give one of them its own file name.
Where a project keeps prefixes, a whole word (`project_open`) reads
better than two letters.

Named arguments make a call say what a flag means:
`read_diff(page, root, big_ok: true)`. On the compiled tiers a named
call costs more than a positional one, so in a loop that runs per node
or per element, pass by position.

## Functions, comments and files

**A function does one thing and fits on a screen.** An update function
that handles every message is a router: it names the handler for each
message, and each handler is a function of its own. The routing table
is then the list of what the program handles.

**A block for a few steps, a function when they have a name.** A
block's bindings end with it, so intermediates do not leak. If you are
about to write a comment that says what a block does, make that
comment the name of a function instead.

**Comments say why.** `///` doc comments on every shared name, in
sentences: what it is, what it answers, and what `()` means there. A
`//!` header at the top of every file says what the file is for and
what it may depend on. Inside a function, a comment explains a reason
or a workaround; one that restates the next line can go.

**One file, one idea.** Order a file top to bottom: the `//!` header,
the `use` lines, shared types and constants, shared functions, private
helpers, then `test` blocks. Functions are hoisted and values are not,
so constants go above their first use. Share as little as you can, and
give a package one front file (`index.ol`) that re-exports its public
names with `share use`.

**Slots in a list are a last resort.** A record kept as a list of
positions for speed (`b[3]` meaning "the deadline") reads as noise.
Where the speed is measured and needed, name the slots
(`let DEADLINE = 3`); everywhere else, use a record.

## Letting the tools hold you to it

Several of these habits are mechanical, and olang's tools rewrite them
exactly:

| Command | Rewrites |
|---|---|
| `olang fix maps` | `map_get(x, "k")` to `x.k`, and `map_get_or(x, "k", d)` to `x.k ?? d` |
| `olang fix guards` | a guard that only kept a field read off `()` |
| `olang fix defaults` | `if x == () => d else => x` to `x ?? d` |
| `olang fix first-last` | a length-guarded `xs[0]` or `xs[len(xs) - 1]` to `first(xs)` or `last(xs)` |
| `olang fix units` | `to_int(math.floor(e))` to `math.floor(e)` |

Each takes `--check` (report, exit 1 when something would change) and
`--dry-run`, so a project's test suite can hold its code to them.

The rest can be held by a project's own rules: a `rule_` function in an
olang file, listed under `[check] rules` in `olang.toml`, that reads
each file's syntax tree and answers its findings ([Command-line
tooling](tooling.md) has the details). This one finds nested
`map_set` calls with fixed keys:

```olang no-run
// tools/rules.ol
fn chained(n) = n.kind == "call" && n.target == "map_set" && len(n.args) == 3
    && n.args[0].kind == "call" && n.args[0].target == "map_set"
share fn rule_map_set_chain(nodes) = nodes
    |> filter(chained)
    |> map((n) => { message: "nested map_set: write one spread", line: n.line })
```

A deliberate exception says `// allow: rule_map_set_chain` on its line.
olang's own source tree keeps rules for the habits in this chapter, and
a rewrite for the record and `unwrap_or` forms, in `tools/idioms/`.

## What 1.0 cannot say yet

A few things this chapter would like to recommend need the language to
change, so they wait for a release after 1.0. The
[roadmap](roadmap.md) lists them: a record literal with field
shorthand (`{ holes }` for `{ holes: holes }`), an update form for
nested fields, enum cases with named fields, string-prefix patterns in
`match`, a placeholder in pipes (`map(_.name)`), slices (`xs[a..b]`),
compound assignment (`n += 1`), an alias for a whole module, and an
`olang fmt` that re-wraps long lines.
