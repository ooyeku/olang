# The olang Language Reference

This is the complete reference for the olang language: every construct, its
syntax, and its behavior, each with a runnable example. Code blocks marked
`olang` are executed by the test suite (`tests/doc_examples_test.rs`) on every
change — what you read here is what the interpreter actually does.

Part of [the olang book](README.md) ·
[Tour](tour.md) · [Standard Library](stdlib.md) · [Packages](packages.md) ·
[Internals](internals.md) · [Stability](stability.md)

---

## Table of Contents

1. [Source Structure](#source-structure)
2. [Values and Runtime Types](#values-and-runtime-types)
3. [Literals](#literals)
4. [Variables and Assignment](#variables-and-assignment)
5. [Operators and Precedence](#operators-and-precedence)
6. [Strings](#strings)
7. [Collections](#collections)
8. [Control Flow](#control-flow)
9. [Pattern Matching](#pattern-matching)
10. [Functions](#functions)
11. [Pipelines](#pipelines)
12. [User-Defined Types](#user-defined-types)
13. [Traits](#traits)
14. [Error Handling](#error-handling)
15. [Async and Concurrency](#async-and-concurrency)
16. [Modules and Sharing](#modules-and-sharing)
17. [Testing](#testing)
18. [Type Annotations](#type-annotations)
19. [Appendix: Keywords and Grammar](#appendix-keywords-and-grammar)

---

## Source Structure

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

## Values and Runtime Types

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
println(to_string(0xff) + " " + to_string(0b1010) + " " + to_string(0o17))
println(to_string(-42))
```

Integers are 64-bit signed. Overflow in arithmetic is a runtime error, not a
silent wraparound.

### Floats

A decimal point is required; scientific notation is supported:

```olang
println(to_string(3.14))
println(to_string(1.5e3))     // 1500
println(to_string(2.5e-1))    // 0.25
```

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

**Template strings** (backticks) interpolate expressions with `${...}`:

```olang
let n = 6
println(`${n} times 7 is ${n * 7}`)
```

**Character literals** (`'a'`) are one-character strings — olang has no
separate character type:

```olang
let c = 'x'
println(typeof(c) + " " + c)
```

## Variables and Assignment

`let` binds a name to a value. `let mut` is accepted and marks intent — but
note that **every olang binding is assignable**; `mut` is documentation for
the reader, not a permission for the compiler:

```olang
let fixed = 10
let mut counter = 0
counter = counter + 1
println(to_string(fixed + counter))
```

### The mutability model

It is worth being precise about what assignment does, because olang
separates two things that many languages fuse: *bindings* are mutable,
*values* are not.

- A **binding** is a name-to-value association. Assignment (`counter =
  counter + 1`) points the name at a different value. This is always
  allowed.
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

### Destructuring

`let` patterns destructure tuples and lists, with `...rest` capturing a
list's remainder:

```olang
let (x, y) = (3, 4)
let [first, second] = [10, 20]
let [head, ...tail] = [1, 2, 3, 4]
println(to_string(x + y + first + second + head) + " tail: " + to_string(tail))
```

Struct patterns work in `let` too:

```olang
type Point = struct { x: Int, y: Int }
let p = Point { x: 1, y: 2 }
let Point { x, y } = p
println(to_string(x + y))
```

### Scope

Function bodies (including lambdas and nested `fn`s), `match` arms, and
`for` bodies introduce fresh scopes: a `let` inside them — and a `for`
loop's iteration variable — is gone when they end. A bare block
expression, by contrast, does **not** currently fence its bindings: a
`let` made inside `{ ... }` remains visible after the block. Write code
as if blocks scoped (the tooling assumes it, and a future release may
tighten the rule); when you need real encapsulation, use a function.

```olang
fn f() = {
    let inner = 5      // scoped: gone when f returns
    inner
}
println(to_string(f()))

for i in 0..3 { }      // i is scoped to the loop
let msg = match 1 { n => "arm bindings are scoped too" }
println(msg)
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

A lambda (or nested `fn`) closes over the bindings it references — and
it captures them **by value**: the closure carries a snapshot of the
environment as it was at the moment the closure was created. Two rules
follow, and both are worth internalizing early because they shape how
olang programs are structured.

**Rule one: later rebinding does not reach into a closure.**

```olang
let base = 10
let add_base = (n) => n + base
let base = 99                 // rebinding does not affect the closure
println(to_string(add_base(5)))   // 15
```

**Rule two: assignment inside a closure writes to the closure's own
snapshot, never back to the enclosing scope** — and the snapshot is
restored on each call, so a closure cannot accumulate hidden state:

```olang
let mut clicks = 0
let record = () => {
    clicks = clicks + 1    // updates this call's copy only
    clicks
}
println(to_string(record()))   // 1
println(to_string(record()))   // 1 — each call starts from the snapshot
println(to_string(clicks))     // 0 — the outer binding never moved
```

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

## Operators and Precedence

From loosest to tightest binding:

| Level | Operators | Notes |
|---|---|---|
| 1 | `&&` `\|\|` | logical, short-circuit |
| 2 | `==` `!=` `<` `<=` `>` `>=` | comparison |
| 3 | `+` `-` | additive |
| 4 | `*` `/` `%` | multiplicative |
| 5 | `&` `\|` `^` `<<` `>>` | bitwise |
| 6 | `\|>` | pipeline |
| 7 | `..` `..=` | ranges |
| 8 | `-` `!` (prefix) | unary |
| 9 | `f(x)` `.field` `[i]` `?` | call, access, index, try |

When in doubt, parenthesize — especially around comparisons feeding `&&`,
which read best fully grouped: `(a >= lo) && (a <= hi)`.

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
println(to_string(7 / 2) + " " + to_string(7 % 3) + " " + to_string(7.0 / 2))
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
println(to_string(6 & 3) + " " + to_string(6 | 3) + " " + to_string(6 ^ 3))
println(to_string(1 << 4) + " " + to_string(32 >> 2))
```

### String and list `+`

`+` concatenates two strings and concatenates two lists. Mixing a number
and a string is a type error, not a silent coercion — convert the number
with `to_string(...)` first:

```olang
println("count: " + to_string(42))
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
println(to_string(str.index_of("abc", "z")))   // -1 when absent
println(join(split("a,b,c", ","), " + "))
```

Strings iterate by character with `for` (each a 1-character string), and
`str.chars` gives the same characters as a list:

```olang
for c in "abc" { print(c + ".") }
println("")
println(to_string(str.chars("abc")))
```

## Collections

### Lists

Ordered, heterogeneous, immutable values. Index with `[i]` (0-based),
concatenate with `+`, and transform with the higher-order builtins:

```olang
let xs = [3, 1, 4, 1, 5]
println(to_string(xs[0]) + " len=" + to_string(len(xs)))
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
    println(name + ": " + to_string(score))
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
println(to_string(map_len(scores)) + " then " + to_string(map_len(scores2)))
println(to_string(map_has_key(scores, "cyn")) + "/" + to_string(map_has_key(scores2, "cyn")))
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
println(user.name + " is " + to_string(user.age))
```

Objects and maps differ: object fields are identifiers accessed with dot;
map keys are arbitrary strings accessed with `map_get`. A `{ ... }` with
`identifier: value` pairs is an object; `#{ ... }` is always a map.

**Disambiguation note:** `{ ... }` containing statements is a
[block](#blocks-are-expressions); `{ name: expr }` is an object literal.

## Control Flow

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

### `while`

```olang
let mut n = 1
while n < 100 { n = n * 2 }
println(to_string(n))   // 128
```

### `for`

Iterates lists, ranges, and strings (by character), binding each element:

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
snapshots, so mutating enclosing state is not visible to the caller —
use `par for` for real per-element effects and heavy computation, and
`par_map` when you want values back. If several iterations fail, the
error reported is the one the sequential loop would have hit first.
`break` and `return` cannot cross the parallel boundary; `continue`
works within an iteration. The loop evaluates to Unit.

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

## Pattern Matching

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
    "double of " + to_string(n) + " is " + to_string(d)
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
println(to_string(inc(1)) + " " + make() + " " + to_string(complex(3)))
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

## User-Defined Types

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
println(to_string(area(Rect(3.0, 4.0))) + " " + to_string(area(Unknown)))
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
Union and intersection *annotations* exist in the grammar for future use.
See [Stability](stability.md) for the full experimental list.

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

impl Greet for Robot { fn name(self) = "unit-" + to_string(self.id) }
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

## Error Handling

olang uses `Result` values, not exceptions. `Ok(v)` carries a success,
`Err(e)` an error; functions that can fail return one of the two.

```olang
fn safe_div(a, b) = if b == 0 => Err("division by zero") else => Ok(a / b)

let good = safe_div(10, 2)
let bad = safe_div(1, 0)
println(to_string(is_ok(good)) + " " + to_string(is_err(bad)))
```

### Consuming results

Pattern match, or use the helper builtins:

```olang
fn safe_div(a, b) = if b == 0 => Err("division by zero") else => Ok(a / b)

let msg = match safe_div(10, 3) {
    Ok(v) => "got " + to_string(v),
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

### `try` / `catch`

`try { ... } catch (e) { ... }` is expression-level sugar over a `Result`:
`Ok` unwraps to its value, `Err` binds the error to `e` and evaluates the
catch block, and **any other value passes through unchanged** (the try
block simply succeeded). It does **not** catch runtime errors (an actual
`1 / 0` still aborts) — it destructures Results.

```olang
fn risky(n) = if n > 0 => Ok(n * 2) else => Err("negative input")
let a = try { risky(21) } catch (e) { 0 }
let b = try { risky(-1) } catch (e) { 0 }
println(to_string(a) + " " + to_string(b))
```

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

## Async and Concurrency

olang has two concurrency mechanisms, both explicit:

- **Deterministic promises** (`async`, `Promise.resolve/reject/delay`,
  `all`, `race`): a promise carries a settled value or a *deadline*, and
  `await` sleeps exactly as long as needed. Awaiting several delays started
  together costs the longest, not the sum. No threads are involved —
  timing is simulated, deterministically.
- **Real background threads** (`spawn`): `spawn expr` evaluates the
  expression on its own OS thread against a snapshot of the current
  bindings, returning a promise that `await` joins. This is genuine
  parallelism — three 100ms tasks awaited together take ~100ms.

### `async` functions and lambdas

`async fn` declares a function returning a promise; `async (args) => ...`
is the lambda form. `await` resolves a promise to its value:

```olang
async fn fetch_score(id) = {
    let base = Promise.delay(id * 10, 5)   // value, then ms
    let v = await base
    v + 1
}
println(to_string(await fetch_score(3)))
```

### The Promise API

| Expression | Meaning |
|---|---|
| `Promise.resolve(v)` | already-settled promise of `v` |
| `Promise.reject(e)` | failed promise |
| `Promise.delay(v, ms)` | resolves to `v` after `ms` milliseconds |
| `Promise.all(list)` | list of results; waits for the slowest |
| `Promise.race(list)` | first result; waits only for the fastest |
| `spawn f()` | run a call as a promise |

```olang
let jobs = [Promise.delay("a", 10), Promise.delay("b", 5), Promise.resolve("c")]
println(to_string(Promise.all(jobs)))     // ["a", "b", "c"] — total wait ≈ 10ms
println(Promise.race([Promise.delay("slow", 50), Promise.delay("fast", 5)]))
```

`Promise.all` accepts any list expression — a variable, a `map` result — not
just a literal.

### `spawn`

`spawn expr` starts evaluating the expression on a **real background
thread** and returns a promise immediately; `await` joins it. The thread
sees a snapshot of the bindings at spawn time (capture by value, like
closures), a failing task rejects the promise, and awaiting the same
promise again returns the memoized result:

```olang
fn heavy(n) = {
    time.sleep(20)
    n * 2
}
let a = spawn heavy(10)
let b = spawn heavy(11)
println(to_string(await a + await b))   // both ran concurrently

let p = spawn heavy(21)
println(to_string(await p == await p))  // memoized: true
```

Spawned promises compose with `Promise.all`/`race` like any other.

**Failure is a value.** Awaiting a failed task (or any rejected promise)
yields `Err(e)` rather than aborting, so worker failure is handled with
the ordinary Result toolkit — one bad task never kills the batch:

```olang
fn work(n) = if n == 1 => unwrap(Err("task died")) else => n * 10
let jobs = [spawn work(0), spawn work(1), spawn work(2)]
let results = jobs |> map((j) => try { await j } catch (e) { -1 })
println(to_string(results))   // [0, -1, 20]
```

## Modules and Sharing

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

## Type Annotations

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
fn label(n: Int) -> String = "#" + to_string(n)
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
`Result<T, E>`, `Promise<T>`, `A | B` unions, generic applications
`Name<T>`, `()` unit, and (reserved) intersection/literal forms.

## Appendix: Keywords and Grammar

Reserved keywords — not usable as identifiers:

```text
fn let type if else match for while loop break continue return
true false async await try catch error share use
struct enum test trait impl
```

`mut` is a *contextual* keyword: special only right after `let`. `Ok`,
`Err`, `Promise`, and `spawn` are ordinary names with built-in meaning.

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
