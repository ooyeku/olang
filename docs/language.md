# Language reference

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Standard library reference](stdlib.md) ·
[Packages and dependencies](packages.md) ·
[Architecture and internals](internals.md) ·
[Stability and compatibility](stability.md)

This chapter is the complete reference for the olang language: every
construct, its syntax, and its behavior, each with a runnable example. Code
blocks marked `olang` are executed by the test suite
(`tests/doc_examples_test.rs`) on every change, so the reference stays
consistent with the interpreter.

## Table of contents

1. [Source structure](#source-structure)
2. [Values and runtime types](#values-and-runtime-types)
3. [Literals](#literals)
4. [Variables and assignment](#variables-and-assignment)
5. [Operators and precedence](#operators-and-precedence)
6. [Strings](#strings)
7. [Collections](#collections)
8. [Control flow](#control-flow)
9. [Pattern matching](#pattern-matching)
10. [Functions](#functions)
11. [Pipelines](#pipelines)
12. [User-defined types](#user-defined-types)
13. [Traits](#traits)
14. [Error handling](#error-handling)
15. [Concurrency](#concurrency)
16. [Modules and sharing](#modules-and-sharing)
17. [Testing](#testing)
18. [Type annotations](#type-annotations)
19. [Appendix: keywords and grammar](#appendix-keywords-and-grammar)

## Source structure

An olang program is a sequence of statements. **Newlines separate
statements** — there is no required terminator. A semicolon `;` is accepted
as an optional separator, and comments run from `//` to the end of the line.

```olang
// A complete program: three statements on three lines.
let greeting = "hello"
let audience = "world"   // trailing comments are fine
println(greeting + ", " + audience)
```

Statements are one of: a declaration (`let`, `fn`, `type`, `trait`, `impl`,
`error`, `test`), a module statement (`use`, `share`), or an expression.
Expressions evaluated at the top level are allowed and their value is
discarded (the last one is the program's result in the REPL).

Identifiers start with a letter and continue with letters, digits, and
underscores: `total`, `user_name`, `isValid2`. Identifiers cannot be
[keywords](#appendix-keywords-and-grammar).

olang is expression-oriented: `if`, `match`, blocks, and function bodies all
produce values.

## Values and runtime types

olang is dynamically typed at runtime. Every value has a type name that
`typeof` reports:

| Value | `typeof` |
|---|---|
| `42` | `Int` |
| `3.14` | `Float` |
| `"text"` | `String` |
| `true` | `Bool` |
| `[1, 2]` | `List` |
| `(1, 2)` | `Tuple` |
| `#{ "k": 1 }` | `Map` |
| `{ x: 1 }` | `Object` (anonymous) or the struct's type name |
| `1..5` | `Range` |
| `() => 1` | `Function` |
| `Ok(1)` / `Err("e")` | `Result` |
| unit / JSON `null` / missing | `Unit` |

```olang
println(typeof(42) + " " + typeof(3.14) + " " + typeof("hi") + " " + typeof(true))
println(typeof([1]) + " " + typeof((1, 2)) + " " + typeof(#{ "k": 1 }))
```

olang is gradually typed: [type annotations](#type-annotations) are
optional, and every annotation you write is enforced at runtime.
Unannotated code is fully dynamic. The full story — enforcement,
the static checker, adoption strategy — has
[its own chapter](types.md).

## Literals

### Integers

Decimal, binary (`0b`), octal (`0o`), and hexadecimal (`0x`), with optional
sign and `_` separators for readability:

```olang
println(to_string(1_000_000))
println(`${0xff} ${0b1010} ${0o17}`)
println(to_string(-42))
```

Integers are 64-bit signed. Overflow in arithmetic is a runtime error, not a
silent wraparound.

### Floats

A float literal is digits with a decimal point, an exponent, or both —
`3.14`, `1e20`, `2.5e-3`, `1E+6`. Underscores group digits as they do in
integers:

```olang
println(to_string(3.14))
println(to_string(1.5e3))     // 1500.0
println(to_string(2.5e-1))    // 0.25
println(to_string(1e20))      // 1e20
```

Printing round-trips: `to_string` of any finite float is itself a valid
float literal that reads back to the identical value, whether it prints
in plain form (`1500.0`) or exponent form (`1e301`).

Floats trap rather than produce `NaN`: `0.0 / 0.0`, `math.sqrt(-1.0)`,
`math.log(-1.0)`, and the other operations whose IEEE result would be
`NaN` raise a runtime error instead. Division by zero is an error for
floats exactly as for integers. One edge is currently outside that
wall: arithmetic that *overflows* — `1e308 * 10.0` — yields `inf`
rather than trapping, and `inf` values propagate IEEE-style from there
(the roadmap records this edge; `inf` and `NaN` are printable but are
not literals).

### Booleans

```olang
let yes = true
let no = false
println(to_string(yes && !no))
```

### Strings

Double-quoted, with escape sequences:

| Escape | Meaning |
|---|---|
| `\"` `\\` `\/` | quote, backslash, slash |
| `\n` `\r` `\t` `\0` `\b` `\f` | newline, return, tab, NUL, backspace, form feed |
| `\xNN` | byte by two hex digits |
| `\uNNNN`, `\u{...}` | Unicode scalar |

```olang
println("line one\nline two")
println("say \"hi\"\ttabbed")
println("\u{41}\u{42}\u{43}")   // ABC
```

**Raw strings** (`r"..."`) take no escapes:

```olang
println(r"C:\path\no\newlines")
```

**Template strings** (backticks) interpolate expressions with `${...}`.
Like raw strings, they take no escape sequences — `\n` inside backticks
is a backslash and an `n`, not a newline — and an interpolation cannot
contain another template string. Control characters come from a
double-quoted string, and a nested interpolation is written by binding
the inner template to a name first:

```olang
let n = 6
println(`${n} times 7 is ${n * 7}`)
println("line one\n" + `line ${n}`)
```

**Character literals** (`'a'`) are one-character strings — olang has no
separate character type:

```olang
let c = 'x'
println(typeof(c) + " " + c)
```

## Variables and assignment

`let` binds a name to a value. A plain `let` binding is **immutable**:
assigning to it is an error. Add `mut` when the name must be
reassigned:

```olang
let fixed = 10
let mut counter = 0
counter = counter + 1
println(to_string(fixed + counter))
```

Assignment is not a declaration. A bare `counter = 1` for a name that
was never bound is an error, not an implicit `let` — the two mistakes
this rules out (a typo silently creating a new variable, and a write
that was meant to update something else) are among the most expensive
in a dynamic language. Both errors are reported before the program
runs, so they cannot hide behind a branch that only executes in
production:

```text
cannot assign to 'total': it is not declared in this scope. Declare it
first with `let total = ...`, or `let mut total = ...` if it needs to change
```

```text
cannot assign to 'n': it is not declared mutable. Declare it with
`let mut n = ...`, or bind a new value with `let n = ...` to shadow it
```

The second message names the alternative deliberately. Shadowing — a
fresh `let` of the same name — is always available and is often the
better answer, because it produces a new binding rather than a mutable
one:

```olang
let raw = "  42 "
let raw = str.trim(raw)     // a new binding, not a mutation
println(raw)
```

### The mutability model

It is worth being precise about what assignment does, because olang
separates two things that many languages fuse: *bindings* are
reassignable when declared `mut`, *values* are never mutable.

- A **binding** is a name-to-value association. Assignment (`counter =
  counter + 1`) points the name at a different value. This is allowed
  when the binding was declared `let mut` — and always for function
  parameters, which are the function's own locals initialized from the
  arguments: rebinding one never touches the caller. Parameter rebinding
  is also the collections' calling convention (`h = heap.push(h, ...)`
  inside a structure's own operations), where a shadow would pin a
  second reference to the handle and cost the in-place optimization.
- A **value** — a list, map, struct, string — is immutable. No
  operation modifies a value in place; operations like `map_set`,
  list `+`, and `str.replace` build and return *new* values, sharing
  structure with the old ones where possible (which keeps the copies
  cheap).

The consequence: aliasing is always safe. Two names bound to the same
list can never observe each other's "changes", because there is no such
thing as changing a list — only rebinding a name to a new one:

```olang
let a = [1, 2]
let mut b = a
b = b + [3]
println(to_string(a))   // [1, 2] — a never changes
println(to_string(b))   // [1, 2, 3]
```

This one property underwrites much of the language: closures can
snapshot their environment cheaply, `spawn` and `par_map` can hand
values to other threads without locks, and the execution tiers can
share values by reference without defensive copies.

Assignment (`name = expr`) updates an existing binding — or creates one if
the name is unbound. Prefer `let` for first bindings; it reads better and
survives future tightening of this rule (see [Stability](stability.md)).

An initializer is optional; an uninitialized binding is `Unit` until
assigned. An uninitialized `let` must end its statement — a forgotten `=`
(as in `let scores #{ "ada": 99 }`) is a parse error, not a silent
Unit binding:

```olang
let pending
println(typeof(pending))   // Unit
```

`()` is also a literal, which is how a Unit is written down rather than
arrived at. It is the value a function with nothing to return gives
back, what `map_get` yields for a missing key, and what a null becomes
when it crosses in from JSON or a data column — so being able to write
it is what makes those comparable:

```olang
let nothing = ()
println(to_string(nothing == ()))                  // true
println(to_string(map_get(#{ "a": 1 }, "zz") == ()))   // true
```

### Destructuring

`let` patterns destructure tuples and lists, with `...rest` capturing a
list's remainder:

```olang
let (x, y) = (3, 4)
let [first, second] = [10, 20]
let [head, ...tail] = [1, 2, 3, 4]
println(`${x + y + first + second + head} tail: ${tail}`)
```

Struct patterns work in `let` too:

```olang
type Point = struct { x: Int, y: Int }
let p = Point { x: 1, y: 2 }
let Point { x, y } = p
println(to_string(x + y))
```

### Scope

Every block introduces a scope. A `let` inside a block — a function or
lambda body, a `match` arm, a loop body, an `if` branch, or a bare
`{ ... }` — is gone when the block ends, as is a `for` loop's iteration
variable. Referring to such a name afterward is an error.

```olang
fn f() = {
    let inner = 5      // scoped: gone when f returns
    inner
}
println(to_string(f()))

for i in 0..3 { }      // i is scoped to the loop
let msg = match 1 { n => "arm bindings are scoped too" }
println(msg)

{
    let staged = 10    // scoped to this block
    println(to_string(staged))
}
```

Reading `staged` after that block is an error: `'staged' is not in scope
here`. Assignment reaches outward, though — a block may write to a
mutable binding declared outside it, which is how a loop accumulates:

```olang
let mut total = 0
for n in [1, 2, 3] {
    total = total + n   // `total` is declared outside, and is `mut`
}
println(to_string(total))
```

Shadowing is allowed and idiomatic: a new `let` for the same name binds
a fresh variable from that point on, which keeps transformation
pipelines readable without inventing `x2`, `x3` names:

```olang
let text = "  Hello  "
let text = str.trim(text)
let text = str.to_lower(text)
println(text)
```

### Closures capture by value

A lambda (or nested `fn`) closes over the bindings it references, and it
captures them **by value**: the closure carries a snapshot of the environment
as it was at the moment the closure was created. Two rules follow from this,
and both affect how olang programs are structured.

**Rule one: later rebinding does not reach into a closure.**

```olang
let base = 10
let add_base = (n) => n + base
let base = 99                 // rebinding does not affect the closure
println(to_string(add_base(5)))   // 15
```

**Rule two: a closure cannot assign to a binding it captured.** Since
the write could only ever reach the closure's own snapshot, it is
rejected rather than performed:

```text
cannot assign to 'clicks': it is captured from an enclosing scope, and
functions capture by value — the outer 'clicks' would not change. Return
the new value, or hold the state in a cell (`let clicks = cell(...)`,
then `cell.set(clicks, ...)`)
```

The rule is about the *binding*, not about closures being read-only: a
closure reads captured values freely, and writes its own locals and
parameters freely. Only a write that would cross the boundary outward
is refused.

The same rule governs a [`par for`](#par-for--parallel-iteration) body,
for the same reason and with a different fix: each worker thread runs
the body against its own snapshot of the environment, so a write to an
enclosing binding is refused there too. A cell does not help across that
boundary — cells are thread-confined — so results come back as values
from `par_map` or over a channel.

This is a deliberate design, not a missing feature, and three major
pieces of the language rest on it:

- **Parallelism is safe by construction.** `spawn`, `par_map`, and
  `par for` hand closures to other threads. Because a closure owns an
  immutable snapshot, no thread can see another's writes — there is
  nothing shared to race on, and no locks exist in the language.
- **Modules are self-contained.** An exported function closes over its
  module's complete scope at load time; nothing a caller does can
  reach in and alter what the module's functions see.
- **The execution tiers can optimize.** A snapshot fixed at creation
  time is a compile-time constant to the bytecode compiler and the
  JIT; capture-by-reference would forbid most of what makes tiered
  execution fast.

**Programming without shared mutable state.** The idiom that replaces
"mutate an outer variable from inside a callback" is: *make the data
flow through returns*. Accumulate with `fold`, transform with `map`,
and let each function return the new state:

```olang
let events = ["add", "add", "remove", "add"]
let count = events |> fold(0, (n, e) =>
    if e == "add" => n + 1 else => n - 1)
println(to_string(count))   // 2
```

When a program is *built around* callbacks — a browser frontend is the
clearest case — the same principle scales up to an architecture: keep
no state in the program at all, and treat an external store as the one
source of truth. An event handler that tried `let mut items = [...]`
would lose every write to its own snapshot; instead, each handler asks
the authority (a server, the DOM) for current state, computes, and
writes back:

```olang no-run
// The stateless frontend pattern (see the dom chapter of the stdlib).
// State lives on the server; every handler re-renders from it.
fn render(items) =
    dom.set_html(dom.query("#rows"), items |> map(row_html) |> join(""))

fn reload() =
    dom.fetch("GET", "/api/items", "", (resp) => {
        render(map_get(unwrap(json.parse(resp)), "items"))
    })

dom.on(dom.query("#add"), "click", (id) => {
    dom.fetch("POST", "/api/items", new_item_body(), (resp) => reload())
})
reload()
```

No handler holds state, so capture-by-value costs nothing — and the
architecture that falls out (event → request → re-render) is the same
one large frameworks arrive at deliberately. The
[dom chapter](stdlib.md#dom--the-browser) develops this pattern in
full with a working application.

### Cells: the one mutable location

Returning the new value is the right answer most of the time. It is a
poor one when the state is updated from deep inside a call chain, or
from a callback whose signature you do not control — threading an
accumulator through six functions that have no other interest in it
obscures what each of them is for.

A **cell** is the escape hatch: a value whose contents can be replaced
in place.

```olang
let total = cell(0)
let add = (n) => cell.update(total, (t) => t + n)
for n in [1, 2, 3, 4] { add(n) }
println(to_string(cell.get(total)))   // 10
```

`cell(v)` makes one; `cell.get(c)` reads it; `cell.set(c, v)` replaces
the contents; `cell.update(c, f)` applies `f` to the current value,
stores the result, and returns it. (`cell(v)` and `cell.new(v)` are the
same function — the module is callable, so the common case reads as a
noun.)

A cell is a *location*, not a value. Two cells holding equal contents
are not equal, and binding one to a second name aliases the same
location rather than copying it:

```olang
let a = cell(1)
let b = a
cell.set(b, 5)
println(to_string(cell.get(a)))       // 5 — one location, two names
println(to_string(cell(1) == cell(1)))  // false — different locations
```

**A cell belongs to the thread that created it.** Reading or writing
one from another thread is an error, and sending one through a channel
is refused at the send. This is what keeps the guarantee that made
capture-by-value worth its cost in the first place: two threads still
cannot reach the same mutable location, so the absence of data races
remains structural rather than a matter of discipline. A cell created
*inside* a task, and used only there, is perfectly ordinary — the rule
is about crossing, not about tasks.

```olang
let readings = cell([])
let worker = spawn {
    // A cell made in here belongs in here.
    let local = cell(0)
    for n in [1, 2, 3] { cell.update(local, (t) => t + n) }
    cell.get(local)
}
cell.set(readings, [task.join(worker)])
println(to_string(cell.get(readings)))   // [6]
```

Finally, `cell.update(c, f)` refuses re-entrant access: touching the
same cell from inside `f` is an error rather than a write that `f`'s
return value would silently overwrite. Compute the new value from the
argument instead.

Reach for a cell when the alternative is genuinely worse. Most olang
programs need none.

## Operators and precedence

From loosest to tightest binding:

| Level | Operators | Notes |
|---|---|---|
| 1 | `\|\|` | logical or, short-circuit |
| 2 | `&&` | logical and, short-circuit |
| 3 | `==` `!=` `<` `<=` `>` `>=` | comparison |
| 4 | `\|>` | pipeline |
| 5 | `..` `..=` | ranges |
| 6 | `+` `-` | additive |
| 7 | `*` `/` `%` | multiplicative |
| 8 | `&` `\|` `^` `<<` `>>` | bitwise |
| 9 | `-` `!` (prefix) | unary |
| 10 | `f(x)` `.field` `[i]` `?` | call, access, index, try |

All binary operators are **left-associative**, and comparisons do not
chain (`a < b < c` is a type error — write `a < b && b < c`). The
ordering follows the conventional readings:

- **`&&` binds tighter than `||`.** `a || b && c` is `a || (b && c)`,
  as in C, Rust, and Python.
- **Arithmetic feeds a pipeline, and a pipeline feeds a comparison.**
  `x + 1 |> f` pipes the sum — it is `f(x + 1)` — while
  `xs |> len == 3` compares the pipeline's result. (This is the
  pipeline placement Elixir uses.)
- **Range bounds are arithmetic.** `0..n-1` is `0..(n-1)`, so the
  everyday loop bound needs no parentheses.
- **Bitwise (8) binds tighter than arithmetic.** `1 << 4 * 2` is
  `(1 << 4) * 2` — parenthesize mixed shift/arithmetic expressions.

```olang
fn double(x) = x * 2
println(to_string(true || false && false))   // true: && binds tighter
let n = 5
println(to_string(2 + 1 |> double))          // 6: the sum is piped
let mut hits = 0
for i in 0..n-1 { hits = hits + 1 }
println(to_string(hits))                     // 4: 0..(n-1)
```

When in doubt, parenthesize — especially comparisons feeding `&&`, which
read best fully grouped: `(a >= lo) && (a <= hi)`. See
[Common Pitfalls](pitfalls.md) for worked examples.

> **History.** Before 0.69, `&&` and `||` shared one level, and `|>` and
> ranges bound *tighter* than arithmetic — `0..n-1` was a runtime error
> and `x + 1 |> f` piped only the `1`. Re-ordering them to the readings
> above was the third and final deliberate pre-1.0 breaking change
> ([Stability](stability.md)); code that parenthesized mixed operators,
> as this book always advised, is unaffected.

### Evaluation order

Evaluation is strict (arguments are evaluated before a call runs) and
proceeds **left to right** everywhere: binary operands, call arguments,
and the elements of list, tuple, and map literals all evaluate in
source order. The exceptions are the constructs whose entire purpose is
*not* evaluating something: `&&`/`||` skip the right operand when the
left settles the answer, and `if`/`match` evaluate only the taken arm.

```olang
fn side(tag, v) = {
    println(tag)
    v
}
let sum = side("left", 1) + side("right", 2)     // prints left, right
let xs = [side("first", 10), side("second", 20)] // prints first, second
println(to_string(sum + xs[0] + xs[1]))
```

A pipeline `a |> f(b)` evaluates like the call it desugars to,
`f(a, b)`.

### Arithmetic

`/` on two integers is integer division; `%` is remainder. Mixing an `Int`
and a `Float` produces a `Float`:

```olang
println(`${7 / 2} ${7 % 3} ${7.0 / 2}`)
println(to_string(2 + 0.5))
```

### Comparison and equality

All comparison operators work on numbers (mixing Int/Float) **and on
strings**, which order lexicographically by Unicode scalar value:

```olang
println(to_string("apple" < "banana"))
let c = "7"
println(to_string((c >= "0") && (c <= "9")))   // character-range check
```

Equality is **structural** for lists, tuples, maps, structs, and enum
values — two values are equal when they have the same shape and equal
parts, not when they are the same object (olang has no object identity
to observe):

```olang
println(to_string([1, 2] == [1, 2]))
println(to_string((1, "a") == (1, "a")))
println(to_string(#{ "a": 1 } == #{ "a": 1 }))
```

The precise rules:

- **Numbers compare numerically across Int and Float** at the operator:
  `1 == 1.0` is `true`. Inside a *structural* comparison, however, kinds
  are strict — `[1] == [1.0]` is `false`, because the elements are an
  Int and a Float.
- **Struct equality includes the type name**: two structs with the same
  fields but different declared types are not equal.
- **Comparing unrelated kinds is a type error, not `false`** — `1 ==
  "1"` and `true == 1` raise rather than quietly answering. Convert
  explicitly (`to_string`, `to_int`) when you mean a cross-type
  comparison.
- **The one exception is Unit.** `x == ()` and `x != ()` answer for
  *every* value — false and true respectively unless `x` is Unit.
  Unit is the absence value (a missing map key, an absent substring, a
  JSON null), so "is this nothing?" must be askable about a value that
  turned out to be present; requiring the answer to already be known
  before asking would defeat the question. Ordering against Unit
  (`x < ()`) is still a type error.
- **`Result` values are not compared with `==`.** Match on
  `Ok(v)`/`Err(e)` (or test with `is_ok`/`is_err`) and compare the
  payloads — the pattern is clearer than a comparison would be, and it
  is the supported form.
- Within a structural comparison, a kind mismatch between elements makes
  the values unequal (`false`) rather than raising.

```olang
type A = struct { x: Int }
type B = struct { x: Int }
println(to_string(A { x: 1 } == A { x: 1 }))   // true
println(to_string(A { x: 1 } == B { x: 1 }))   // false: type names differ
println(to_string(1 == 1.0))                   // true: numeric comparison
```

### Logical operators

`&&` and `||` **short-circuit**: the right operand is evaluated only when
the left does not settle the result. This makes bounds-check guards safe:

```olang
let xs = [1, 2, 3]
let i = 5
let ok = (i < len(xs)) && (xs[i] > 0)   // xs[5] is never evaluated
println(to_string(ok))
```

`!` negates a boolean.

### Bitwise operators

On integers: and `&`, or `|`, xor `^`, shifts `<<` `>>`:

```olang
println(`${6 & 3} ${6 | 3} ${6 ^ 3}`)
println(`${1 << 4} ${32 >> 2}`)
```

### String and list `+`

`+` concatenates two strings and concatenates two lists. Mixing a number
and a string is a type error, not a silent coercion — convert the number
with `to_string(...)` first:

```olang
println(`count: ${42}`)
println(to_string([1, 2] + [3]))
let mut acc = []
acc = acc + ["grown"]        // the idiomatic append
println(to_string(acc))
```

### Ranges

`a..b` is half-open, `a..=b` inclusive. Ranges are values and iterate:

```olang
let mut sum = 0
for i in 1..4 { sum = sum + i }        // 1+2+3
for i in 1..=4 { sum = sum + i }       // 1+2+3+4
println(to_string(sum))
```

## Strings

Strings are immutable. Global builtins cover the basics (`len`, `contains`,
`starts_with`, `ends_with`, `split`, `join`), and the `str` module has the
full toolkit — `str.substring`, `str.index_of`, `str.char_at`, `str.trim`,
`str.pad_start/pad_end`, `str.to_upper/to_lower`, `str.replace`,
`str.parse_int/parse_float`, and more (see the
[stdlib reference](stdlib.md#str--strings)).

```olang
let s = "  olang  "
println("[" + str.trim(s) + "]")
println(str.substring("hello", 1, 4))          // ell
println(to_string(str.index_of("abc", "z")))   // () when absent — test with != ()
println(join(split("a,b,c", ","), " + "))
```

Strings iterate by character with `for` (each a 1-character string), and
`str.chars` gives the same characters as a list:

```olang
for c in "abc" { print(c + ".") }
println("")
println(to_string(str.chars("abc")))
```

### Codepoints and graphemes

Indexing counts Unicode codepoints: `len`, `str.length`, `str.char_at`,
`str.substring`, and iteration all agree on that unit, and it never
loses information. A *visible* character can span several codepoints —
an emoji with a skin-tone modifier, a letter with a combining accent, a
flag — and `str.graphemes` is the view for those cases: the string's
grapheme clusters (UAX #29) as a list of strings. Count what a reader
sees with `len(str.graphemes(s))`, take the nth visible character by
index, slice with list operations and join back with `str.join(gs,
"")`. `str.reverse` works on graphemes, because reversing is a visual
request — clusters stay whole.

```olang
println(to_string(len(str.graphemes("abc"))))     // 3
println(str.join(str.graphemes("olang"), "."))    // o.l.a.n.g
println(str.reverse("abc"))                       // cba
```

## Collections

### Lists

Ordered, heterogeneous, immutable values. Index with `[i]` (0-based),
concatenate with `+`, and transform with the higher-order builtins:

```olang
let xs = [3, 1, 4, 1, 5]
println(`${xs[0]} len=${len(xs)}`)
println(to_string(sort(xs)))
println(to_string(xs |> map((x) => x * 2) |> filter((x) => x > 4)))
println(to_string(xs |> fold(0, (acc, x) => acc + x)))
```

Multi-line list literals may end with a trailing comma:

```olang
let langs = [
    "olang",
    "rust",
]
println(to_string(len(langs)))
```

### Subscripting beyond the built-in collections

`[i]` is not limited to the types above. A native value — one supplied by
a module implemented in Rust — may define what its own subscript means,
and the data stack's do:

```olang
let sales = ods.read_csv("region,amount\neast,25.5\nwest,320.0\n")
println(to_string(ods.to_list(sales["amount"])))   // a Frame takes a column name
println(to_string(sales["amount"][1]))             // a Series takes a position
```

A value that defines no subscript reports that it cannot be indexed,
naming its type. Subscripting is always read-only: there is no
`value[k] = x` for a native value, because these values are immutable and
an assignment that silently produced a copy would be a trap.

The rule such a type is expected to follow is the one the Frame follows:
**one key, one reading.** A Frame accepts a String (a column) and a Bool
mask (rows), which are disjoint by type, and refuses a row position
outright rather than adding a third meaning. See
[The Data Stack](ods.md#reaching-a-column).

### Tuples

Fixed-shape groups, indexed positionally:

```olang
let pair = (1, "one")
println(to_string(pair[0]) + " is " + pair[1])
```

Tuples destructure in `let`, `for`, and `match`, and are what `zip`,
`enumerate`, `entries`, and `group_by` produce — so paired data flows
naturally:

```olang
let (lo, hi) = (3, 9)
println(to_string(hi - lo))

for (name, score) in zip(["ada", "bob"], [99, 82]) {
    println(`${name}: ${score}`)
}

fn quadrant(p) = match p {
    (0, 0) => "origin",
    (x, y) if x > 0 && y > 0 => "first",
    _ => "elsewhere"
}
println(quadrant((2, 5)) + " " + quadrant((0, 0)))
```

### Maps

`#{ key: value }` builds a hash map. Keys are strings (integers, floats, and
booleans are converted to their string form). Maps are immutable — `map_set`
returns a new map:

```olang
let scores = #{ "ada": 99, "bob": 82 }
println(to_string(map_get(scores, "ada")))
let scores2 = map_set(scores, "cyn", 91)
println(`${map_len(scores)} then ${map_len(scores2)}`)
println(`${map_has_key(scores, "cyn")}/${map_has_key(scores2, "cyn")}`)
println(to_string(sort(map_keys(scores2))))
```

`map_get` on a missing key returns `Unit` (test presence with
`map_has_key`). The `map_*` accessors also read **any struct-like value** —
anonymous objects, structs, and parsed JSON objects — so dynamic key access
works uniformly:

```olang
let obj = { a: 1, b: 2 }
let key = "b"
println(to_string(map_get(obj, key)))
```

### Anonymous objects

`{ field: value }` builds an object with named fields, accessed with `.`:

```olang
let user = { name: "ada", age: 36 }
println(`${user.name} is ${user.age}`)
```

Objects and maps differ: object fields are identifiers accessed with dot;
map keys are arbitrary strings accessed with `map_get`. A `{ ... }` with
`identifier: value` pairs is an object; `#{ ... }` is always a map.

**Disambiguation note:** `{ ... }` containing statements is a
[block](#blocks-are-expressions); `{ name: expr }` is an object literal.

## Control flow

### `if` expressions

`if` is an expression. Its arms use `=>` and produce values; `else` is
optional (a false condition without `else` yields `Unit`). Chains use
`else if`:

```olang
let x = 15
let size = if x < 10 => "small"
    else if x < 100 => "medium"
    else => "large"
println(size)
```

Arms can be blocks:

```olang
let score = 85
let grade = if score >= 90 => {
    "A"
} else => {
    "B or below"
}
println(grade)
```

### Blocks are expressions

A block evaluates its statements and yields the last expression's value:

```olang
let result = {
    let a = 6
    let b = 7
    a * b
}
println(to_string(result))
```

`a` and `b` are gone after the block; `result` is what survives. That is
the idiom for a multi-step computation whose intermediates should not
outlive it — a block is the smallest unit of encapsulation the language
has, smaller than a function and free of a call.

### `while`

```olang
let mut n = 1
while n < 100 { n = n * 2 }
println(to_string(n))   // 128
```

### `for`

Iterates lists, tuples, ranges, and strings (by character), binding each
element:

```olang
let mut total = 0
for x in [10, 20, 30] { total = total + x }
for i in 0..3 { total = total + i }
println(to_string(total))   // 63

for ch in "abc" { print(ch) }
println("")
```

The binding may be a tuple of names, destructuring each element — the
natural shape for `enumerate` (index pairs) and `entries` (key/value pairs
of a map or object):

```olang
for (i, x) in enumerate(["a", "b"]) {
    println(to_string(i) + ": " + x)
}
for (k, v) in entries(#{ "b": 2, "a": 1 }) {
    println(k + " -> " + show(v))      // entries are sorted by key
}
```

### `par for` — parallel iteration

`par for` fans iterations across OS worker threads (one interpreter,
with its own bytecode tier and JIT, per worker) and waits for all of
them — an implicit barrier. It iterates lists, ranges, and strings, with
the same tuple destructuring as `for`. `par` is not a reserved word; it
only means something directly before `for`.

Semantics match `spawn` and `par_map`: the body runs against worker
snapshots. Reading an enclosing binding is how the body gets its inputs,
but **assigning to one is refused before the program runs** — the write
could only reach a snapshot. Use `par for` for real per-element effects
and heavy computation; when you want values back, use `par_map`, or send
them over a [channel](stdlib.md#chan--channels). A `let mut` declared
inside the body is local to one iteration and is unaffected.

If several iterations fail, the error reported is the one the sequential
loop would have hit first. `break` and `return` cannot cross the parallel
boundary; `continue` works within an iteration. The loop evaluates to
Unit.

```text
cannot assign to 'tally': `par for` runs its body on worker threads, each
against its own snapshot of the environment, so the write would be discarded
rather than reaching the outer 'tally'. Produce a value per item and combine
them — `sum(par_map(xs, (x) => ...))` — or send results over a `chan`
```

A [cell](stdlib.md#cell--mutable-locations) is deliberately not the
answer here, unlike the closure case: cells are confined to the thread
that created them, so one made outside the loop is unreachable from a
worker.

Confinement is a property a value carries, not a rule about cells. Any
value holding a mutable location declares itself confined, and every
crossing then refuses it without knowing what it is: reading one from a
`spawn`ed task fails, and `chan.send` refuses to carry it. A streaming
[CSV or JSON-lines reader](ods.md#files-larger-than-memory) is the second
such value — it holds a file position — and it inherited both refusals
without a line of code written for it. To use one across threads, read on
the thread that opened it and send the *rows*.

```olang no-run
par for (i, chunk) in enumerate(chunks) {
    let report = analyze(chunk)
    unwrap(fs.write_file("report-" + show(i) + ".txt", report))
}
```

### `loop`, `break`, `continue`

`loop` repeats forever until `break`; `continue` skips to the next
iteration of any loop:

```olang
let mut n = 0
let mut odds = 0
loop {
    n = n + 1
    if n > 10 => break
    if n % 2 == 0 => continue
    odds = odds + 1
}
println(to_string(odds))   // 5
```

`break value` makes the loop evaluate to that value — the idiomatic
search-until-found shape:

```olang
let mut n = 0
let first_big_square = loop {
    n = n + 1
    if n * n > 50 => break n * n
}
println(to_string(first_big_square))   // 64
```

## Pattern matching

`match` tests a value against arms in order; the first matching pattern's
expression is the result. Arms are separated by commas (a trailing comma is
fine).

```olang
fn describe(n) = match n {
    0 => "zero",
    1 | 2 | 3 => "a few",          // or-patterns
    4..10 => "several",            // range pattern (half-open)
    x if x < 0 => "negative",      // guard
    _ => "many"                    // wildcard
}
println(describe(0) + ", " + describe(2) + ", " + describe(7) + ", " + describe(-4) + ", " + describe(99))
```

### Pattern kinds

| Pattern | Example | Matches |
|---|---|---|
| Literal | `0`, `"yes"`, `true` | that exact value |
| Or | `1 \| 2 \| 3` | any alternative |
| Range | `1..10`, `'a'..'z'` | value within the range |
| Wildcard | `_` | anything, binds nothing |
| Identifier | `x` | anything, binds it as `x` |
| Tuple | `(a, b)` | a 2-tuple, binding parts |
| List | `[a, b]`, `[h, ...t]` | exact shape, or head + rest |
| Enum variant | `Some(v)`, `Node(l, r)` | that variant, binding payloads |
| Result | `Ok(v)`, `Err(e)` | result values |
| Struct | `Point { x, y }` | struct fields (shorthand or `field: pat`) |
| Anonymous struct | `{ kind: k }` | object fields |
| Guard | `pat if cond` | pattern plus condition |

```olang
fn area(shape) = match shape {
    { kind: "circle", r: r } => 3.14159 * r * r,
    { kind: "rect", w: w, h: h } => w * h,
    _ => 0.0
}
println(to_string(area({ kind: "rect", w: 3.0, h: 4.0 })))
```

List patterns with rest:

```olang
fn sum(xs) = match xs {
    [] => 0,
    [head, ...tail] => head + sum(tail)
}
println(to_string(sum([1, 2, 3, 4])))
```

### Identifier patterns vs unit variants

A bare name in a pattern is a **fresh binding** — unless the name is a
*declared unit variant* of an enum, in which case it matches that variant by
equality. This is what makes `match dir { North => ..., South => ... }` work
while keeping ordinary bindings safe:

```olang
type Dir = enum { North, South }
fn flip(d) = match d { North => South, South => North }
println(to_string(flip(North) == South))
```

## Functions

### Declarations

`fn name(params) = expression`. The body is a single expression — which can
be a block, so `fn f(x) = { ... }` is the multi-statement form:

```olang
fn double(x) = x * 2
fn describe(n) = {
    let d = double(n)
    `double of ${n} is ${d}`
}
println(describe(21))
```

Note the `=` before a block body: `fn f(x) { ... }` (without `=`) is not
valid olang.

### Parameters: annotations, defaults, named arguments

Parameters may carry optional type annotations and default values; calls may
pass arguments by name in any order:

```olang
fn greet(name: String, greeting = "hello") = greeting + ", " + name
println(greet("ada"))
println(greet("lin", "hey"))
println(greet(greeting: "yo", name: "sam"))
```

A default is evaluated **at call time, in the function's own scope** —
as if it were the first statement of the body. It sees the function's
closure and every parameter to its left, and it never sees the
caller's locals. Defaults evaluate left to right, once per call that
omits them:

```olang
fn window(lo, hi = lo + 10, label = `${lo}..${hi}`) = label
println(window(5))          // 5..15
println(window(5, 8))       // 5..8
println(window(5, label: "custom"))
```

Parameters without defaults are required; a call must cover them
positionally or by name, and positional arguments may not follow named
ones. Lambdas take defaults with the same rules. Functions with
defaults run on every tier: a compiled call site fills literal
defaults as constants, and any other omitted default is evaluated by
the reference semantics — the two are indistinguishable by
observation, like every other tier boundary.

### Return type annotation

```olang
fn square(x: Int) -> Int = x * x
println(to_string(square(9)))
```

### Lambdas

`(params) => expression` — anonymous functions, the workhorses of the
higher-order builtins. Zero-parameter and block-bodied lambdas work too:

```olang
let inc = (x) => x + 1
let make = () => "made"
let complex = (x) => {
    let y = x * x
    y + 1
}
println(`${inc(1)} ${make()} ${complex(3)}`)
```

Functions are first-class values: store them in variables, lists, maps, and
object fields; pass and return them freely:

```olang
let ops = #{ "inc": (x) => x + 1, "dbl": (x) => x * 2 }
let f = map_get(ops, "dbl")
println(to_string(f(21)))

fn compose(f, g) = (x) => f(g(x))
let add1_then_double = compose((x) => x * 2, (x) => x + 1)
println(to_string(add1_then_double(5)))   // 12
```

### Early return

`return expr` exits the nearest enclosing function (or lambda) immediately
with the value; a bare `return` yields `Unit`. Guards at the top of a
function read naturally:

```olang
fn classify(n) = {
    if n < 0 => return "negative"
    if n == 0 => return "zero"
    "positive"
}
println(classify(-3) + " " + classify(0) + " " + classify(7))

fn first_even(xs) = {
    for x in xs { if x % 2 == 0 => return x }   // escapes the loop too
    -1
}
println(to_string(first_even([3, 5, 8])))
```

`return` outside any function is an error. For Result-returning functions,
[`?`](#the--operator) is usually the better early exit.

### Recursion

Functions can call themselves and each other (declaration order does not
matter for top-level functions):

```olang
fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)
println(to_string(fib(10)))
```

### Nested functions

`fn` declarations may appear inside a function body. A nested function sees
the enclosing function's parameters and locals (captured by value, like a
lambda), is scoped to the body, and may call itself and its sibling nested
functions:

```olang
fn bullet_list(items, marker) = {
    fn line(x) = marker + " " + show(x)       // captures `marker`
    items |> map(line) |> join("\n")
}
println(bullet_list(["read", "write"], "-"))

fn sum_list(xs) = {
    fn go(rest, acc) = match rest {
        [] => acc,
        [h, ...t] => go(t, acc + h)           // self-recursive nested fn
    }
    go(xs, 0)
}
println(to_string(sum_list([1, 2, 3, 4])))
```

Use a nested `fn` where a lambda would need a name — recursive helpers,
or a step function shared by several call sites in the same body.

### Generics and trait bounds

Type parameters parse and are erased at runtime (olang is dynamically
typed); trait bounds are enforced at call time — passing a value whose type
lacks the required trait is a runtime error:

```olang
trait Show { fn show(self) -> String }
type Tag = struct { name: String }
impl Show for Tag { fn show(self) = "#" + self.name }

fn display<T: Show>(x: T) = x.show()
println(display(Tag { name: "olang" }))
```

## Pipelines

`|>` feeds the left value as the **first argument** of the right function
call. Pipelines read as left-to-right data flow and are the idiomatic way to
chain list operations:

```olang
let result = [1, 2, 3, 4, 5, 6]
    |> filter((x) => x % 2 == 0)
    |> map((x) => x * x)
    |> fold(0, (acc, x) => acc + x)
println(to_string(result))   // 4 + 16 + 36
```

The right side may be a bare function name (called with one argument) or a
call with extra arguments (the piped value is prepended):

```olang
fn add(a, b) = a + b
println(to_string(5 |> add(3)))
```

## User-defined types

### Structs

```olang
type Point = struct { x: Int, y: Int }
let p = Point { x: 3, y: 4 }
println(to_string(p.x * p.x + p.y * p.y))
println(typeof(p))   // Point
```

Struct values are immutable; build a new one to "change" a field.

**Construction is validated against the declaration.** A struct literal
must name a declared struct type and supply exactly the declared fields —
a missing or surprise field is an error naming it, and an undeclared
struct-literal name is an error (use an anonymous `{ ... }` object for
free-form records).

**Field values are checked against their declared types.** When a field's
annotation is one the runtime can verify — `Int`, `Float`, `Bool`,
`String`, or a declared struct or enum type — a value of the wrong type is
an error naming the field, the type, what was expected, and what was
supplied. The match is exact: an `Int` value does **not** satisfy a
`Float` field (there is no widening at construction). Fields whose
annotation the runtime cannot reliably check — a generic type parameter, a
list or map, a function type — stay dynamic and accept any value, as do all
fields of an anonymous `{ ... }` object.

```olang no-run
type Point = struct { x: Int, y: Int }
Point { x: 1 }                    // error: missing field 'y'
Point { x: 1, y: 2, z: 3 }        // error: no field 'z'
NeverDeclared { s: 42 }           // error: unknown struct type
Point { x: "hi", y: 2 }           // error: field 'x' of Point expects Int, got String

type Box<T> = struct { value: T }
Box { value: "anything" }         // fine: a generic field stays dynamic
{ x: "anything", y: 2 }           // fine: anonymous objects are free-form
```

### Enums (algebraic data types)

Variants may be bare (unit), carry positional payloads, or both. Declaring
an enum binds each variant name as a constructor:

```olang
type Shape = enum {
    Circle(Float),
    Rect(Float, Float),
    Unknown
}

fn area(s) = match s {
    Circle(r) => 3.14159 * r * r,
    Rect(w, h) => w * h,
    Unknown => 0.0
}
println(`${area(Rect(3.0, 4.0))} ${area(Unknown)}`)
```

Enums are recursive — a variant can hold values of its own type — which is
how trees and other inductive structures are written:

```olang
type Tree = enum { Leaf(Int), Node(Tree, Tree) }
fn total(t) = match t {
    Leaf(n) => n,
    Node(l, r) => total(l) + total(r)
}
println(to_string(total(Node(Node(Leaf(1), Leaf(2)), Leaf(3)))))
```

Generic type parameters on enums (`type Option<T> = enum { ... }`) parse and
are erased at runtime.

When an enum is shared from a module, importing the type also imports its
variant constructors — see [Modules](#modules-and-sharing).

### What is not supported (yet)

Union type *declarations* (`type Id = Int | String`) are not accepted;
discriminated unions are written as enums or as objects with a `kind` field.
Union *annotations* (`x: Int | String`) are not in this list — they have
had semantics since 0.50 ([Types](types.md)). Intersection annotations
(`A & B`) are not accepted at all; writing one is a parse error. See
[Stability](stability.md) for the full list.

## Traits

A trait declares methods; `impl Trait for Type` provides them. Method calls
dispatch on the receiver's runtime type. Traits may supply default method
bodies, which impls can override:

```olang
trait Greet {
    fn name(self) -> String
    fn hello(self) -> String = "hello, " + self.name()   // default method
}

type Robot = struct { id: Int }
type Human = struct {}

impl Greet for Robot { fn name(self) = `unit-${self.id}` }
impl Greet for Human {
    fn name(self) = "friend"
    fn hello(self) = "hey there"                         // override
}

println(Robot { id: 7 }.hello())
println(Human {}.hello())
```

Traits work over enums as well as structs, and a list of different types
sharing a trait dispatches correctly per element:

```olang
trait Area { fn area(self) -> Int }
type Sq = struct { s: Int }
type Rc = struct { w: Int, h: Int }
impl Area for Sq { fn area(self) = self.s * self.s }
impl Area for Rc { fn area(self) = self.w * self.h }

let shapes = [Sq { s: 3 }, Rc { w: 2, h: 5 }]
println(to_string(shapes |> fold(0, (acc, s) => acc + s.area())))
```

`implements(value, "TraitName")` asks whether a value's type implements a
trait:

```olang
trait Show { fn show(self) -> String }
type P = struct {}
impl Show for P { fn show(self) = "p" }
println(to_string(implements(P {}, "Show")))
```

Field access wins over methods: if a struct has a field `value`, `x.value`
is the field even if a trait method of the same name exists.

## Error handling

olang uses `Result` values, not exceptions. `Ok(v)` carries a success,
`Err(e)` an error; functions that can fail return one of the two.

```olang
fn safe_div(a, b) = if b == 0 => Err("division by zero") else => Ok(a / b)

let good = safe_div(10, 2)
let bad = safe_div(1, 0)
println(`${is_ok(good)} ${is_err(bad)}`)
```

### Consuming results

Pattern match, or use the helper builtins:

```olang
fn safe_div(a, b) = if b == 0 => Err("division by zero") else => Ok(a / b)

let msg = match safe_div(10, 3) {
    Ok(v) => `got ${v}`,
    Err(e) => "failed: " + e
}
println(msg)

println(to_string(unwrap(safe_div(8, 2))))          // panics on Err
println(to_string(unwrap_or(safe_div(1, 0), -1)))   // default on Err
println(to_string(unwrap_or_else(safe_div(1, 0), (e) => 0 - len(e))))
println(to_string(result_map(safe_div(6, 2), (v) => v * 10)))
```

### The `?` operator

Inside a function, `expr?` unwraps an `Ok` or returns the `Err` to the
caller immediately — the concise way to propagate failures:

```olang
fn parse_both(a, b) = {
    let x = str.parse_int(a)?
    let y = str.parse_int(b)?
    Ok(x + y)
}
println(to_string(parse_both("2", "40")))
println(to_string(is_err(parse_both("2", "oops"))))
```

### Where failure stops: the boundaries

`Result` covers *expected* failure — the file that might be missing, the
input that might be malformed. A **runtime error** is different: a type
error, a failed assertion, a division by zero. Those are bugs, and olang
does not offer a way to catch one mid-expression. `try`/`catch` used to
look like that mechanism and was not (it destructured `Result`s); it was
removed in 0.65, and `match` and `unwrap_or` say the same thing.

What olang does have is **structural boundaries** — places where a
failing unit is already isolated from the rest of the program, so the
failure becomes a value on the far side without any construct at the
failure site:

| Boundary | A runtime error inside it becomes |
|---|---|
| a spawned task | `Err(e)` from [`task.join`](#spawn-and-taskjoin) |
| an `http.serve` handler | a logged 500; the server keeps serving |
| one `par_map` / `par_filter` element | the error propagates out of the call |

```olang
fn risky(n) = if n > 2 => 1 + "not a number" else => n * 10
let jobs = [spawn risky(1), spawn risky(9)]
println(to_string(jobs |> map((j) => match task.join(j) { Err(e) => -1, v => v })))
```

That is the whole recovery story, and it is deliberate. A supervisor
recovers because the thing it supervises runs somewhere it can watch,
not because it wrapped an expression in a handler. Anywhere else, a
runtime error stops the program — which is what you want from a bug.

### `error` declarations

`error Name { ... }` declares a family of error values: bare variants are
singleton values, and variants with a payload (`Invalid: { msg: String }`)
become constructors taking the payload fields positionally. Wrap them in
`Err(...)` and match them like any enum — this is the structured
alternative to string errors:

```olang
error AppError {
    NotFound,
    Invalid: { msg: String }
}

fn lookup(id) = {
    if id == 0 => return Err(NotFound)
    if id < 0 => return Err(Invalid("id must be positive"))
    Ok("user-" + show(id))
}

fn describe(r) = match r {
    Ok(user) => user,
    Err(NotFound) => "no such user",
    Err(Invalid(msg)) => "bad request: " + msg
}
println(describe(lookup(7)))
println(describe(lookup(0)))
println(describe(lookup(-1)))
```

Error variants are ordinary enum values at runtime: they compare with `==`,
`typeof` reports the declared error type's name, and unit variants match by
name in patterns.

### Runtime errors carry a stack trace

When a program aborts on a genuine runtime error — division by zero,
an out-of-bounds index, an arity mismatch — the report carries the
innermost located statement's span *and the call stack live at that
point*:

```text
  × Runtime error: Division by zero
   ╭─[demo.ol:3:5]
 3 │     x / 0
   ·     ▲
   ╰────

  Call stack (outermost first):
    → outer
    → middle
    → inner
```

The trace is a property of the language, not of a tier: the same
program reports the same span, frames, and message whether it ran
interpreted, on the bytecode VM, or through a JIT deoptimization —
[the execution tiers](ovm.md#correctness-policy) are held to
trace-identical error reporting by the test suite. One shape note:
statements inside block bodies pinpoint the failing statement, while a
single-expression function body (`fn f(x) = ...`) attributes the error
to the nearest enclosing located statement — its call site.

## Concurrency

olang has **one** concurrency model, and it is threads. There are three
pieces, all explicit:

- **`spawn`** starts work on a real OS thread and hands back a task
  handle; `task.join` collects the result.
- **`chan`** is a queue tasks use to send values to each other.
- **`par_map`, `par_filter`, and `par for`** apply a function across a
  collection on a worker pool.

There is no event loop, no scheduler, and no colouring of functions into
sync and async. Any function can be spawned, because a task is a thread
running an ordinary call.

### `spawn` and `task.join`

`spawn expr` evaluates the expression on its own thread and returns a
**task handle** immediately. `task.join(t)` blocks until that thread
finishes and returns what it produced:

```olang
fn heavy(n) = {
    time.sleep(20)
    n * 2
}
let a = spawn heavy(10)
let b = spawn heavy(11)
println(to_string(task.join(a) + task.join(b)))   // both ran concurrently
```

Both calls above overlap: the second `spawn` does not wait for the
first. Only `task.join` blocks, and by the time it runs the work is
already underway — which is why joining several is just `map`:

```olang
fn heavy(n) = {
    time.sleep(20)
    n * 2
}
let jobs = [spawn heavy(1), spawn heavy(2), spawn heavy(3)]
println(to_string(jobs |> map(task.join)))   // [2, 4, 6]
```

A spawned thread sees a **snapshot** of the bindings at spawn time —
capture by value, exactly like a closure — so no two threads share
mutable state and no locks exist in the language. Joining the same
handle twice returns the memoized result rather than running the work
again:

```olang
fn heavy(n) = { time.sleep(20); n * 2 }
let p = spawn heavy(21)
println(to_string(task.join(p) == task.join(p)))   // memoized: true
```

**Failure is a value.** A task that fails yields `Err(e)` from
`task.join` rather than aborting the program, so worker failure is
handled with the ordinary Result toolkit and one bad task never kills
the batch:

```olang
fn work(n) = if n == 1 => unwrap(Err("task died")) else => n * 10
let jobs = [spawn work(0), spawn work(1), spawn work(2)]
let results = jobs |> map((j) => match task.join(j) { Err(e) => -1, v => v })
println(to_string(results))   // [0, -1, 20]
```

### Bounding the wait

`task.join_timeout(t, ms)` returns `Ok(v)` if the task finished within
the budget and `Err("timed out")` otherwise.

Be precise about what that means: it bounds how long you *wait*, not
how long the task *works*. An OS thread cannot be cancelled from
outside without leaving whatever it was touching in an unknown state,
so olang does not offer a cancel that would be a lie. A timed-out task
runs to completion and its result stays collectible from the same
handle:

```olang
fn slow(n) = { time.sleep(80); n }
let t = spawn slow(7)
println(show(task.join_timeout(t, 5)))      // Err("timed out")
println(show(task.join_timeout(t, 5000)))   // Ok(7) — it kept running
```

When you need work that genuinely stops early, give the task something
to check: a channel it polls, or a value it re-reads each iteration.

### Channels

A channel is a queue of values with a sending and a receiving end.
Where `task.join` collects one result at the end, a channel streams
results as they are produced — which is what you want for a worker pool,
a pipeline stage, or any producer whose output the consumer should start
handling immediately. The full API is in
[the stdlib chapter](stdlib.md#chan--channels).

```olang
let ch = chan.new()
let worker = spawn {
    for n in [1, 2, 3] { chan.send(ch, n * n) }
    chan.close(ch)
    "done"
}
let mut total = 0
let mut going = true
while going {
    match chan.recv(ch) {
        Ok(v) => { total = total + v }
        Err(e) => { going = false }
    }
}
println(to_string(total) + " (" + show(task.join(worker)) + ")")
```

### Data parallelism

For the common case — the same function over every element — reach for
`par_map`, `par_filter`, or `par for` instead of spawning by hand. They
run on a worker pool, carry the same snapshot semantics as `spawn`, and
are differential-tested against their sequential counterparts, so
swapping `map` for `par_map` cannot change results:

```olang
let squares = par_map([1, 2, 3, 4, 5], (n) => n * n)
println(to_string(sum(squares)))
```

See [`par for`](#par-for--parallel-iteration) for the loop form.

### Which to reach for

| You want | Use |
|---|---|
| The same function over a collection | `par_map` / `par_filter` / `par for` |
| A handful of distinct jobs, results at the end | `spawn` + `task.join` |
| Results as they arrive, or tasks talking to each other | `chan` |
| Mutable state within one thread | [`cell`](#cells-the-one-mutable-location) |

## Modules and sharing

A module is a `.ol` file. `share` marks what a module exports; `use` imports
from another module. These examples are `no-run` because they need multiple
files — see [`examples/`](../examples/) for complete working packages.

### Exporting with `share`

```olang no-run
// lib/geometry.ol
share fn area(w, h) = w * h            // shared function
share let ORIGIN = (0, 0)              // shared constant
share type Shape = enum { Sq(Int), Rc(Int, Int) }   // shared type
share trait Drawable { fn draw(self) -> String }     // traits register globally
fn helper(x) = x + 1                   // private: not shared
```

Shared functions may call the module's private helpers regardless of
declaration order — the module's whole scope is visible to its exports.

### Importing with `use`

```olang no-run
use lib.geometry { area, Shape }       // selective import
use lib.geometry { * }                 // wildcard: everything shared
use lib.geometry                       // namespace only: geometry.area(...)

// Long import lists may span lines and end with a trailing comma:
use lib.geometry {
    area,
    Shape,
}
```

Importing a shared **enum type also imports its variant constructors**,
so `Sq(2)` constructs in the importer. A `use` also binds the module's
name as a namespace (`geometry.area(3, 4)`).

`share use other { name }` re-exports an import (transitive sharing).

### How `use` resolves

Paths are dot-separated. The first segment decides where the search
goes, in this order:

1. **Embedded stdlib modules** (`colx`, `mathx`) — olang source shipped
   inside the binary.
2. **Package dependencies** — if the first segment names a dependency
   from `olang.toml`, resolution continues inside that package (see
   [Packages](packages.md#how-use-finds-a-package)). Dependency names
   win over local files, so a dependency can never be shadowed by a
   sibling `.ol` file.
3. **The importing file's directory** — `use lib.geometry` from
   `src/main.ol` tries `src/lib/geometry.ol`.
4. **The project root** — then `lib/geometry.ol` from the root.
5. **The native stdlib** — `use str` (or `use std.str`) resolves the
   built-in module, though the native modules are in scope without any
   `use`.

Loading a module executes its file once in a fresh environment and
collects the `share`d bindings; a module's exports can call its private
helpers regardless of declaration order, because exports are closed
over the module's *complete* scope after the whole file has run.

Native stdlib modules (`str`, `col`, `math`, `json`, ...) are always in
scope — no `use` needed. Embedded olang-source modules (`colx`, `mathx`)
are imported with `use colx` and ship inside the binary.

For dependencies on other packages — manifests, versioning, lockfiles, the
registry — see [Packages](packages.md).

## Testing

`test "name" { ... }` blocks embed tests next to code. Assertions raise an
error when they fail (so a failing test aborts the run); passing assertions
are silent:

```olang
fn add(a, b) = a + b

test "addition works" {
    assert_eq(add(2, 2), 4)
    assert(add(1, 1) > 1, "sum should exceed operands")
    assert_ne(add(1, 2), 4)
    assert_true(add(0, 1) == 1)
    assert_false(add(1, 1) == 3)
    assert_eq(
        add(20, 22),
        42,
        "assertion arguments may span lines"
    )
}
println("tests passed")
```

These assertions (`assert`, `assert_eq`, `assert_ne`, `assert_true`,
`assert_false`, with an optional trailing message) are language-level
forms: they raise on failure, which is what makes a failing test abort.
The `testing` stdlib module offers a related but different tool —
assertion *functions* that return `Result` values for building custom
harnesses — see the [stdlib reference](stdlib.md#testing--assertions)
for the distinction.

## Type annotations

This section is the annotation *grammar* reference;
[the Types chapter](types.md) tells the whole gradual-typing story —
enforcement semantics, the static checker, and how to adopt types
incrementally.

Annotations may appear on `let` bindings, parameters, and return types,
and each one is a promise the runtime keeps. A parameter annotation
checks the argument at the call boundary, a return annotation checks the
value the function produces, and a `let` annotation checks the bound
value — with the same clear error on every execution tier:

```olang no-run
fn label(n: Int) -> String = `#${n}`
label("seven")
// Type error: parameter 'n' of label expects Int, got String
```

Three rules keep enforcement predictable. Unannotated code is fully
dynamic — no checks, no cost, no judgment. Container annotations check
shallowly at runtime: `List<Int>` promises "a List" in O(1); element
types are the static checker's concern — and it takes them:
[`olang check`](tooling.md#olang-check) (and the language server in your
editor) decomposes literals element by element, so `[1, "a"]` against
`List<Int>` is flagged before the program runs, with no false positives.
And `Int`/`Float` are strict — an `Int` does not satisfy a `Float`
annotation, matching struct-field enforcement. Generic type parameters
are erased at runtime and never checked (their trait *bounds* are).

```olang
let count: Int = 3
let names: [String] = ["a", "b"]
let table: Map<String, Int> = #{ "k": 1 }
fn apply(f: (Int) -> Int, x: Int) -> Int = f(x)
println(to_string(apply((n) => n + count, 39)))
```

Annotation forms: `Int`, `Float`, `String`, `Bool`, `Map`, custom type
names, `[T]` lists, `(A, B)` tuples, `Map<K, V>`, `(A, B) -> R` functions,
`Result<T, E>`, `A | B` unions, scalar literals
(`"open" | "done"` is a lightweight enum), generic applications
`Name<T>`, `()` unit, and (reserved) intersection forms.

## Appendix: keywords and grammar

Reserved keywords — not usable as identifiers:

```text
fn let if else match for while loop break continue return
true false struct enum
```

Fifteen words, and that is the whole list.

**Eight more that introduce declarations are *contextual*, not
reserved**: `share`, `error`, `test`, `type`, `trait`, `impl`, `use`,
and `meta` (which introduces a [macro declaration](macros.md) only when
directly followed by `fn`). Each only ever appears at the start of its
declaration form, and
the token after it settles the reading — so a declaration and a variable
of the same name coexist:

```olang
let type = "a value"          // an ordinary binding
type Point = struct { x: Int }   // still a type declaration
println(`${type} / ${Point { x: 1 }.x}`)
```

They work as field names too: `row.type` and `row.error` parse.

Each keyword and contextual word, in one line:

| Keyword | Meaning |
|---|---|
| `fn` | Declare a function (`fn name(params) = expr`) |
| `let` | Bind a name; `let mut` makes the binding reassignable |
| `type` | *(contextual)* Declare a `struct` or `enum` type |
| `if` / `else` | Conditional *expression* — `if cond => a else => b` |
| `match` | Pattern-match an expression over arms |
| `for` | Iterate over a list, tuple, range, or string (for a map, iterate `entries(m)`) |
| `while` | Loop while a condition holds |
| `loop` | Loop forever until `break` |
| `break` / `continue` | Exit a loop (optionally with a value) / skip to the next iteration |
| `return` | Return early from a function |
| `true` / `false` | Boolean literals |
| `error` | *(contextual)* Declare a named error type with fields |
| `share` | *(contextual)* Export a declaration from a module |
| `use` | *(contextual)* Import from another module or package |
| `struct` / `enum` | Type-definition forms after `type Name =` |
| `trait` / `impl` | *(contextual)* Declare a trait / implement it for a type |
| `test` | *(contextual)* A named test block, run by `olang test` |
| `meta` | *(contextual)* `meta fn` declares a [macro](macros.md), gone before the program runs |

`mut` and `par` are *contextual* too: `mut` is special only right after
`let`, and `par` only directly before `for` (`par for x in xs`). `Ok`,
`Err`, `Result`, and `spawn` are ordinary names with built-in meaning
rather than reserved words — as are `async`, `await`, `try`, `catch`,
and `Promise`, which named constructs olang no longer has.

`@name(args)` invokes a [macro](macros.md) in expression position, and
`@name` above a `type`, `fn`, or `let` declaration decorates it; both
are resolved and gone before the program runs.

Statement separators are newlines or `;`. Comments are `//` to end of
line. A leading `#!` line (`#!/usr/bin/env olang`) is host metadata,
not syntax: mask-skipped by the parser with line numbers preserved, so
`chmod +x script.ol` just works and errors still point at the right
line.

The full grammar is [`grammar.pest`](../grammar.pest) at the repository
root — pest PEG syntax, and the single source of truth the parser is
generated from. The precedence table in
[Operators](#operators-and-precedence) mirrors its expression hierarchy.

---

Next: [the standard library reference](stdlib.md), or how it all runs in
[Internals](internals.md).
