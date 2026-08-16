# Types and gradual typing

Part of [the olang book](README.md) · [Language reference](language.md) ·
[Command-line tooling](tooling.md) · [Stability and compatibility](stability.md)

olang is gradually typed. Code without annotations is fully dynamic and
carries no runtime type checks. Every annotation that is written is enforced
in two places: the runtime enforces it at its boundary on every execution
tier, and the static checker (`olang check`, and the language server in an
editor) reports violations it can prove before the program runs. This chapter
describes what annotations mean, where they are enforced, what the checker
proves, and how to adopt types incrementally.

Every plain `olang` block in this chapter is executed by the test suite;
blocks marked `no-run` show programs that fail deliberately.

## Table of contents

- [The idea: annotations are promises](#the-idea-annotations-are-promises)
- [The three rules](#the-three-rules)
- [Runtime enforcement](#runtime-enforcement)
- [The static checker](#the-static-checker)
- [Element types — where the checker goes deeper](#element-types--where-the-checker-goes-deeper)
- [Adopting types gradually](#adopting-types-gradually)
- [What is deliberately not checked](#what-is-deliberately-not-checked)
- [Stability](#stability)

## The idea: annotations are promises

A dynamically typed language allows a program to run without declaring the
types of values. This is convenient during exploration, but it defers the
detection of type errors to runtime, sometimes far from the cause. A
statically typed language detects those errors before the program runs, at
the cost of requiring annotations throughout.

olang takes a gradual position: the author chooses, at each site, whether to
annotate. Unannotated code behaves as it does in any dynamically typed
language. An annotation, wherever it appears, changes the contract from
documentation to enforcement:

```olang
fn label(n: Int) -> String = "#" + to_string(n)
println(label(7))
```

Call `label("seven")` and the runtime refuses at the boundary, on every
tier, with the same message the static checker would have shown you in
the editor before you ran it:

```olang no-run
label("seven")
// Type error: parameter 'n' of label expects Int, got String
```

Annotations may appear on function parameters, return types, `let`
bindings, lambda parameters, and struct fields. The full grammar of
annotation forms — `Int`, `Float`, `String`, `Bool`, `[T]`, `(A, B)`,
`Map<K, V>`, function types, `Result<T, E>`, generics — is in
[the language reference](language.md#type-annotations).

## The three rules

Three rules make enforcement predictable enough to hold in your head:

1. **Unannotated code is fully dynamic.** No annotation, no check, no
   cost. A program that never mentions a type runs exactly as it did
   before olang was gradually typed. Adding types to one function judges
   that function's boundaries and nothing else.

2. **Containers check shallowly at runtime.** `List<Int>` promises "a
   List", verified in O(1) at the boundary — the runtime never walks a
   million elements to admit an argument. Element types are the *static
   checker's* concern, and it takes them seriously
   ([below](#element-types--where-the-checker-goes-deeper)).

3. **`Int` and `Float` are strict.** An `Int` does not satisfy a `Float`
   annotation — there is no silent widening, matching how struct fields
   have always been enforced. Write `to_float(n)` when you mean it.

## Runtime enforcement

Each annotation site checks at a specific moment:

- A **parameter** annotation checks each argument at the call boundary.
- A **return** annotation checks the value the function produces.
- A **`let`** annotation checks the bound value at the binding.
- A **struct field** annotation checks at construction.

```olang
type Point = struct { x: Float, y: Float }

fn dist(a: Point, b: Point) -> Float = {
    let dx: Float = a.x - b.x
    let dy: Float = a.y - b.y
    math.sqrt(dx * dx + dy * dy)
}

let d = dist(Point { x: 0.0, y: 0.0 }, Point { x: 3.0, y: 4.0 })
println(to_string(d))
```

Break any of those promises and the error names the site, the
expectation, and what actually arrived:

```olang no-run
fn half(n: Int) -> Int = n / 2
half(3.5)      // Type error: parameter 'n' of half expects Int, got Float

let count: Int = "many"
               // Type error: let binding 'count' expects Int, got String

Point { x: 1, y: 2.0 }
               // Type error: field 'x' of Point expects Float, got Int
```

Details worth knowing:

- **Every tier enforces identically.** The interpreter, the bytecode VM,
  and the JIT produce the same acceptance and the same error text — the
  JIT compiles a function only when it can *prove* the return annotation
  is satisfied, and otherwise leaves it on the checking bytecode path.
  Tier promotion never weakens a promise.
- **Function annotations check callability and arity.** `f: (Int) ->
  Int` verifies the value is callable (a function or a builtin) and can
  be called with exactly the annotation's parameter count — a
  two-parameter lambda passed where `(Int) -> Int` is declared fails at
  the boundary (`expects (Int) -> Int, got a function taking 2
  parameters`), and a lambda with defaults satisfies any arity in its
  range. The parameter and return *types* inside the signature are the
  checker's territory; the callee's own annotations enforce themselves
  when it is called.

- **`Result<T, E>` checks the payload that is present.** Unlike a
  container, a Result holds exactly one payload, so the O(1) discipline
  allows one step more: an `Ok` value checks its payload against `T`,
  an `Err` against `E` — shallowly, one name comparison
  (`Result<List<Int>, E>` checks that an Ok payload is "a List").
  A dishonest side names itself:

  ```olang no-run
  fn parse(s) -> Result<Int, String> = Ok(s)
  parse("x")
  // Type error: return value of parse expects Result<Int, String>, got Ok(String)
  ```

- **Literal types check by value.** An annotation can be a scalar
  literal — the value must equal it. Alone that is a constant
  assertion; in a union it is a lightweight enum, and the error names
  the value that arrived:

  ```olang no-run
  fn set_status(s: "open" | "in-progress" | "done") = s
  set_status("cancelled")
  // Type error: parameter 's' of set_status expects "open" | "in-progress" | "done", got "cancelled"
  ```

- **Unions accept any branch.** `x: Int | String` admits an Int or a
  String and rejects everything else, at the same O(1) cost (branch
  count is annotation-sized). Branches keep their own rules — a
  `Result<Int, String> | Int` union applies the payload check when the
  value is a Result. A union containing an unenforceable branch (e.g. a
  generic parameter) is entirely unchecked rather than wrongly strict —
  rejecting a value the unenforceable branch might have accepted would
  break the gradual contract.

  ```olang no-run
  fn tag(x: Int | String) = show(x)
  tag(true)
  // Type error: parameter 'x' of tag expects Int | String, got Bool
  ```

- **Async functions check the resolved value.** An `async fn` annotated
  `-> Promise<Int, String>` describes the promise the caller receives;
  the runtime enforces `Int` on the value the body resolves to. At
  non-async sites a `Promise<...>` annotation checks its base — a
  parameter declared `Promise<Int>` rejects a plain `Int` — and the
  checker knows that `await` carries the resolved type onward.
- **Generic type parameters are erased.** In `fn first<T>(xs: List<T>)
  -> T`, the `T` has no runtime identity and is never checked — but the
  `List` base still is, and trait *bounds* on generics are enforced as
  they always were.
- **Checks are precomputed.** The work happens once, at declaration; a
  call pays one tag comparison per annotated site. Unannotated
  parameters cost literally nothing.

## The static checker

Runtime enforcement means a broken promise fails *at the boundary*
instead of corrupting state three calls later. The static checker moves
the same failure earlier still — to the terminal, or the editor, at the
moment the mistake is typed:

```bash
olang check
```

`olang check [paths]` walks every `.ol` file, and the same pass runs
inside `olang lsp`, so editors show these as error squiggles as you type
(see [Editors](editors.md)). Its discipline is **no false positives**: a
diagnostic appears only when an annotation is provably dishonest.
Everything it cannot prove stays silent, and unannotated code is never
judged — the checker will never ask you to add types to satisfy it.

What it proves today:

- **Literal arguments against annotated parameters** — `f("hello")` when
  `f` declares `x: Int`.
- **Annotated bindings against known-type values** — `let x: Int =
  "nope"`, and the flow onward: a binding annotated `String` passed to a
  parameter annotated `Int` is flagged at the call.
- **Known return types** — a call to `fn mk() -> String` used where an
  `Int` is expected; and the reverse, a declared return contradicted by
  what the body provably produces (`fn g(x: Int) -> String = x * 2`).
- **Arity of calls to known functions** — too many arguments always, too
  few when the missing parameters have no defaults.
- **Struct literals against their declarations** — mirroring the
  runtime's construction check.
- **Element types inside container literals** — the next section.

Every diagnostic is positioned in the source, rendered with the line and
a caret, and carries one of two labels — because the checker is honest
about its relationship to the runtime:

- *"this would fail at runtime"* — the runtime's own enforcement rejects
  this too, and the message is the runtime's error text **word for
  word**. The diagnostic you see in the editor is the error you would
  have hit.
- *"the annotation's promise is broken here"* — the runtime's shallow
  O(1) checks would let this run, but an annotation in the flagged chain
  is provably false.

`olang check` exits non-zero when anything is found, so it slots
directly into CI. Command details live in
[Tooling](tooling.md#olang-check).

## Element types — where the checker goes deeper

Rule 2 says the runtime checks `List<Int>` shallowly. The checker is
where the element promise gets teeth. Container **literals** decompose
against their annotations element by element, with the path spelled out:

```olang no-run
let xs: List<Int> = [1, "a", 3]
// element 1 of let binding 'xs' expects Int, got String

let m: Map<String, Int> = #{ "a": 1, "b": "two" }
// value for key "b" of let binding 'm' expects Int, got String

let p: (Int, String) = (1, "a", 3)
// let binding 'p' expects a 2-element tuple, got 3 elements

type Config = struct { tags: List<String> }
Config { tags: ["a", 2] }
// element 1 of field 'tags' of Config expects String, got Int
```

Paths compose through nesting — `[[1], [2, "a"]]` against
`List<List<Int>>` reports `element 1 of element 1 of ...`. `Ok`/`Err`
literals decompose the same way (`Ok payload of let binding 'r' expects
Int, got String`), and beyond them the checker follows Results where
the runtime stops: `expr?` is known to carry the Ok payload's type, and
`match` arms narrow — in `match get() { Ok(v) => ..., Err(e) => ... }`
against a known `Result<Int, String>`, `v` is an `Int` and `e` a
`String` inside their arms.

Deep types also flow through the program: annotated bindings and known
return types carry their element structure, so passing a `List<String>`
binding to a parameter declared `List<Int>` is flagged with the full
types (`expects List<Int>, got List<String>`) even though no literal is
in sight.

The conservatism is deliberate and symmetric. An empty list satisfies
any element type; a list whose elements the checker cannot type stays
silent; a generic `List<T>` erases the element promise but keeps the
base. When the checker speaks, the annotation is provably false — there
is nothing to configure and nothing to suppress.

### Match exhaustiveness over literal enums

A union of literals declares exactly which values are admissible — so
when one is matched, the checker knows what "covering every case"
means. A `match` that misses a member gets an advisory warning naming
what's missing; an unguarded binding or `_` arm covers everything and
silences it; and a match whose arms cover *none* of the admissible
values is reported as a runtime error, because it fails on every run:

```olang no-run
fn advance(s: "open" | "active" | "done") = match s {
    "open" => "active",
    "active" => "done"
}
// warning: match is not exhaustive: "done" has no arm — add it or a catch-all
```

Or-patterns count member by member (`1 | 2 => "low"` covers both), and
guarded arms count for nothing — a guard may reject at runtime, so it
proves no coverage. Dynamic scrutinees (`String`, unannotated values)
are never judged.

## Adopting types gradually

The practical playbook, in the order that pays:

1. **Annotate module boundaries first** — the `share`d functions other
   files call. That is where a wrong value travels furthest before
   failing, and where one annotation protects every caller.
2. **Annotate struct fields.** Construction-time checks catch shape
   errors at the source, and struct annotations feed both the runtime
   and the checker's element analysis.
3. **Leave hot inner loops and glue dynamic** if you like — enforcement
   costs one tag comparison per annotated site per call, but the checks
   you don't write cost nothing at all.
4. **Run `olang check` in CI.** It is fast (a parse plus one pass per
   file), silent on dynamic code, and every finding is real.

A program can live indefinitely at any point on this spectrum — fully
dynamic scripts, typed cores with dynamic edges, or annotations
everywhere. All of `examples/` runs with the enforcement on; annotations
that were already honest cost nothing to keep.

## What is deliberately not checked

Knowing the boundaries tells you what an annotation cannot promise:

- **Deep runtime container checks.** The runtime will not walk a
  container at a boundary — that would turn an O(1) call into O(n).
  Shallow at runtime, deep in the checker, is the permanent division of
  labor.
- **Generic type parameters** — erased at runtime, `Unknown` to the
  checker. Their trait bounds are enforced separately.
- **Signature types inside function annotations.** The runtime checks
  callability and arity; a provable mismatch between `(Int) -> Int` and
  a lambda's own annotations (`(s: String) => ...`) is the checker's
  finding, labeled a promise-break.
- **`Promise` payloads at rest.** A pending promise's payload doesn't
  exist yet; the runtime checks the base at non-async sites and the
  resolved value at async returns, and the checker flows the payload
  type through `await`. There is nothing left at rest to check.
- **Result payloads check one level.** The runtime verifies the present
  side's *base* type; structure inside the payload (list elements, a
  nested Result's own payload) is the checker's territory, like every
  other deep promise.
- **Reserved forms** — intersection (`A & B`) annotations parse today
  and gain semantics later
  ([Stability](stability.md#reserved--parses-today-semantics-later)).
  Unions and literal types both left this list in 0.50.
- **The checker never speculates.** No inference across module
  boundaries, no narrowing from `if typeof(x) == ...`, no guesses about
  dynamic code. Anything short of proof is silence.

## Stability

The gradual-typing contract is part of the language's
[stability commitment](stability.md): annotation forms keep parsing,
enforcement semantics are locked by the doc tests in this chapter and
the regression suites, and the checker's no-false-positive discipline is
a fixed rule — new checker capabilities may prove *more*, but a clean
program stays clean. Enforcement landed as the one deliberate breaking
change of 0.48.0, recorded in [CHANGELOG.md](../CHANGELOG.md); programs
whose annotations were honest noticed nothing.

---

Next: [the language reference](language.md#type-annotations) for the
full annotation grammar, [Tooling](tooling.md#olang-check) for the
`olang check` command, or [Editors](editors.md) for the same
diagnostics in your editor.
