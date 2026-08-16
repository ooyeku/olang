# Common pitfalls

Part of [the olang book](README.md) · [Language reference](language.md).

A small number of olang behaviors commonly surprise newcomers, particularly
programmers arriving from Rust, Python, or JavaScript, where the equivalent
construct behaves differently. This chapter collects those behaviors in one
place. Each is intentional; each entry describes the behavior and the idiom
that works with it. The first three — missing map keys, integer division, and
the two operator-precedence rules — account for most early confusion.

## Missing map keys return Unit, not a Result

`map_get` on a key that is not present returns **`Unit`** (`()`), not an
error and not a `Result`. This is the single most common trip-up.

```olang no-run
let m = #{ "a": 1 }
map_get(m, "z")               // => ()  (Unit), not Err, not None
unwrap_or(map_get(m, "z"), 0) // TYPE ERROR: unwrap_or's first arg must be a Result
```

`unwrap_or` is for `Result`, so passing it a `Unit` is a type error. Guard
the lookup with `map_has_key` instead:

```olang
let m = #{ "a": 1 }
let a = if map_has_key(m, "a") => map_get(m, "a") else => 0
let z = if map_has_key(m, "z") => map_get(m, "z") else => 0
println(show(a) + " " + show(z))   // 1 0
```

Contrast this with parsing, which *does* speak `Result` (below): the two
are easy to conflate.

## Integer division truncates; overflow is an error

`/` between two integers does integer (truncating) division. For a
fractional result, at least one operand must be a float. Integer division
or modulo **by zero is a runtime error** (not `NaN`/infinity), and integer
arithmetic is **checked** — overflow raises rather than wrapping.

```olang
println(show(7 / 2))       // 3     (integer division)
println(show(7.0 / 2.0))   // 3.5   (float division)
println(show(7 / 2.0))     // 3.5   (one float is enough)
println(show(7 % 3))       // 1     (remainder)
```

## Falsy values: `false`, `0`, `0.0`, `""`, `[]`, `()`

An `if`/`while` condition is normally a boolean from a comparison, but
olang accepts any value and defines truthiness: **`false`, `0`, `0.0`, the
empty string, the empty list/tuple, an empty range, and `Unit` are falsy;
everything else is truthy.** So `if xs => …` *does* test a list for
emptiness — but relying on that reads worse than saying what you mean.

```olang
let xs = []
println(if xs => "has items" else => "empty")   // empty
println(if [1] => "has items" else => "empty")   // has items
```

Note the asymmetry: a bare value is fine in `if`, but `&&`/`||` require
real booleans — `1 && 2` is a type error even though `if 1` is not.

## `&&` and `||` share one precedence level

Unlike C and Rust, `&&` and `||` sit at the **same** precedence and
associate left-to-right. `true || false && false` parses as
`(true || false) && false`, which is `false` — not the `true` you would
get if `&&` bound tighter. **Always parenthesize mixed `&&`/`||`.**

```olang
println(show(true || false && false))     // false — (true || false) && false
println(show(true || (false && false)))   // true  — say what you mean
```

## Ranges bind tighter than `+`/`-`

A range's bounds are parsed *before* arithmetic, so `0..n-1` reads as
`(0..n) - 1` — a range minus an integer, which is a runtime type error.
Parenthesize the bound:

```olang
let xs = [10, 20, 30]
for i in 0..(len(xs) - 1) { print(show(xs[i]) + " ") }   // 10 20
println("")
```

