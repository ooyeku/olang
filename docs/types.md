# Types and gradual typing

olang is gradually typed. Code without annotations is fully dynamic and
carries no runtime type checks. Every annotation that is written is enforced
in two places: the runtime enforces it at its boundary on every execution
tier, and the static checker (`olang check`, and the language server in an
editor) reports violations it can prove before the program runs. This chapter
describes what annotations mean, where they are enforced, what the checker
proves, and how to adopt types incrementally.

Part of [the olang book](README.md) · [Language reference](language.md) ·
[Command-line tooling](tooling.md)

Blocks marked `no-run` show programs that fail on purpose.

## Table of contents

- [The idea: annotations are promises](#the-idea-annotations-are-promises)
- [The three rules](#the-three-rules)
- [Runtime enforcement](#runtime-enforcement)
- [The static checker](#the-static-checker)
- [Element types — where the checker goes deeper](#element-types--where-the-checker-goes-deeper)
- [Adopting types gradually](#adopting-types-gradually)
- [What is not checked](#what-is-not-checked)

## The idea: annotations are promises

The author chooses, at each site, whether to annotate. Unannotated code
behaves as it does in any dynamically typed language; an annotation,
wherever it appears, is enforced:

```olang
fn label(n: Int) -> String = `#${n}`
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

Three rules make enforcement predictable:

1. **Unannotated code is fully dynamic.** No annotation, no check, no
   cost. Adding types to one function judges that function's boundaries
   and nothing else.

2. **Containers check shallowly at runtime.** `List<Int>` promises "a
   List", verified in O(1) at the boundary — the runtime never walks a
   million elements to admit an argument. Element types are the *static
   checker's* concern
   ([below](#element-types--where-the-checker-goes-deeper)).

3. **`Int` and `Float` are strict.** An `Int` does not satisfy a `Float`
   annotation — there is no silent widening, at a parameter as at a
   struct field. Write `to_float(n)` when you mean it.

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

- **A tuple annotation checks the length.** `(Int, String)` accepts a
  tuple of two elements and refuses one of three (`expects (Int,
  String), got a 3-tuple`); the element types are the checker's, as a
  list's are.
- **`()` accepts only Unit**, so `x: Int | ()` is an Int or nothing, and
  refuses a String.
- **A lambda's return annotation is enforced** as a named function's:
  `(x) -> Int => …` checks what the lambda returns.

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
  generic parameter) is entirely unchecked rather than wrongly strict.

  ```olang no-run
  fn tag(x: Int | String) = show(x)
  tag(true)
  // Type error: parameter 'x' of tag expects Int | String, got true
  ```

- **`Any` takes every value.** A field, parameter or return annotated
  `Any` accepts whatever arrives and is never checked, by the runtime or
  by `olang check`; it composes like any other type (`[Any]`,
  `Map<String, Any>`, `Result<Any, String>`), so a struct can declare a
  free-form field beside its checked ones:
  `type RunResult = struct { ok: Bool, value: Any }`. An unannotated
  parameter means the same; `Any` says it where an annotation is
  required, as in a struct field.
- **Generic type parameters are erased.** In `fn first<T>(xs: List<T>)
  -> T`, the `T` has no runtime identity and is never checked — but the
  `List` base still is, and trait *bounds* on generics are enforced.
- **Checks are precomputed.** The work happens once, at declaration; a
  call pays one tag comparison per annotated site. Unannotated
  parameters cost nothing.

## The static checker

The static checker reports a broken promise before the program runs, in
the terminal or in the editor:

```bash
olang check
```

`olang check [paths]` walks every `.ol` file, and the same pass runs
inside `olang lsp`, so editors show these as error squiggles as you type
(see [Editors](editors.md)). It reports **no false positives**: a
diagnostic appears only when an annotation is provably dishonest.
Everything it cannot prove stays silent, and unannotated code is never
judged.

What it proves:

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
a caret, and carries one of two labels:

- *"this would fail at runtime"* — the runtime's own enforcement rejects
  this too, and the message is the runtime's error text **word for
  word**. The diagnostic you see in the editor is the error you would
  have hit.
- *"the annotation's promise is broken here"* — the runtime's shallow
  O(1) checks would let this run, but an annotation in the flagged chain
  is provably false.

`olang check` exits non-zero when anything is found. Command details
are in
[Tooling](tooling.md#olang-check).

## Element types — where the checker goes deeper

The runtime checks `List<Int>` shallowly; the checker checks the
elements. Container **literals** decompose
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

The checker is conservative. An empty list satisfies
any element type; a list whose elements the checker cannot type stays
silent; a generic `List<T>` erases the element promise but keeps the
base. When the checker speaks, the annotation is provably false — there
is nothing to configure and nothing to suppress.

### Match exhaustiveness over enums and literal unions

A declared `enum` names its variants, so a `match` whose scrutinee is known
to be that enum (an annotated parameter or binding, or a value built from
one of its constructors) gets the same advisory warning when a variant has
no arm, for example ``match over `Sky` is not exhaustive: Rain has no arm``.

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

A program can stay at any point on this spectrum: fully dynamic
scripts, typed cores with dynamic edges, or annotations everywhere.

## What is not checked

- **Deep runtime container checks.** The runtime does not walk a
  container at a boundary; element types are the checker's.
- **Generic type parameters** — erased at runtime, `Unknown` to the
  checker. Their trait bounds are enforced separately.
- **Signature types inside function annotations.** The runtime checks
  callability and arity; a provable mismatch between `(Int) -> Int` and
  a lambda's own annotations (`(s: String) => ...`) is the checker's
  finding, labeled a promise-break.
- **Result payloads check one level.** The runtime verifies the present
  side's *base* type; structure inside the payload (list elements, a
  nested Result's own payload) is the checker's territory, like every
  other deep promise.
- **Intersections** — `A & B` annotations are not accepted: writing
  one is a parse error, not an unenforced promise.
- **The checker never speculates.** No inference across module
  boundaries, no narrowing from `if typeof(x) == ...`, no guesses about
  dynamic code. Anything short of proof is silence.

---

Next: [the language reference](language.md#type-annotations) for the
full annotation grammar, [Tooling](tooling.md#olang-check) for the
`olang check` command, or [Editors](editors.md) for the same
diagnostics in your editor.
