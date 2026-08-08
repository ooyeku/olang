# Roadmap

Where olang goes from 0.27. Every item here traces to concrete friction hit
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
| 1 | **Write-side struct-likeness**: `map_set` / `map_remove` accept objects, structs, and parsed JSON (returning a new value of the same kind), matching the read-side uniformity of `map_get`/`map_keys` | Workflow engine kept contexts as raw maps solely because parsed JSON couldn't be updated | landed (0.27) |
| 2 | **`for` pattern destructuring** — `for (i, x) in enumerate(xs)`, plus an `entries(m)` builtin so maps iterate as `(key, value)` pairs | `pair[0]`/`pair[1]` indexing in markdown, run_all, and workflow | landed (0.27) |
| 3 | **Strings iterate**: `for ch in "abc"` yields 1-character strings | `str.chars` detours in the regex engine and parser library | landed (0.27) |
| 4 | **`show(v)` builtin** — display rendering: strings bare, everything else as `to_string`. `to_string` keeps its repr form (strings quoted) unchanged | Quote-stripping workarounds in workflow, jsonschema, and the `join` fix | landed (0.27) |

## Tier 2 — control flow and errors

| # | Feature | Grounding | Status |
|---|---|---|---|
| 5 | **`return expr`** — early exit from a function; reuses the `?` unwind machinery | `let mut go = true` loop flags in the template and markdown parsers | landed (0.27) |
| 6 | **Implement `error` declarations** — variants become constructors producing `Err`-matchable values (`match r { Err(NotFound) => ... }`); moves the construct from Reserved to Stable | The webserver's hand-rolled 400/404/500 taxonomy | landed (0.27) |
| 7 | **`break value`** — `loop { ... break x }` as an expression | pairs with `return` | landed (0.27) |

## Tier 3 — stdlib gaps

| # | Feature | Grounding | Status |
|---|---|---|---|
| 8 | **`time` module** — `now_ms`, `monotonic_ms`, `sleep(ms)` | run_all times in whole seconds via `dates.timestamp(dates.now())`; no sub-second clock exists | landed (0.27) |
| 9 | **`fs.walk` / `fs.glob`** | run_all hand-rolled two-level directory discovery over flat `list_dir` | landed (0.27) |
| 10 | **`os.exec` options** — optional third argument `#{ "cwd": ..., "stdin": ..., "env": ... }`; 2-arg form unchanged | run_all wraps every exec in `os.chdir` | landed (0.27) |
| 11 | **`str.fmt("{} of {}", a, b)`** — display-form placeholders | long `+`/`pad_end` chains in every program that prints tables | landed (0.27) |
| 12 | **db transactions** — `db.begin` / `db.commit` / `db.rollback` | the webserver's create-then-select pattern is unsafe under future concurrency | landed (0.27) |

## Tier 4 — the two big lanes

| # | Feature | Grounding | Status |
|---|---|---|---|
| 13 | **OVM coverage expansion**, starting with `&&`/`||` as conditional jumps — today any function containing them falls back to the tree-walker permanently. Then strings, field access, closures. Every expansion lands with tier-agreement tests | verified: the OVM compiler had no And/Or lowering; guards are everywhere post-dogfooding | `&&`/`||` landed (0.27): conditional-jump lowering, 2.2× on guard-heavy hot loops; lane continues (strings, field access, closures) |
| 14 | **`http.serve` keep-alive**, then bounded concurrency — the handler contract is fixed; the execution model grows (as [stability](stability.md) already carves out) | sequential + Connection: close was the original model | keep-alive landed (0.27); bounded worker pool landed (0.29) |

## Tooling track

| # | Feature | Grounding | Status |
|---|---|---|---|
| 15 | **`olang test`** — discover and run `test` blocks across a package in isolation, with reporting | examples now carry self-check `test` blocks; make the pattern first-class | landed (0.28) |
| 16 | **`olang fmt`** — enforce the book's conventions mechanically | the fixed idioms (`= {` bodies, `=>` arms) are formatter-shaped | landed (0.28): whitespace hygiene with an AST-identity safety gate; layout normalization is future work |

## 0.29 — consolidation

An external review (August 2026) scored the language surface and semantic
discipline well but called the platform unevenly finished, and prescribed
consolidation over features. 0.29 addresses its bottom line, item by item.
Version is `0.29.0-dev` until every item lands.

| # | Item | The verified problem | The fix | Status |
|---|---|---|---|---|
| C1 | **Make the struct story real, types explicitly dynamic** | `NeverDeclared { surprise: 42 }` constructs; declared structs accept missing/extra fields — declarations are decoration | Constructing a *declared* struct validates the exact field-name set; an undeclared struct-literal name is an error; anonymous `{ ... }` objects stay free-form. Field *values* stay dynamic, stated once, plainly, in the book | landed (unreleased) |
| C2 | **Finish or rename async; prune lazy scaffolding** | `spawn` evaluates eagerly and wraps a resolved promise; lazy handles are built then immediately forced; memory estimates are placeholders | `spawn` runs on a real thread via the existing thread-safe-clone machinery (pending promise, joined at `await`/`all`/`race`) — with a written fallback: if differential tests show shared-state divergence, keep deterministic semantics and rename the book section honestly. Documented `lazy`/`force` behavior stays; the immediately-forced internal handle plumbing and placeholder estimates are deleted | planned |
| C3 | **Fix package resolution** | The resolver keeps an already-selected version whenever it is ≥ the new requirement's floor — never verifying it *satisfies* the requirement; the `Conflict` error is unreachable | Selection re-verifies every requirement (in-loop and at fixpoint); incompatible ranges produce `Conflict` naming the package and both requirements; MVS tests cover both the conflict and correct-floor cases | landed (unreleased) |
| C4 | **Prune placeholder subsystems** | `sum` hard-codes parallelism at ≥5 elements, bypassing configuration; GC/memory accounting carries placeholder estimates; dead scaffolding surrounds the interpreter core | Numeric builtins respect the configured parallel threshold (sequential by default); an enumerate-then-delete audit removes scaffolding that has no observable behavior — suite green after every deletion | landed (unreleased) |
| C5 | **Split the interpreter into semantic components** | `interpreter.rs` is 5,477 lines — evaluation, module system, patterns, operators, and caching in one file | Mechanical zero-behavior split into `src/interpreter/` (core eval, modules, patterns, ops); public API unchanged; move-only commits, full suite as the proof | planned |
| C6 | **One authoritative capability document** | The README still says "v0.23, experimental", "11 modules", and calls `http.serve` a placeholder — while the book documents 18 modules and a working keep-alive server | [Stability](stability.md) becomes the single capability authority; the README is rewritten short and accurate, deferring to the book (its code blocks already run in CI) | planned |

Deliberately out of scope, with reasons:

- **Enforcing `let mut`** — the book documents that every binding is
  assignable and `mut` marks intent; changing that now would break the
  stability contract for a stylistic gain.
- **Pipeline precedence** (`1 + 2 |> f` parses as `1 + f(2)`) — a real wart,
  but the precedence table is documented stable; revisiting it is a
  major-version conversation, recorded here so it isn't forgotten.

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