The same care applies to bitwise operators and the pipeline `|>`, which
also bind tighter than arithmetic (`1 << 4 * 2` is `(1 << 4) * 2`, i.e.
`32`). When in doubt, add parentheses; the [operator table](language.md#operators-and-precedence)
has the full ordering.

## Parsing returns a `Result`

Unlike `map_get`, the `str.parse_*` family returns a `Result` you must
consume — `unwrap`, `unwrap_or`, `?`, or `match`.

```olang
println(show(unwrap_or(str.parse_int("42"), 0)))    // 42
println(show(unwrap_or(str.parse_int("oops"), -1))) // -1
match str.parse_float("3.14") {
    Ok(f) => println("got " + show(f)),
    Err(e) => println("bad: " + e)
}
```

## `Result` values can't be compared with `==`

There is no equality over `Ok`/`Err`, so `Ok(1) == Ok(1)` raises rather
than returning `true`. Compare the payloads, or `match`:

```olang
let r = str.parse_int("5")
println(if is_ok(r) && unwrap(r) == 5 => "five" else => "other")   // five
```

Two related equality quirks: struct equality includes the type name, so a
`Point` and a `Coord` with identical fields are **not** equal; and while
`1 == 1.0` is `true`, `[1] == [1.0]` is `false` (a structural compare
treats an int element and a float element as different).

## Closures capture by value, at creation

A lambda snapshots the *values* of the variables it uses when it is
created. Reassigning one of those variables afterward does **not** change
what the closure sees — it kept its own copy.

```olang
let mut n = 1
let get = () => n
n = 99
println(show(get()))    // 1, not 99 — the closure captured n's value
```

This is load-bearing — it is what makes modules, safe parallelism, and
tier promotion sound. So *assigning* to a captured binding from inside a
closure is an error, not a silent no-op: the write could only reach the
closure's own snapshot, so the program is refused before it runs.

```text
cannot assign to 'n': it is captured from an enclosing scope, and functions
capture by value — the outer 'n' would not change. Return the new value, or
hold the state in a cell (`let n = cell(...)`, then `cell.set(n, ...)`)
```

Return the new value where you can. Where you cannot — state updated
from deep in a call chain, or from a callback whose shape is fixed —
use a [cell](stdlib.md#cell--mutable-locations):

```olang
let hits = cell(0)
let record = () => cell.update(hits, (n) => n + 1)
record()
record()
println(to_string(cell.get(hits)))   // 2
```

## Definition order: bodies resolve late, values resolve now

A function *body* may call a function defined **later** in the file — call
resolution happens when the call runs, by which point everything is
defined:

```olang
fn a() = b() + 1        // b is defined below; fine
fn b() = 10
println(show(a()))      // 11
```

But binding a name to a function *value* is eager — the right-hand side is
evaluated immediately, so the name must already exist:

```olang no-run
let g = f               // ERROR: Undefined variable: f  (f is defined below)
fn f() = 1
```

Rule of thumb: you can call downward, but you must *reference a value*
upward. Define helpers you assign to variables before the assignment.

## Indexing: negative wraps, no slicing, maps use `map_get`

`[i]` indexing works on lists, tuples, and strings — and a **negative
index counts from the end** (`xs[-1]` is the last element, as in Python).

```olang
let xs = [10, 20, 30]
println(show(xs[-1]))   // 30
println(show(xs[-2]))   // 20
```

Three things `[]` does *not* do:

- **No slicing.** `xs[1..3]` is a runtime type error — use `take`/`skip`
  (`take(skip(xs, 1), 2)`).
- **Maps are not subscriptable.** `m["k"]` fails; use `map_get(m, "k")`.
- **No index- or field-assignment.** `xs[0] = 5` and `p.x = 5` do not
  parse — build a new value (`map_set`, list concatenation, a fresh struct).

## Empty-collection builtins raise

`head`, `tail`, `min`, `max`, and `average` raise on an empty list rather
than returning a `Result` — guard the length first.

```olang
let xs = [3, 1, 2]
let smallest = if len(xs) > 0 => show(min(xs)) else => "none"
println(smallest)   // 1
```

## `for` needs a real binding — `_` is not one

A `for` loop must bind a name, even one you do not use. `for _ in ...` is
a **parse error**; pick a throwaway name like `i` instead. (`_` *is* a
valid wildcard in `match` and `let` — the asymmetry is real.)

```olang
for i in range(0, 3) { print("x") }   // use `i`, not `_`
println("")
```

## A bare name in a `match` pattern is a binding

In a `match` arm, a lowercase bare name is a **fresh binding that matches
anything** — *unless* it is the name of a declared unit enum variant. So a
mistyped variant name silently becomes a catch-all instead of a no-match,
and later arms become unreachable.

```olang
type Color = enum { Red, Green, Blue }
fn name(c) = match c {
    Red => "red",
    Green => "green",
    other => "other: " + show(other)   // `other` binds; a typo like `Gren` would too
}
println(name(Blue))   // other: Blue
```

## A block's bindings end with the block

Every `{ ... }` introduces a scope, so a `let` inside one is not visible
after it. The block still *evaluates* to its last expression — that is
how you get the value out:

```olang
let y = { let hidden = 41; hidden + 1 }
println(show(y))        // 42 — `hidden` itself is gone
```

The trap is a name you meant to keep. Declare it before the block, and
declare it `mut` if the block writes to it:

```olang
let mut label = "unknown"
if true => { label = "ready" }
println(label)          // ready
```

Writing `if true => { let label = "ready" }` instead would bind and
discard a fresh `label`, and reading it afterward is an error rather
than a silent wrong answer.

## `os.args()` includes the program path

`os.args()` returns the full process argv, with the program (or script)
path at index `0` — real arguments start at `[1]`, exactly like C's
`argv`. The `cli` package's `cli.args()` already drops it; by hand,
`skip(os.args(), 1)`.

```olang no-run
let all = os.args()      // ["/path/to/tool", "arg1", "arg2"]
let args = skip(all, 1)  // ["arg1", "arg2"]
```

## Small syntax reminders

A few things that are easy to forget rather than truly surprising:

- **A block's last expression is its value** — no `return` keyword is
  needed at the end of a function or block.

  ```olang
  fn area(w, h) = { let a = w * h; a }   // `a` is the result
  println(show(area(3, 4)))              // 12
  ```

- **The `=>` of an `if` or `match` arm may start on a new line**, so a long
  condition can wrap cleanly:

  ```olang
  let ok = true
  let msg = if ok
      => "ready"
      else => "waiting"
  println(msg)
  ```

- **No compound assignment.** `x += 1` does not parse; write `x = x + 1`.
- **Escapes work in string literals** — `\n`, `\t`, `\"`, `\\`, and
  `\x1b` for control characters (handy for terminal color) — but **not in
  character literals**: `'\n'` is a parse error, use `"\n"`.
- **Assignment is not a declaration, and plain `let` is immutable.**
  `x = 1` for an unbound `x` is an error, and so is assigning to a
  binding that was not declared `let mut`. Re-declaring (shadowing) with
  a fresh `let` is always allowed and is usually the better fix. See
  [the mutability model](language.md#the-mutability-model).
- **A block's bindings end with the block.** A `let` inside `{ ... }`,
  a loop body, an `if` branch, or a `match` arm is not visible
  afterward. Declare the name before the block — `let mut` if the block
  needs to write to it. See [Scope](language.md#scope).

---

None of these are bugs — they are the consequences of a few simple rules
(Unit for absence, values-not-references in closures, C-style argv,
comparisons that refuse across kinds). Once they are in muscle memory the
language gets out of your way. For the precise semantics behind each, the
[Language Reference](language.md) is the authority; for the standard-library
functions mentioned here, see [the stdlib chapter](stdlib.md).
