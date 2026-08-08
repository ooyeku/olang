# Roadmap

Where olang goes from 0.26. Every item here traces to concrete friction hit
while building the [example programs](../examples/) — nothing is speculative.
All of it is additive, per the [stability policy](stability.md): documented
syntax and behavior do not change; the language grows and gets faster.

Part of [the olang book](README.md) ·
[Stability](stability.md) · [Internals](internals.md)

Statuses: **planned** → **in progress** → **landed** (with the release).

## Tier 1 — completing what exists

Small, high-value items that finish surfaces the language already has.

| # | Feature | Grounding | Status |
|---|---|---|---|
| 1 | **Write-side struct-likeness**: `map_set` / `map_remove` accept objects, structs, and parsed JSON (returning a new value of the same kind), matching the read-side uniformity of `map_get`/`map_keys` | Workflow engine kept contexts as raw maps solely because parsed JSON couldn't be updated | landed (unreleased) |
| 2 | **`for` pattern destructuring** — `for (i, x) in enumerate(xs)`, plus an `entries(m)` builtin so maps iterate as `(key, value)` pairs | `pair[0]`/`pair[1]` indexing in markdown, run_all, and workflow | landed (unreleased) |
| 3 | **Strings iterate**: `for ch in "abc"` yields 1-character strings | `str.chars` detours in the regex engine and parser library | landed (unreleased) |
| 4 | **`show(v)` builtin** — display rendering: strings bare, everything else as `to_string`. `to_string` keeps its repr form (strings quoted) unchanged | Quote-stripping workarounds in workflow, jsonschema, and the `join` fix | landed (unreleased) |

## Tier 2 — control flow and errors

| # | Feature | Grounding | Status |
|---|---|---|---|
| 5 | **`return expr`** — early exit from a function; reuses the `?` unwind machinery | `let mut go = true` loop flags in the template and markdown parsers | landed (unreleased) |
| 6 | **Implement `error` declarations** — variants become constructors producing `Err`-matchable values (`match r { Err(NotFound) => ... }`); moves the construct from Reserved to Stable | The webserver's hand-rolled 400/404/500 taxonomy | landed (unreleased) |
| 7 | **`break value`** — `loop { ... break x }` as an expression | pairs with `return` | landed (unreleased) |

## Tier 3 — stdlib gaps

| # | Feature | Grounding | Status |
|---|---|---|---|
| 8 | **`time` module** — `now_ms`, `monotonic_ms`, `sleep(ms)` | run_all times in whole seconds via `dates.timestamp(dates.now())`; no sub-second clock exists | landed (unreleased) |
| 9 | **`fs.walk` / `fs.glob`** | run_all hand-rolled two-level directory discovery over flat `list_dir` | landed (unreleased) |
| 10 | **`os.exec` options** — optional third argument `#{ "cwd": ..., "stdin": ..., "env": ... }`; 2-arg form unchanged | run_all wraps every exec in `os.chdir` | landed (unreleased) |
| 11 | **`str.fmt("{} of {}", a, b)`** — display-form placeholders | long `+`/`pad_end` chains in every program that prints tables | landed (unreleased) |
| 12 | **db transactions** — `db.begin` / `db.commit` / `db.rollback` | the webserver's create-then-select pattern is unsafe under future concurrency | landed (unreleased) |

## Tier 4 — the two big lanes

| # | Feature | Grounding | Status |
|---|---|---|---|
| 13 | **OVM coverage expansion**, starting with `&&`/`||` as conditional jumps — today any function containing them falls back to the tree-walker permanently. Then strings, field access, closures. Every expansion lands with tier-agreement tests | verified: the OVM compiler has no And/Or lowering; guards are everywhere post-dogfooding | planned |
| 14 | **`http.serve` keep-alive**, then bounded concurrency — the handler contract is fixed; the execution model grows (as [stability](stability.md) already carves out). Concurrency waits on a shareable-interpreter story | sequential + Connection: close is the honest current model | planned |

## Tooling track

| # | Feature | Grounding | Status |
|---|---|---|---|
| 15 | **`olang test`** — discover and run `test` blocks across a package in isolation, with reporting | examples now carry self-check `test` blocks; make the pattern first-class | planned |
| 16 | **`olang fmt`** — enforce the book's conventions mechanically | the fixed idioms (`= {` bodies, `=>` arms) are formatter-shaped | planned |

## Explicitly not planned

- **Qualified `Type::Variant` syntax** — variant constructors already travel
  with imports; a second access form would split the idiom.
- **Union type declarations** — enums cover the use cases; the annotation
  forms stay reserved.
- **Mutable closure capture** — capture-by-value is load-bearing for modules
  and OVM promotion. State lives in explicit stores; if demand appears, an
  explicit `cell` module beats changing capture semantics.
- **Macros / metaprogramming** — ten real programs never wanted one.

## Process

Each item lands the way everything since 0.25 has: root-cause
implementation, regression tests, both execution tiers agreeing (or the OVM
explicitly refusing), the book updated in the same commit — the doc tests
hold the two together — and a CHANGELOG entry saying why.
