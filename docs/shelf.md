# The library shelf

The shelf is olang's local package registry: a per-user catalog of
libraries, kept by name, importable from any project on the machine.
It answers the question every growing codebase meets — "how do I use
my own library from another project without remembering where it
lives" — and it answers it without a server, an account, or a network:
the shelf is one small TOML file, and the libraries are ordinary
directories of olang source.

A fresh shelf does not start empty. The first time it is touched it is
seeded with a small curated set of **starter libraries** — pure olang,
documented, tested — so a new installation can `otc add textkit` in
its first minute. Starters are ordinary shelf citizens: remove one and
it stays removed; restore it and it comes back.

- [The model](#the-model)
- [Commands](#commands)
- [The starter libraries](#the-starter-libraries)
  - [textkit](#textkit--text-layout-and-humane-formatting)
  - [validate](#validate--declarative-checks-for-map-shaped-input)
  - [markdown](#markdown--a-markdown-renderer)
- [Pinning and drift](#pinning-and-drift)
- [Doc comments reach `:help`](#doc-comments-reach-help)
- [Locations and environment](#locations-and-environment)

## The model

The shelf maps names to directories. Registering a library records
where it lives; depending on it records only the *name*:

```toml
# olang.toml — the manifest a project commits
[dependencies]
geometry = { shelf = "geometry" }
```

The manifest stays free of machine-specific paths, so it means the
same thing on every machine that has a `geometry` on its shelf. The
lockfile pins what the name resolved to — directory and content
checksum — so nothing moves or changes without being noticed (see
[Pinning and drift](#pinning-and-drift)).

A shelvable library is a directory with an `index.ol` (the entry point
`use` resolves) or an `olang.toml` naming it. `otc new <name> --lib`
scaffolds exactly that shape.

## Commands

Register a library once, per user, from anywhere:

```bash
otc lib add ~/code/geometry   # registers under its package name
otc lib add . --name geo      # the current directory, under a chosen name
otc lib list                  # every shelved library, and where it points
otc lib remove geometry       # take it off the shelf
otc lib restore textkit       # re-shelve a starter (or refresh it)
otc lib restore               # re-shelve every starter
```

Then, from any project anywhere:

```bash
otc add geometry              # manifest gains geometry = { shelf = "geometry" }
otc install                   # resolve and pin (otc add does this too)
```

`otc lib list` annotates each entry: `(starter)` for the shipped
libraries, `(missing — directory is gone; re-register, remove, or
restore)` when a registered directory has disappeared.

Removing a *user* library only forgets the registration — the source
directory is untouched, and projects already depending on it keep
their lockfile pin until their next install. Removing a *starter* also
deletes its materialized directory, because the shelf owns that copy;
`otc lib restore <name>` writes it back. Restore doubles as refresh:
restoring a starter that is already present overwrites it with the
running toolchain's copy, which is how starters pick up improvements
after an upgrade.

## The starter libraries

Three libraries ship with every shelf. Each is pure olang — no
capabilities, no effects — each function carries a `///` doc comment,
and each library carries its own `test` blocks, which run in the
toolchain's test suite on every change: a starter without passing
tests cannot ship. The sources are compiled into the binary, so
seeding works offline; the materialized copies are versioned with the
toolchain that wrote them.

They are deliberately few. The bar for adding one is a task that
programs keep solving by hand — the shipped set exists because its
functions kept being rewritten in example after example.

### textkit — text layout and humane formatting

The functions every report, table, and CLI ends up writing by hand.

| Function | Does |
|---|---|
| `rpad(s, w)` / `lpad(s, w)` | pad right/left with spaces to width `w`; longer strings pass through |
| `center(s, w)` | center in width `w`, extra space to the right |
| `rule(w)` | a horizontal rule of `w` box-drawing dashes |
| `wrap(text, width)` | greedy word wrap, returned as a list of lines |
| `columns(rows, gap)` | align rows of cells into columns, `gap` spaces apart |
| `table(headers, rows)` | a finished table: headers, rule, aligned body |
| `money(cents)` | integer cents as currency — exact, comma-grouped, negative-safe |
| `human_bytes(n)` | a byte count at a humane scale: `"1.5 KB"` |
| `human_duration(ms)` | milliseconds as people say it: `"3.2s"`, `"5m 07s"` |
| `slugify(s)` | a url- and filename-safe slug: `"hello-world"` |

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

Money stays in integer cents end to end — that is what makes it exact;
`money` is the display edge, negative-safe where a naive formatter
loses the sign of the cents.

### validate — declarative checks for map-shaped input

Data arriving from a form, a JSON body, a CSV row, or a config file is
a Map of unknowns. `validate.check` holds it against a rule list and
reports **every** problem, each naming its field, so a caller can show
a complete error report in one round trip.

| Function | Does |
|---|---|
| `check(m, rules)` | `Ok(m)`, or `Err(problems)` — one `"field: problem"` string per failure |
| `ok(m, rules)` | just the verdict, as a Bool |

A rule is `[field, kind]` or `[field, kind, opts]`. Kinds: `"str"`,
`"int"`, `"float"`, `"num"`, `"bool"`, `"list"`, `"map"`, `"any"`.
Opts: `required: false` (absent is fine; present still checks),
`min`/`max` (numeric bounds, or length bounds for strings and lists),
`one_of` (an allowed-values list), `pattern` (a regex the whole string
must match).

```olang no-run
use validate { check }

let rules = [
    ["date",   "str", #{"pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"}],
    ["amount", "int", #{"min": 1}],
    ["kind",   "str", #{"one_of": ["expense", "income"]}],
    ["note",   "str", #{"required": false, "max": 200}]
]

match check(#{"date": "2026-8-25", "amount": 0, "kind": "loan"}, rules) {
    Ok(row) => println("valid"),
    Err(problems) => for p in problems { println(p) }
}
// date: does not match ^[0-9]{4}-[0-9]{2}-[0-9]{2}$
// amount: below minimum 1
// kind: must be one of ["expense", "income"]
```

`kind` may also be `"date"` — an ISO day the value must parse as, with
`min`/`max` as ISO days it must not precede or exceed — and any rule may
carry `"where": (v) => Result`, a predicate whose `Err` message becomes
the field's problem, so a check that fits no kind still rides the one
validation pass. Parsed JSON (`JsonObject`) is map-shaped and checks
like a Map: a request body needs no copy first.

```olang no-run
use validate { check, ok }
let rules = [
    ["due", "date", #{ "required": false, "min": "2026-01-01" }],
    ["n", "int", #{ "where": (v) => if v % 2 == 0 => Ok(v) else => Err("must be even") }]
]
println(show(ok(#{ "due": "2026-08-31", "n": 4 }, rules)))
println(show(check(#{ "n": 3 }, rules)))
```

### markdown — a Markdown renderer

`markdown.to_html(text)` turns a Markdown document into HTML:
headings, paragraphs, `` `code` `` spans and fenced blocks,
**strong**, *emphasis*, links, bulleted and numbered lists,
blockquotes, and horizontal rules. Inline markers nest; unmatched
markers fall through as literal text; everything else is escaped, so
untrusted input renders as text rather than markup.

| Function | Does |
|---|---|
| `to_html(md)` | a whole Markdown document, as HTML |
| `render_inline(s)` | one line's inline spans, for callers with their own block structure |
| `escape(s)` | HTML-escape text: `&`, `<`, `>` become entities |

```olang no-run
use markdown { to_html }

println(to_html("# Notes\n\nShip the **shelf** first."))
// <h1>Notes</h1>
// <p>Ship the <strong>shelf</strong> first.</p>
```

GitHub-style task lists render as checkboxes: `- [ ] write tests`
becomes `<li class="task"><input type="checkbox" disabled> write
tests</li>` (checked and `class="task done"` for `[x]`), kept in source
order so a click in the rendered page can map back to the n-th task
line.

## Pinning and drift

`otc add` and `otc install` write `olang.lock` with the directory the
shelf resolved and a checksum of its contents. From there:

- **A moved or deleted library** fails the next install with the name
  and the path that stopped existing — not a silent import error at
  runtime.
- **An edited library** is normal development: the change is reported
  informationally at install, never treated as tampering, and the lock
  re-pins.
- **`otc install --frozen`** replays the lockfile exactly and fails on
  any drift — the reproducible-build mode.

Because starters are versioned with the toolchain, a lockfile pin on a
starter behaves like any other: upgrading olang and running `otc lib
restore` refreshes the shelf copy, and the next `otc install` in each
project re-pins it, visibly.

## Doc comments reach `:help`

Shelved libraries are plain olang source, so the `///` convention
applies. In the REPL, once a library is imported, `:help` answers for
it — the starter libraries are fully documented this way:

```text
olang> use textkit { money }
olang> :help money

═══ money ═══

  share fn money(cents)

  Integer cents as currency, negative-safe and comma-grouped:
  `money(123456789)` is "$1,234,567.89", `money(-150)` is "-$1.50".
  Keeping amounts in cents end to end is what makes them exact.
```

The same sources feed `olang doc`, which renders a browsable API page
for any directory of documented olang code — your shelf included.

## Locations and environment

| Path | Holds |
|---|---|
| `~/.olang/shelf.toml` | the registry: name → directory |
| `~/.olang/shelf/<name>/` | materialized starter libraries |
| `OLANG_SHELF` | overrides the registry file's location (starters materialize into `shelf/` beside it); how tests isolate |

Seeding happens exactly once, when the shelf file first comes into
being. A shelf whose starters were deliberately removed stays exactly
as its owner left it — nothing respawns behind your back.
