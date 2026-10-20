# Notebooks

Part of [the olang book](README.md) · [Command-line tooling](tooling.md)

A notebook is olang code in cells between prose: a markdown file whose
name ends `.olmd`. olang reads it as a program, so `olang run`, `test`,
`check`, `fmt` and `expand` take a notebook as they take a `.ol` file,
and every location they report is a line of the notebook itself. This
chapter is the reference for the file format and for how olang reads it.
The notebook pane, its outputs and its reactive runs are olang Studio's,
specified in its own notebook design (studio/docs/notebooks.md).

## Table of contents

- [An example](#an-example)
- [Cells](#cells)
- [Cell options](#cell-options)
- [The program](#the-program)
- [Front matter](#front-matter)
- [Params](#params)
- [Capabilities](#capabilities)
- [Controls](#controls)
- [Pinned output](#pinned-output)
- [The YAML subset](#the-yaml-subset)
- [Commands](#commands)
- [Diagnostics](#diagnostics)

## An example

````markdown
---
title: Sales by region
params:
  region: "east"
  since: "2026-01-01"
capabilities:
  fs: "read"
  net: false
---

# Sales by region

Revenue for **${region}** since ${since}.

```olang {name: load}
let sales = unwrap(ods.read_csv_file("data/sales.csv"))
```

```olang
let mine = sales |> ods.filter(sales["region"] == region)
let total = ods.sum(mine["amount"])
```

```olang {name: chart, hide: code}
plot.bar(mine["month"], mine["amount"], #{ "title": "Sales" })
```
````

`olang run sales.olmd` runs the three cells top to bottom with `region`
and `since` bound, reading files but never writing them and with no
network. `olang run sales.olmd --param region=west` runs it for another
region.

## Cells

A fenced code block whose info string begins with the word `olang` is a
code cell; everything else in the file is prose. Fences follow
CommonMark:

- a fence is three or more backticks or three or more tildes, indented
  by at most three spaces;
- a backtick fence's info string has no backtick in it;
- a block closes at a line of the same character, at least as many of
  it as opened the block, and nothing else but spaces;
- four or more spaces of indentation make an indented code block, which
  is prose.

| Info string | The block is |
|---|---|
| `olang` | a code cell |
| `olang {options}` | a code cell with [options](#cell-options) |
| `olang no-run`, `olang no-run {options}` | a cell that is shown and checked but never run |
| `output {cell: name, kind: …}` | a cell's [pinned output](#pinned-output); the program never sees it (a bare `output` block is prose) |
| anything else (`sql`, `json`, `olangx`, none) | prose |

A fence inside a list item or a block quote is not read as a cell. A
fence that is never closed is a [diagnostic](#diagnostics), not a block
that runs to the end of the file as CommonMark would have it.

## Cell options

A cell's options follow `olang` in the info string as a record,
`{key: value, …}`. A value is a bare word, a double-quoted string (`\"`
and `\\` escape), `true` or `false`.

| Option | Values | Meaning |
|---|---|---|
| `name` | a word or a string | the cell's id for links and runs; two cells may not share one |
| `hide` | `code`, `output`, `all` | what is hidden when the notebook is drawn or exported |
| `run` | `auto`, `manual` | `manual` makes a pure cell wait to be run (Studio) |
| `pin` | `true`, `false` | keep the cell's last output in the file, in an `output` block under it |
| `as` | `markdown` | a String result is drawn as markdown |

olang reads and validates the options and keeps them for the tools that
draw a notebook; a run does not depend on them. An option olang does not
know, a value outside its list, a key given twice and a malformed record
are each a diagnostic at the cell's line and the option's column.

## The program

A notebook's program is its file with every line that is not a cell's
code made empty: prose, fences, the front matter and `output` blocks
become empty lines, and the cells' lines stay as they are. Line *N* of
the program is line *N* of the file and a cell's columns are the
file's, so every diagnostic, error frame, test location and coverage
line points into the notebook as written. `olang expand x.olmd` prints
the program.

The cells form one module. A name a cell defines is visible to the cells
below it, and `test` blocks in any cell are the notebook's tests.
`olang check` holds a notebook to two rules of its own besides the
language's (a `.ol` file keeps only the language's):

- **One name, one cell.** A top-level name — a `let`, a function, a type,
  a trait, an error type — defined in two cells is an error at each,
  naming the other's line. (A `let` may shadow another in one cell, as in
  a file; a `use` of one name in two cells is no clash: each cell imports
  what it needs.)
- **Top to bottom.** A cell uses only the names defined above it: one
  that reads a name a later cell defines is an error at the use, naming
  that line — a function included, which the language would hoist.
  Reading order is the program's order.

A run reads the program as the language does, so the rules are the
check's (and the editor's), not a reason for `olang run` to refuse.

A `no-run` cell is part of the program `olang check` and `olang fmt`
read, so it must parse and is held to the checker's rules; `olang run`
and `olang test` leave it out. A later cell should not use a name only a
`no-run` cell defines: the check sees the name, the run does not.

A notebook runs in its own folder: its working directory is the folder
the notebook is in, wherever `olang` is started, so a relative path in a
cell (`data/sales.csv`) is the notebook's. `use` resolves as it would
for a `.ol` file beside the notebook: a sibling module, the enclosing
project's own modules, and the dependencies of the project whose
`olang.toml` is above it.

## Front matter

Front matter is optional: YAML between a line `---` at the very top of
the file and the next line `---` (or `...`). olang reads three keys and
leaves every other alone.

| Key | Value |
|---|---|
| `title` | a string |
| `params` | a map of each param's name to its default ([Params](#params)) |
| `capabilities` | the run's grant ([Capabilities](#capabilities)) |

Front matter that is opened and never closed is a diagnostic.

## Params

Each entry of `params` is bound as a top-level `let` before the first
cell, as if declared there. The `let` is placed on the front matter line
of the param's key, so the program keeps the file's lines. The value's
type is its YAML value's:

| YAML | olang |
|---|---|
| `~`, `null`, nothing | `()` |
| `true`, `false` | `Bool` |
| `42`, `-7` | `Int` |
| `2.5`, `1e3` | `Float` |
| `east`, `"east"`, `'east'` | `String` |
| `[a, b]`, a block list | `List` |
| `{low: 1}`, a block map | `Map` with String keys (`limits.low`) |

`--param NAME=VALUE` overrides one param's default for a run, a test run
or a check; it is repeatable. The value is olang source when it parses
as a literal (`5`, `2.5`, `true`, `"east"`, `[1, 2]`, `#{ "a": 1 }`,
`()`) and a String otherwise, so `--param since=2026-02-01` is the
String `"2026-02-01"`. `olang run` takes `--param` before the notebook
or after it: a notebook's arguments after its name are its params, and
the rest are `os.args()`'s.

```bash
olang run sales.olmd --param region=west --param since=2026-02-01
olang test analysis/ --param sample=100
olang check sales.olmd --param region=west
```

A param may be a control's spec: a map whose keys are `value` and any of
`options` (a list), `min`, `max` and `step` (numbers) — and no other
key. Its `value` is what the param binds; a value that is not one of its
`options`, or a number outside `min` … `max`, is a diagnostic at the
value, as other front matter problems are. Any other map stays a Map
param. A `--param` is held to the spec the same way (an error at the
param's line), and `reread` reads it too. olang Studio draws such a
param as its control.

```yaml
params:
  since: {value: "30 days ago", options: ["7 days ago", "30 days ago", "90 days ago"]}
  top_n: {value: 3, min: 1, max: 10, step: 1}
```

`--param` may also name a **control cell** — one defining one name with
a `ui` control (`let region = ui.select("east", […])`): its control
answers the value as it binds (`runtime.preset`), held to its kind,
range or options (an error naming the param otherwise). `--for` runs
the notebook once a set: `--for region=east,west` (every value of one
param or control, split at the commas outside quotes and brackets) or
`--for sets.json` (a list of objects; `.yaml` a list of maps; `.csv` a
header of names and a row a set), each run a child of its own under a
line naming its set, the exit the first failing run's.

```bash
olang run sales.olmd --for region=east,west,north,south
olang run sales.olmd --for sets.csv --param top_n=5
```

A `--param` that names no param the notebook declares is an error at the
line of `params:` (or line 1), and the notebook does not run. A param's
name must be one a `let` can bind: an identifier that is not a keyword.
A cell that defines a param's name again — a top-level `let`, `fn` or
`type` — is an error at that cell's line: a cell reads a param but does
not replace it.

## Capabilities

`capabilities` is the grant the notebook runs under, in the vocabulary
of `olang.toml`'s `[capabilities]` ([Packages](packages.md)):

| Key | Values |
|---|---|
| `fs` | `true`, `false`, `read` |
| `net`, `proc`, `db`, `env` | `true`, `false` |

A capability the block does not name stays allowed, as in the manifest.
The grant is intersected with the `[capabilities]` of the project whose
`olang.toml` encloses the notebook (its own grant and each dependency's
attenuation) and with `--deny` and `OLANG_DENY`: each can only remove.
`olang test` runs a notebook's tests under the same grant.

The block is fail-closed. A key outside the five, a value outside its
list (`fs: write`, `net: "no"`), and a `capabilities` that is not a map
are each a diagnostic at their line, and the notebook refuses to run
rather than run with a grant it cannot honour.

## Controls

A cell that binds a control makes the notebook a small app. `ui`'s
controls — `ui.slider`, `ui.select`, `ui.date`, `ui.text`, `ui.toggle`
and `ui.brush` over a chart (a `plot` chart's SVG, or a `gg` plot: a
range dragged in one of its panels, in that panel's data units) — each
answer their value:

```olang
use ui
let lift = ui.slider(0.08, 0.0, 0.3, 0.01)
```

Run as a program (`olang run`, `test`, `check`), `lift` is `0.08`. In a
notebook's kernel the cell's reply says its control (`control`: its
kind, the name the cell binds, its value, its default, its range or
options, a brush's chart), and the editor draws it; a value the editor
sets (`set`) is bound to `lift` at once, and kept as the cell's: running
the cell again keeps it while it still fits — the same kind, the same
default as when it was set, within the range or the options the cell
gives now. A default changed in the code, or a cell deleted, forgets it.
A control cell binds one control and defines one name (the names a
`use` binds aside); a cell binding several controls or defining several
names says its first control without a name, read-only. Binding reaches
no capability: a control cell is pure, so the cells reading it run again
on their own.

## Pinned output

A cell with `pin: true` keeps its last output in the file, so a reader
with no olang (a repository's web page, a plain editor) sees it. It is a
fenced block directly under the cell, blank lines between:

````markdown
```olang {name: top, pin: true}
ods.head(by_region, 3)
```

```output {cell: top, kind: table}
region  revenue  orders
String  Float    Int
──────  ───────  ──────
east    13597.5  96
west    13529.5  95
south   11191.9  99
4 rows × 3 columns · the first 3 shown
```
````

| `kind` | The block holds |
|---|---|
| `text` (the default) | the value's words (a string bare, a float as a person reads it), what the cell printed above them, an error as `error: …`, a control as `name = value` |
| `table` | a Frame's, a Series' or a list of records' columns aligned: names, types, a rule of `─` under each column (every column starts where its rule does), its first twenty rows (`null` for a null), its size |
| `chart` | the chart's SVG, its description inside (`plot.describe` reads it) |

The block's fence is three backticks, or one more than the longest run
opening a line of its body. A cell whose output is nothing (it only
defines) keeps no block. The rules, each a diagnostic: a pinned cell has
a `name` (its block names it); an output block stands directly under the
cell its `cell` names, and that cell is pinned. olang reads the block as
prose, so a run, `olang test` and `olang check` never see it; `olang fmt`
leaves it byte for byte. `olang notebook pin` writes them from a run (and
`--check` says which differ); olang Studio writes them as a pinned cell
runs, and draws one where nothing has run.

## The YAML subset

olang reads front matter in a strict subset of YAML, and anything
outside it is a diagnostic with its line, never a guess:

- block maps (`key: value`) and block lists (`- item`), nested by
  indentation in spaces, a list item that is itself a map
  (`- name: x`);
- flow lists `[a, b]` and flow maps `{a: 1, b: 2}`, each on one line;
- plain scalars, resolved as in the table above (any other plain text is
  a String, so `title: Sales, by region` is one String);
- double-quoted strings with the escapes `\"`, `\\`, `\/`, `\n`, `\t`,
  `\r`, `\0`, `\b`, `\f`, `\xHH`, `\uHHHH`, `\UHHHHHHHH`, and
  single-quoted strings in which `''` is a quote;
- `#` comments, at a line's start or after a space.

Not in the subset: anchors and aliases (`&a`, `*a`), tags (`!!str`),
block scalars (`|`, `>`), flow collections over several lines, a second
document, and tabs as indentation. A key given twice is an error.

## Commands

| Command | On a notebook |
|---|---|
| `olang x.olmd`, `olang run x.olmd` | runs the program; refuses on any diagnostic |
| `olang test x.olmd` | runs its `test` blocks; `olang test DIR` finds `.olmd` files beside `.ol` files |
| `olang check x.olmd` | checks its cells as one module, with the checker's advisories; `--format json` gives the file's lines, the notebook's own problems with the code `notebook` |
| `olang fmt x.olmd` | formats each cell's code, `no-run` cells too, and leaves prose, fences, front matter and `output` blocks byte for byte; `--check` reports instead |
| `olang expand x.olmd` | prints the program |
| `:run x.olmd` in the REPL | runs it in its folder, under its grant |
| `olang lsp` | a notebook is one document: its cells one module on the file's own lines, so completion (a Frame's columns and a map's keys inside the strings that name them, from the literals the program builds), hover, signature help, go to definition, references, highlights and rename work across cells, by binding (a cell's parameter, lambda or block `let` that shadows a param or another cell's name is its own); a param is its front matter key, and a name a `${expr}` in prose reads is found (and renamed) there too; prose, fences and the front matter's other lines are about nothing; its problems are `olang check`'s (the notebook's own, code `notebook`), a name only prose reads not unused; formatting is `olang fmt`'s |
| `olang run x.olmd --for NAME=A,B` | runs it once a parameter set ([Params](#params)) |
| `olang notebook export x.olmd` | runs it cell by cell (as its kernel does) and writes a self-contained page: prose, code (unless hidden), outputs (tables as HTML tables, charts as SVG in both themes, pictures inline), controls at their value (a brush draggable: the range is shaded and the points outside it dimmed on the page, nothing run — the page's only script), light and dark by the reader's appearance; `--html`, `--pdf` (macOS: WebKit's print), `--json` (the outputs document), `--outputs RUN.json` (draw given outputs, run nothing), `--param` |
| `olang notebook pin x.olmd` | runs it and writes each pinned cell's [output block](#pinned-output); `--check` writes nothing and exits 1 when one differs |
| `olang notebook app x.olmd` | writes a web-sdk app (`-o DIR`, default `<stem>-app` beside it): `olang run main.ol` serves the page, its controls form fields (a brush a range dragged across its chart) and its params editable; a change runs again, in the notebook's kernel, only the cells that read it, and the page swaps their outputs in |
| `olang repl --serve --notebook x.olmd` | the notebook's kernel: its params bound, its grant, its folder, `eval` by cell (each saying the capabilities it reached), a cell's names read without running it (`deps`), its names and values (`vars`), its front matter read again (`reread`), a cell's control and the value an editor sets for it (`set`); a Frame's column summaries, its views filtered and sorted and a column's distinct values paged by handle, a chart's description as data, an evaluation's profile ([the protocol](tooling.md#olang-repl---serve)) |

`olang check DIR` and `olang fmt DIR` take a folder's notebooks with its
`.ol` files; `--watch` re-runs when a notebook changes. `olang fmt`
formats each cell as a file of its own and holds the result to the same
safety gate as a `.ol` file: the program before and after must parse to
the same tree, or the notebook is left as it is and reported. A notebook
with a fence that is never closed is not formatted.

## Diagnostics

A notebook's own problems are reported at the file's line and column,
in the form `file:line:column: error: message` by a run and `olang test`,
and with the file's text around them by `olang check`:

- a fence or front matter that is never closed;
- a cell's info string with something after `olang` other than `no-run`
  and an options record;
- an option olang does not know, a bad value, a key given twice, a
  malformed record, two cells with one name;
- front matter outside the [YAML subset](#the-yaml-subset), a `params`
  or `capabilities` that is not a map, a param that is no name, an
  unknown capability or value;
- a `--param` the notebook does not declare, a cell defining a param;
- for `olang check`: a name defined in two cells, a cell using a name a
  later cell defines ([the program](#the-program)).

Each is an error: `olang run` and `olang test` refuse the notebook, and
`olang check` counts it as a problem (exit 1).
