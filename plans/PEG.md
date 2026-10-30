# `peg` — parsing structured text, fast and with good errors

*Local-only (branch `rc6`, olang 1.0.0-rc.6) until it is polished. The
remote stays on rc.5.*

## What it is for

Programs keep meeting text with structure: a query bar (olang-track's
`project: OL is: overdue sort: -updated`), a config format, a log line,
a formula, a route pattern, a small rules notation, a protocol. Today
each one is a hand-written scanner with its own bugs and its own poor
error messages. `peg` makes each a grammar of a few lines, giving back a
tree with positions, errors that say what was expected and where, and a
speed close to hand-written code.

It is a library, not a language workbench: no new syntax in olang, no
editor plugins, no registering file types.

## How it reads

```olang
use peg

let Query = peg.grammar(```
  query   = term*
  term    = field ":" value / word
  field   = Name
  value   = "-"? (Quoted / Bare) ("," (Quoted / Bare))*
  word    = "-"? (Quoted / Bare)
  Name    = [a-z_]+
  Bare    = [^ \t",]+
  Quoted  = "\"" [^"]* "\""
  skip    = [ \t]+
```)?

match Query.parse("project: OL is: overdue -draft") {
    Ok(tree) => tree.terms,                     // labelled nodes, each with its span
    Err(e) => println(e.show())                 // "expected a value after `is:` at 1:13"
}
```

- **The notation:** the grammar as a string (an ordinary olang string; a
  raw multi-line string keeps it readable). Rules in lower case may have
  space between their parts, as `skip` says. Rules in capitals are tokens,
  matched with nothing in between.
- **Expressions:** a precedence table instead of a rule for every level:
  ```
  expr = precedence(atom) { left "+" "-"; left "*" "/"; right "^"; prefix "-" }
  ```
- **Trees:** each match is a node with its `kind`, its `text`, its
  `span` (line, column, byte range) and its children. Labels name the
  parts (`term = key:field ":" val:value`), so a node reads as `t.key`
  and `t.val`.
- **Actions, when a tree is not wanted:** an olang function per rule turns
  a match straight into a value: `peg.grammar(text, #{ "Number": (m) => to_float(m.text) })`.
- **Left recursion** is allowed (`sum = sum "+" product / product`), so
  grammars can be written the natural way.

## Errors people can act on

- The furthest point the parse reached and what could have come there,
  in words: `expected a value after "is:" (1:13)`.
- `e.show()` prints the line with a caret; `e.span`, `e.expected` and
  `e.found` are data for an app (olang-track's query bar underlines the
  spot).
- **Recovery, when asked** (`#{ "recover": ["\n", ";"] }`): the parse
  skips to the next sync point and goes on, so one pass reports every
  error in a file.

## Fast

*(Since rc.6 the matching is native and the olang functions below are
the reference engine kept for testing: plans/PEG-NATIVE.md.)*

- A grammar compiles once into olang functions, one per rule, written
  as recursive descent. They are ordinary code, so the bytecode tier
  takes them when they are hot; there is no grammar interpreter on the
  hot path.
- Memoisation (packrat) only where a rule is tried twice at one place, so
  memory stays flat on simple grammars.
- **A budget, tested in the benchmarks:**
  - a JSON grammar within 3× of the built-in `json.parse`;
  - olang-track's query language within 2× of its current hand-written parser.

## Tests and tools in the box

- `peg.check(grammar)`: rules nothing uses, an ordered choice whose later
  alternative can never match, left recursion it cannot resolve.
- `peg.examples(grammar, n, seed)`: valid inputs generated from the
  grammar, for fuzzing whatever consumes the trees.

## Milestones

| | milestone | done when |
|---|---|---|
| G1 | parse | grammars from text, tokens and `skip`, labels, trees with spans, left recursion, precedence tables, errors with expected sets; tested over JSON, CSV, an expression language and olang-track's query language |
| G2 | fast | the grammar compiled to olang functions on the bytecode tier, packrat only where needed; the budget met and held by a benchmark |
| G3 | useful | actions, recovery, `peg.check`, `peg.examples`; docs/stdlib.md's `peg` section with three worked examples |
| G4 | proven | olang-track's query bar parsed by `peg` with the same answers and errors at least as good, at the budget's speed |

## Separately: expansion cost

Not part of `peg`, but measured on the way: expanding an imported
`meta fn` cost about a second on rc.5 (one `@timed` use, 0.95 s of
olang-track's start). It is worth its own fix on `rc6` whether or not
`peg` uses macros.

## Done, and polished, when
- G1–G4 hold.
- The `peg` section of the stdlib docs teaches it in one page.
- A new structured format takes an afternoon, not a week.
