# Language reference

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Standard library reference](stdlib.md) ·
[Packages and dependencies](packages.md) ·
[Architecture and internals](internals.md) ·
[Stability and compatibility](stability.md)

This chapter is the complete reference for the olang language: every
construct, its syntax, and its behavior, each with a runnable example. Every
`olang` block runs as written.

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

An expression continues onto the next line when its line ends with a
binary operator, or when the next line starts with one. So a long
pipeline, condition or sum breaks before its `|>`, `&&`, `||`, `??`,
comparison, `+` or `-`. `+` has no unary form, so a line that starts
with it always continues. A line that starts with `-` continues only
when the `-` is written as a binary operator, with a space after it
(`- 10.0`); a `-` glued to its operand (`-1`) is a negation, and starts
a statement of its own. A negation whose value is then dropped is
reported by `olang check` (class `no_effect`, [tooling.md](tooling.md#advisory-warnings)).

```olang
let words = ["b", "a", "c"]
    |> sort
    |> join("-")
let total = 1 +
    2
let price = 1.0 + 2.0
    - 10.0                     // continues: `price` is -7.0
let net = 100
    - 15
    + 5
println(`${words} ${total} ${price} ${net}`)   // a-b-c 3 -7.0 90
```

A method chain breaks the same way, before its `.`: a line that starts
with one `.` and a name continues the expression above it, so each step
of a chain may take a line of its own, with comments and blank lines
between. `..` (a range) and `...` (a spread) never continue a line.

```olang
let f = ods.frame({ city: ["Oslo", "Lima", "Rome"], price: [4.0, 2.5, 3.5] })
let cheap = f
    .with({ eur: ods.col("price") * 0.9 })
    // the steps may carry comments
    .filter(ods.col("eur") < 3.5)
    .sort_by("eur")
println(cheap.city.to_list())   // ["Lima", "Rome"]
```

Statements are one of: a declaration (`let`, `fn`, `type`, `trait`, `impl`,
`error`, `test`), a module statement (`use`, `share`), or an expression.
A top-level function is declared once per file: a second `fn` of the
same name in the same file is an error, from `olang check` and before the
file runs (the REPL, where each input is a program of its own, lets a
later input redefine a name).
Expressions evaluated at the top level are allowed and their value is
discarded (the last one is the program's result in the REPL).

Identifiers start with a letter or an underscore and continue with
letters, digits, and underscores: `total`, `user_name`, `isValid2`,
`_unused`. A bare `_` is not a name but the discard pattern (`let _ =
...`, `for _ in ...`, a `match` arm). Identifiers cannot be
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
| `json.parse("{}")`'s object | `JsonObject` |
| `1..5` | `Range` |
| an enum's value (`Circle(1.0)`) | the enum's type name (`Shape`) |
| `() => 1` | `Function` |
| a builtin (`len`, `str.trim`) | `Builtin` |
| `Ok(1)` / `Err("e")` | `Result` |
| `bytes.from_string("a")` | `Bytes` |
| `cell.new(0)` | `Cell` |
| `chan.new()` | `Channel` |
| a task handle (`spawn f()`) | `Task` |
| a module (`math`) | `Module` |
| unit / JSON `null` / missing | `Unit` |

```olang
println(typeof(42) + " " + typeof(3.14) + " " + typeof("hi") + " " + typeof(true))
println(typeof([1]) + " " + typeof((1, 2)) + " " + typeof(#{ "k": 1 }))
```

A map, an object and a parsed JSON object are three kinds that `typeof`
tells apart, and one kind everywhere else: they read alike with `.`
and `map_get`, compare with `==`, match the same record pattern
(`{ kind: "a" }` matches all three) and satisfy the same annotation
(`m: Map` accepts all three). A map's keys are strings: an Int, a Float
or a Bool key is its text, and because `1 == 1.0`, a whole Float is the
Int's text — `#{ 1: "a", 1.0: "b" }` is one key, `"1"`, holding `"b"`
(the later entry wins), and `map_get(m, 1.0)` finds `m`'s `"1"`. Any
other Float is its printed form (`#{ 1.5: x }` has the key `"1.5"`).

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
in plain form (`1500.0`) or exponent form (`1e301`). The form is plain
for a magnitude in [1e-4, 1e16) and for zero, and exponent form
otherwise (`1e-5`, `1e16`); a whole number in plain form keeps its `.0`,
and negative zero prints `-0.0`.

A Float is always a finite number. Operations whose IEEE result would
be `NaN` or infinite raise a runtime error instead: `0.0 / 0.0`,
`math.sqrt(-1.0)`, `math.log(-1.0)`, division by zero (exactly as for
integers), and overflow — `1e308 * 10.0` is `Float overflow in
multiplication`, as `math.exp(1000.0)` and `math.pow(10.0, 400.0)` are
errors, and `str.parse_float("1e999")` is an `Err` ("out of range").
A literal beyond the range, `1e400`, is a syntax error. The rule holds
on all three tiers: the bytecode and JIT tiers check every float
result and raise the same message. `inf` and `NaN` have printed forms
(`to_string` renders them for values that arrive from outside — a
Series computed by the data stack's IEEE kernels, see the ods chapter)
but no literal and no arithmetic path produces them.

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
| `\xNN` | the character U+00NN, by two hex digits: `"\xe9"` is `é`, one character (two bytes in UTF-8), not the byte 0xE9 |
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

Both mistakes are caught before they print: `olang check` warns on an
escape-looking `\n`, `\t`, or `\r` inside a template (write `\\n` — the
same two output characters — to mark the backslash deliberate; escapes
inside a `${...}`'s own double-quoted strings are real syntax and draw
no warning), and the parse error from a backtick inside `${...}` names
the nesting rule.

**Character literals** (`'a'`) are one-character strings — olang has no
separate character type:

```olang
let c = 'x'
println(typeof(c) + " " + c)
```

### How values print

`print`, `println`, `show`, `to_string` and a template's `${…}` write a
value as olang source writes it, so what a program prints reads back as
the value. A String at the top level is the exception: it prints as its
text (`println("a")` writes `a`); inside a container it is quoted and
escaped.

| Value | Printed |
|---|---|
| String inside a container | `"a\"b"`, with `\"`, `\\`, `\n`, `\r`, `\t` and other control characters escaped |
| List, tuple | `[1, "a"]`, `(1, "a")` |
| Map | `#{"k": 1}`, its keys in the map's order |
| Object, parsed JSON object | `{ a: 1, b: 2 }`, its fields in their order (a JSON object with a key that is not a name prints as the map it reads as, `#{"a b": 1}`) |
| Struct | `P { x: 1, y: 2 }`, its fields in the order the declaration writes them |
| Enum value | its constructor: `North`, `Ring(2.0)` |
| `Result` | `Ok(1)`, `Err("bad")` |
| Unit | `()` |
| Function, builtin, range, module | `<function: name>`, `<builtin: name>`, `1..5`, `<module: N members>` |

```olang
type Sample = struct { y: Int, x: Int }
type Mark = enum { Dot, Ring(Float) }
println([Sample { x: 1, y: 2 }, Ring(2.0), Dot])   // [Sample { y: 2, x: 1 }, Ring(2.0), Dot]
println(#{ "k": ["a\"b"] })                     // #{"k": ["a\"b"]}
println({ name: "ada" })                         // { name: "ada" }
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
was never bound is an error, not an implicit `let`, and so is an
assignment to a binding not declared `mut`. Both are reported before
the program runs, whichever branch they sit in:

```text
cannot assign to 'total': it is not declared in this scope. Declare it
first with `let total = ...`, or `let mut total = ...` if it needs to change
```

```text
cannot assign to 'n': it is not declared mutable. Declare it with
`let mut n = ...`, or bind a new value with `let n = ...` to shadow it
```

Shadowing — a fresh `let` of the same name — is always available and
is often the better answer, because it produces a new binding rather
than a mutable one:

```olang
let raw = "  42 "
let raw = str.trim(raw)     // a new binding, not a mutation
println(raw)
```

An assignment answers `()`: the write is its effect, and it is not a
value to pass on. `println(n = 3)` prints `()`, `let y = (x = 5)` binds
`()`, and a block or function that ends in an assignment answers `()`
(end it with the name, `{ total = total + n; total }`, when its value
is wanted). Every tier agrees.

### The mutability model

olang separates two things: *bindings* are reassignable when declared
`mut`, *values* are never mutable.

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

Closures snapshot their environment cheaply because of it, and `spawn`
and `par_map` hand values to other threads without locks.

Assignment (`name = expr`) only updates an existing `let mut` binding;
it never creates one.

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
back, what reading a missing key yields, and what a null becomes
when it crosses in from JSON or a data column — so being able to write
it is what makes those comparable:

```olang
let nothing = ()
println(to_string(nothing == ()))                  // true
println(to_string(#{ "a": 1 }.zz == ()))   // true
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

A function's or a lambda's parameter destructures the same way — a
tuple, a list or an object pattern where a name goes:
[Parameters](#parameters-annotations-defaults-named-arguments).

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

### Where a name is looked up

A name a function body does not bind itself is resolved **lexically**:
in the enclosing scopes of the function's own definition, then in the
top-level table of the file that defines it, then in the program's
top-level bindings. The frames that *called* the function are never a
source of names. So a library's `serve` with a local `head_for`, calling
an application function that names `head_for`, reaches the
application's `head_for` — declared before or after the caller, on the
main thread or on a task — and a name nothing binds is an error that
names it, not a lookup in whatever frame happened to be active.

Two consequences of the file-level table: a module's functions see every
sibling in their file, private or shared, however the file is ordered;
and a nested `fn` sees its sibling nested functions, including those
declared after it in the same body.

```olang
fn page_head(req) = head_for(1)          // resolves to the head_for below
fn head_for(n) = "app " + to_string(n)
fn serve(head) = {
    fn head_for(req) = head(req)         // a local of serve: invisible to page_head
    let t = spawn { head_for(0) }
    show(task.join(t))
}
println(serve(page_head))                // Ok("app 1")
```

### Closures capture by value

A lambda (or nested `fn`) closes over the local bindings it references,
and it captures them **by value**: the closure carries a snapshot of
those bindings as they were at the moment the closure was created. A
**top-level** name is different: it is never captured. Every function,
wherever it was declared, reads a top-level name's binding as it is
when the function runs. Three rules follow, and they affect how olang
programs are structured.

**Rule one: later rebinding of a local does not reach into a closure.**

```olang
fn make_adder() = {
    let base = 10
    let add_base = (n) => n + base
    let base = 99             // rebinding does not affect the closure
    add_base
}
println(to_string(make_adder()(5)))   // 15
```

**Rule two: a top-level name is read when the function runs.** A
top-level `let mut` the program reassigns, or a name a later top-level
`let` binds again, is seen as it stands at each call — by a function
declared above it, a lambda made at the top level, a module's functions
reading their module's names — on every tier:

```olang
let mut limit = 1
fn over(x) = x > limit
println(to_string(over(5)))   // true
limit = 10
println(to_string(over(5)))   // false: the binding as it is now
```

A task (`spawn`) and an `http.serve` worker run on a copy of the
program's top-level scope taken when they start: a binding the main
program changes afterwards is not seen on that thread.

**Rule three: a closure cannot assign to a binding it captured**, nor a
function to a top-level binding. Since
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

Because a closure owns an immutable snapshot, a closure handed to
another thread (`spawn`, `par_map`, `par for`) cannot see another
thread's writes: there is nothing shared to race on, and the language
has no locks.

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
        render(unwrap(json.parse(resp)).items)
    })

dom.on(dom.query("#add"), "click", (id) => {
    dom.fetch("POST", "/api/items", new_item_body(), (resp) => reload())
})
reload()
```

No handler holds state, so capture-by-value costs nothing. The
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

The same holds inside a collection: a list, tuple or map holding a cell
equals another only when it holds the same cell (`[a] == [a]` is true,
`[cell(1)] == [cell(1)]` false).

**A cell belongs to the thread that created it.** Reading or writing
one from another thread is an error, and sending one through a channel
is refused at the send, so two threads never reach the same mutable
location. A cell created *inside* a task, and used only there, is
ordinary: the rule is about crossing, not about tasks.

```olang
let readings = cell([])
let worker = spawn {
    // A cell made in here belongs in here.
    let local = cell(0)
    for n in [1, 2, 3] { cell.update(local, (t) => t + n) }
    cell.get(local)
}
cell.set(readings, [unwrap(task.join(worker))])
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
| 4 | `??` | default for `()`, short-circuit |
| 5 | `\|>` | pipeline |
| 6 | `..` `..=` | ranges |
| 7 | `\|` | bitwise or |
| 8 | `^` | bitwise xor |
| 9 | `&` | bitwise and |
| 10 | `<<` `>>` | shifts |
| 11 | `+` `-` | additive |
| 12 | `*` `/` `%` `@` | multiplicative; `@` is the matrix product |
| 13 | `-` `!` (prefix) | unary |
| 14 | `f(x)` `.field` `[i]` `?` | call, access, index, try |

All binary operators are **left-associative**, and comparisons do not
chain: `a < b < c` and `a == b == c` are parse errors (write `a < b &&
b < c`, or parenthesize the comparison meant to come first). The
ordering follows the conventional readings:

- **`&&` binds tighter than `||`.** `a || b && c` is `a || (b && c)`,
  as in C, Rust, and Python.
- **Arithmetic feeds a pipeline, and a pipeline feeds a comparison.**
  `x + 1 |> f` pipes the sum — it is `f(x + 1)` — while
  `xs |> len == 3` compares the pipeline's result. (This is the
  pipeline placement Elixir uses.)
- **`??` defaults what the operators below it compute, and feeds a
  comparison.** It sits below the arithmetic (and the pipeline and
  ranges) and above the comparisons, as Kotlin's `?:` does:
  `a.n ?? 0 > 3` is `(a.n ?? 0) > 3`, and `m.n ?? 1 + 2` is
  `m.n ?? (1 + 2)`.
- **Range bounds are arithmetic.** `0..n-1` is `0..(n-1)`, so the
  everyday loop bound needs no parentheses.
- **The bitwise operators take Rust's order** (and C's, but for
  `&`/`^`/`|` sitting above the comparisons): shifts below `+`/`-`, so
  `1 << 4 * 2` is `1 << 8` and `1 + 2 << 3` is `3 << 3`; then `&`,
  `^`, `|`, all above the comparisons, so `x & 1 == 0` is `(x & 1) ==
  0`.

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
[Common pitfalls](pitfalls.md) for worked examples.

### Evaluation order

Evaluation is strict (arguments are evaluated before a call runs) and
proceeds **left to right** everywhere: binary operands, call arguments,
and the elements of list, tuple, and map literals all evaluate in
source order. The exceptions are the constructs whose entire purpose is
*not* evaluating something: `&&`/`||` skip the right operand when the
left settles the answer, `??` evaluates its right operand only when the
left is `()`, and `if`/`match` evaluate only the taken arm.

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

`/` on two integers is integer division, truncating toward zero; `%` is
the remainder of that division, so it takes the sign of the left
operand: `-7 % 3` is `-1` and `7 % -3` is `1` (as in C, Rust, Go and
JavaScript; Python's `%` floors instead). Integer overflow raises, a
shift included: `x << n` that would lose a set bit or change the sign
raises as the multiplication by 2ⁿ would, and a shift count outside
`0..64` raises. Mixing an `Int` and a `Float` produces a `Float`:

```olang
println(`${7 / 2} ${7 % 3} ${7.0 / 2}`)
println(to_string(2 + 0.5))
```

### Comparison and equality

All comparison operators work on numbers (mixing Int/Float), **on
strings**, which order lexicographically by Unicode scalar value, and
**on tuples**, which order lexicographically by element:

```olang
println(to_string("apple" < "banana"))
let c = "7"
println(to_string((c >= "0") && (c <= "9")))   // character-range check
println((2026, "mar") < (2026, "may"))         // true: the first unequal elements decide
println(sort([(2, "b"), (1, "z"), (2, "a")]))  // [(1, "z"), (2, "a"), (2, "b")]
```

Two tuples compare element by element: the first pair that is not equal
decides, by `<`'s rule for that pair (numbers with numbers, strings with
strings, tuples again), and a tuple that is a prefix of the other is
the smaller. Equal elements need no order of their own, so `(true, 1) <
(true, 2)` holds; an unequal pair `<` cannot order (`(1, "a") < ("b",
2)`) raises. `sort` orders a list of tuples the same way, which is the
way to sort by several keys: `sort(map(rows, (r) => (r.year, r.name)))`,
or `col.sort_by(rows, (r) => (r.year, r.name))`.

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

- **An Int and a Float are equal when they are the same number,
  exactly**: `1 == 1.0` is `true`, and `9007199254740993 ==
  9007199254740992.0` is `false` (the Float is not that integer, though
  it is the f64 nearest it). Inside a *structural* comparison kinds are
  strict — `[1] == [1.0]` is `false`, because the elements are an Int
  and a Float. `contains` and `col.index_of` find an element by `==`, so
  `contains([1, 2], 1.0)` is `true`.
- **Struct equality includes the type name**: two structs with the same
  fields but different declared types are not equal.
- **Equality across unrelated kinds answers rather than raising.** `1 ==
  "1"` is `false` and `true != 1` is `true`, so `x == ()`
  asks "is this nothing?" of any value — Unit is the absence value (a
  missing map key, an absent substring, a JSON null). Ordering is
  stricter: `<`, `<=`, `>`, `>=` between kinds they cannot order (`1 <
  "a"`, `x < ()`) raise a type error. Convert explicitly (`to_string`,
  `to_int`) when a cross-type comparison is meant.
- **A `Result` compares structurally**: `Ok(1) == Ok(1)` is `true`,
  `Ok(1) == Err(1)` `false`.
- **Functions, builtins and ranges have no equality.** `f == g` raises.
  Two lists, tuples, maps, records, enum values or `Result`s that differ
  somewhere are unequal; two that agree everywhere except where one
  holds a function, builtin or range have no answer, and `==` raises
  (`[f] == [g]`, at any depth) — while `[1, f] == [2, f]` is `false`,
  since the data already differs. Compare the parts that hold data. One
  collection compared with itself is equal without being walked, and a
  comparison stops at the first difference.
- **One relation, at the top and inside containers**: what `==` answers
  about two values it answers about them as elements, but for the
  strict number kinds above. Within a structural comparison, a kind
  mismatch between elements makes the values unequal (`false`) rather
  than raising.

```olang
type A = struct { x: Int }
type B = struct { x: Int }
println(to_string(A { x: 1 } == A { x: 1 }))   // true
println(to_string(A { x: 1 } == B { x: 1 }))   // false: type names differ
println(to_string(1 == 1.0))                   // true: numeric comparison
```

On the right of a comparison, a parenthesized group followed by `=>` is
an operand, not a lambda's parameters: `if pos == (row, col) => …`
compares `pos` with the tuple, and the `=>` is the `if`'s. A function
literal compared with is written in parentheses of its own.

```olang
fn at_origin(p) = {
    if p == (0, 0) => return "origin"
    "elsewhere"
}
println(at_origin((0, 0)))   // origin
println(at_origin((1, 0)))   // elsewhere
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

### The default operator `??`

`a ?? b` is `b` when `a` is `()`, and `a` otherwise; `b` is evaluated
only when it is needed, as `&&`'s right side is. With `.` on a map it
is the lookup-with-default — `opts.pad ?? 0` is
`map_get_or(opts, "pad", 0)` — and it folds the presence test
`if x == () => d else => x` into one expression. Only `()` is replaced:
`false`, `0` and `""` are values.

```olang
let opts = #{ "pad": 2, "label": "" }
println(to_string(opts.pad ?? 0))          // 2
println(to_string(opts.width ?? 80))       // 80: absent
println("[" + (opts.label ?? "none") + "]") // []: "" is a value
let first = opts.margin ?? opts.pad ?? 0   // chains: the first that is not ()
println(to_string(first))                  // 2
```

### Bitwise operators

On integers: and `&`, or `|`, xor `^`, shifts `<<` `>>`:

```olang
println(`${6 & 3} ${6 | 3} ${6 ^ 3}`)
println(`${1 << 4} ${32 >> 2}`)
```

### Operators on native values

A value supplied by a module implemented in Rust (a Series, a Frame, a
Date) may define what an operator means for it: every binary operator,
unary `-` and `!`, and `&`, `|` and `^`. The module is asked first,
whenever an operand is one of its values, on every tier alike; when it
declines, the operator's ordinary error stands. That is how `s * 2.0`
scales a column, `-s` negates one, and `a & b` combines two masks.

`@` is the matrix product. No built-in value defines it, so it exists
only for native values: two Series give their dot product, and a
Matrix its product. Anything else raises `` `@` needs a Matrix or Series
on each side ``. It binds like `*`. Unlike every other operator it
never continues a line: a line that starts with `@` is a macro call or
a decorator.

```olang
let w = ods.series([0.5, 0.25, 0.25])
let x = ods.series([4.0, 8.0, 12.0])
println(to_string(w @ x))          // 7.0
println(to_string(w @ x * 2.0))    // 14.0: `@` binds like `*`
println(to_string((-x).to_list()))                     // [-4.0, -8.0, -12.0]
println(to_string(((x > 5.0) & !(x > 10.0)).to_list()))  // [false, true, false]
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

`a..b` is half-open, `a..=b` inclusive. Both bounds are Ints: a Float
bound raises "Range start must be an integer" (or end), in a `for` loop
as anywhere else, so `0.5..3` never counts from 0.5. Ranges are values
and iterate:

```olang
let mut sum = 0
for i in 1..4 { sum = sum + i }        // 1+2+3
for i in 1..=4 { sum = sum + i }       // 1+2+3+4
println(to_string(sum))
```

## Strings

Strings are immutable. Global builtins cover the basics (`len`,
`starts_with`, `ends_with`, `split`, `join`), and the `str` module has the
full toolkit — `str.contains` (the global `contains` searches a list),
`str.substring`, `str.index_of`, `str.char_at`, `str.trim`,
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

The edges, stated: `str.substring(s, start, end)` follows the slices'
one rule (`col.slice` and `bytes.slice` too): a position past the
length stops at the length, an `end` at or before `start` gives `""`,
and a negative position raises. `str.char_at(s, i)` raises on a negative
`i` and answers `()` past the end.
`split(s, "")` splits around every character, with an empty string at
each end (`split("abc", "")` is `["", "a", "b", "c", ""]`). Strings are
compared and searched as their code points: olang does no Unicode
normalization, so `"é"` (one code point) and `"e\u{301}"` (two) are
different strings. `str.to_upper` and `str.to_lower` apply Unicode's
full case mapping, which may change the length (`str.to_upper("straße")`
is `"STRASSE"`), with no locale (no Turkish dotless i).

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

`xs[i] = v` replaces one element of a list held in a `let mut`
binding. A list is a value, so this is an assignment of `xs` — the list
with element `i` replaced — and follows every rule an assignment has:
the binding must be `mut`, and it cannot be a variable captured by a
closure. Another binding of the same list keeps its old elements. An
index outside the list, a negative one included, is the error a read
raises. When nothing else holds the list
the element is written in place, so a loop of element writes costs
O(1) a write.

```olang
let mut grid = [0, 0, 0, 0]
let before = grid
for i in 0..len(grid) {
    grid[i] = i * i
}
grid[len(grid) - 1] = 99
println(grid)     // [0, 1, 4, 99]
println(before)   // [0, 0, 0, 0]
```

The same holds one level into a map or an object: `m.k[i] = v` replaces
element `i` of the list at `m.k` and rebinds `m`. Deeper paths
(`m.a.b[i] = v`, `xs[i][j] = v`) and assigning a field itself
(`m.k = v`) are not supported; write `map_set(m, "k", v)` or
`map_set_in(m, path, v)` for those.

```olang
let mut board = #{ "cells": ["", "", ""] }
board.cells[1] = "x"
println(board.cells)   // ["", "x", ""]
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
println(sales["amount"].to_list())                 // a Frame takes a column name
println(to_string(sales["amount"][1]))             // a Series takes a position
```

A value that defines no subscript reports that it cannot be indexed,
naming its type. Subscripting a native value is read-only: there is no
`value[k] = x`. A Frame accepts a String (a column) and a Bool mask
(rows), and refuses a row position; see
[The data stack](ods.md#reaching-a-column).

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
returns a new map.

A map **keeps its keys in the order they were first set**, as a Python
dict or a JavaScript object does: `entries`, `map_keys`, `map_values`,
printing and `json.stringify` walk that order; `map_set` of a key it
has keeps its place, a new key goes last, and `map_remove` closes the
gap. An object's fields (`{ a: 1 }`, a parsed JSON object) keep theirs
the same way. Equality ignores the order:

```olang
let m = #{ "zeta": 1, "alpha": 2 } |> map_set("mid", 3) |> map_set("zeta", 9)
println(to_string(map_keys(m)))                       // ["zeta", "alpha", "mid"]
println(m)                                             // #{"zeta": 9, "alpha": 2, "mid": 3}
println(to_string(#{ "a": 1, "b": 2 } == #{ "b": 2, "a": 1 }))   // true
```


```olang
let scores = #{ "ada": 99, "bob": 82 }
println(to_string(scores.ada))
let scores2 = #{ ...scores, "cyn": 91 }
println(`${map_len(scores)} then ${map_len(scores2)}`)
println(`${map_has_key(scores, "cyn")}/${map_has_key(scores2, "cyn")}`)
println(to_string(sort(map_keys(scores2))))
```

**`m.key` reads a key** whose name is an identifier: it is exactly
`map_get(m, "key")`. A missing key is `Unit`, and a chain reads nested
maps. A key that is not an identifier (`"content-type"`,
`"log_price:product_x"`), or one computed at run time, is read with
**`m[k]`**, which is exactly `map_get(m, k)` too — `Unit` when the key
is absent, an Int, Float or Bool key read as its text:

```olang
let p = #{ "running": true, "pp": #{ "cur": 3 }, "content-type": "csv" }
println(to_string(p.running))              // true
println(to_string(p.pp.cur))               // 3
println(to_string(p.live_id == ()))        // true: absent is ()
let key = "running"
println(to_string(p[key]))                 // true
println(p["content-type"] + " " + to_string(p["pp"]["cur"]))   // csv 3
println(to_string(p["gone"] ?? 0))         // 0
```

`[]` reads; it does not write (`m[k] = v` is refused, saying so). A key
that is not a String, Int, Float or Bool raises, and `()["k"]` is `()`,
as `().k` is.

`map_get` (and `.`) is the **raw read**: on a missing key it returns
`Unit`, and distinguishing absent from stored-null takes `map_has_key`.
The lookup-with-default is [`??`](#the-default-operator-), or
`map_get_or(m, key, default)` for a key that is not an identifier; both
yield the default for a missing key *and* for a stored `Unit` — absence
is one rule, not two — and `??` evaluates the default only when it is
used:

```olang
let m = #{ "a": 1 }
println(to_string(m.a ?? 0))                // 1
println(to_string(m.z ?? 0))                // 0
println(to_string(map_get_or(m, "z", 0)))   // 0
```

A literal **spreads** another map or object with `...`: `#{ ...m, "k": v }`
is a copy of `m` with `"k"` set, `{ ...o, k: v }` an object's copy, and
entries are laid down in the order written, so a later key wins. The
literal's own kind is the result's (`#{ ...o }` is a map, whatever `o`
is). A nested write is `map_set_in(m, ["pp", "cur"], x)`, its read
`map_get_in(m, ["pp", "cur"])`, and an update through a function
`map_update(m, "n", (v) => (v ?? 0) + 1)`; like `map_set`, each writes
in place a map that nothing else holds:

```olang
let base = #{ "pad": 2, "pp": #{ "cur": 1 } }
let next = #{ ...base, "pad": 4, "title": "x" }
println(to_string(next.pad) + " " + to_string(base.pad))   // 4 2
let moved = map_set_in(base, ["pp", "cur"], 9)
println(to_string(moved.pp.cur))                           // 9
let counted = map_update(base, "hits", (n) => (n ?? 0) + 1)
println(to_string(counted.hits))                           // 1
```

A spread belongs to the two literals only: a list joins with `+`, a
declared struct's literal names every field, and a call takes its
arguments one by one — the parser says so where one is written.

A key wins over a trait method of the same name, as a struct's field
does ([Traits](#traits)): `m.name()` calls the function stored under
`"name"` when the map has that key, and the method only when it has
not. `.` reads a key; `m.k = v` is not an assignment form, so a key is
written with a spread (`#{ ...m, "k": v }`) or `map_set`. The one write
through `.` is an element of a list held at a key, `m.k[i] = v`.

The `map_*` accessors, and `[]`, also read **any struct-like value** —
anonymous objects, structs, and parsed JSON objects — so dynamic key access
works uniformly:

```olang
let obj = { a: 1, b: 2 }
let key = "b"
println(to_string(map_get(obj, key)))
println(to_string(obj[key]))       // 2
```

### Anonymous objects

`{ field: value }` builds an object with named fields, accessed with `.`:

```olang
let user = { name: "ada", age: 36 }
println(`${user.name} is ${user.age}`)
```

Objects and maps are both read with `.`, and `.key` on either is
`map_get(v, "key")`: the key's value, or `Unit` when it is absent. The
same holds for a **parsed JSON object** (`json.parse`), which carries
outside data, where absence is ordinary. Their keys are any strings: an
object literal's key is a name or, for one that is not a name, a quoted
string (`{ "P(B > A)": 0.9, n: 3 }`), and a key that is not a name is
read with `o["P(B > A)"]` or `map_get`. An object whose keys are not
all names prints as the map it reads as (`#{"P(B > A)": 0.9, "n": 3}`).
A declared struct's literal names its fields and takes no quoted key. A
**declared struct** is the record with fixed fields: reading a field
it does not have raises. A `{ ... }` with `identifier: value` pairs is
an object; `#{ ... }` is always a map.

A field of `()` is `()`: an absent record's fields are absent too, so a
chain of reads answers `()` at the first missing link (`s.pp.cur` when
`s.pp` is absent) and `?? default` finishes it. `map_get((), k)` is `()`
as well. A field of anything else that is not a record raises (`5.k`),
as does a declared struct's missing field.

```olang
let s = #{ "name": "ada" }
println(to_string(s.pp.cur))          // (): s.pp is absent
println(to_string(s.pp.cur ?? 0))     // 0
```

`r?.f` is reserved and refused by the parser (it would read as optional
chaining); a field of what `?` unwraps is written `(r?).f`.

```olang
let o = { name: "ada" }
let j = unwrap(json.parse("{\"name\": \"ada\"}"))
println(o.name + " " + j.name)
println(to_string(o.email == () && j.email == ()))   // true: absent is ()
```

A misspelt key is reported by `olang check` before the program runs.

**Disambiguation note:** `{ ... }` containing statements is a
[block](#blocks-are-expressions); `{ name: expr }` is an object literal.

## Control flow

### `if` expressions

`if` is an expression. Its arms use `=>` and produce values; `else` is
optional (a false condition without `else` yields `Unit`). The
condition is a Bool, as an operand of `&&`, `||` and `!` is: any other
value raises "a condition must be a Bool" (an empty list or `()` is not
false; write `len(xs) > 0`, `x != ()`). The same holds for `while` and a
`match` guard. Chains use `else if`:

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

A block evaluates its statements and yields the last expression's
value. A block whose last statement is a `let`, or ends in `;`, has no
last expression, and its value is `()`, as an empty block's is; so is a
function whose body is `{}` (`fn noop() = {}` answers `()`, not an
empty object — write `#{}` for an empty map):

```olang
let result = {
    let a = 6
    let b = 7
    a * b
}
println(to_string(result))
```

```olang
println(to_string({ let a = 1 }))   // ()
println(to_string({ 1; }))          // ()
fn noop() = {}
println(to_string(noop()))          // ()
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
    println(k + " -> " + show(v))      // b -> 2, then a -> 1: the map's order
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

A [cell](stdlib.md#cell--mutable-locations) is not the answer here,
unlike the closure case: cells are confined to the thread
that created them, so one made outside the loop is unreachable from a
worker.

Any value holding a mutable location is confined to its thread the
same way: reading one from a `spawn`ed task fails, and `chan.send`
refuses to carry it. A streaming
[CSV or JSON-lines reader](ods.md#files-larger-than-memory) is one — it
holds a file position — so read on the thread that opened it and send
the *rows*.

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
fine). A value no arm matches raises a runtime error ("Pattern match
failed"), as a `let` whose pattern does not match does; end with `_` when
the arms are not exhaustive (`olang check` warns on a `match` over an
enum that leaves a variant out).

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
| Literal | `0`, `"yes"`, `true` | that exact value, of the same kind: `5` matches the Int 5 and not the Float 5.0, as `[5] == [5.0]` is false |
| Or | `1 \| 2 \| 3` | any alternative |
| Range | `1..10`, `'a'..'z'` | an Int within the range; for a character range, a one-character String within it (`'a'..='z'` does not match `"zebra"`) |
| Wildcard | `_` | anything, binds nothing |
| Identifier | `x` | anything, binds it as `x` |
| Tuple | `(a, b)` | a 2-tuple, binding parts |
| List | `[a, b]`, `[h, ...t]` | exact shape, or head + rest |
| Enum variant | `Some(v)`, `Node(l, r)` | that variant, binding payloads |
| Result | `Ok(v)`, `Err(e)` | result values |
| Struct | `Point { x, y }` | struct fields (shorthand or `field: pat`) |
| Record | `{ kind: k }`, `{}` | a record — a map, an object, a parsed JSON object or a struct — having those keys; `{}` names none and matches any record, never a List, a String, `()` or a number |
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

A pattern binds each name once: `(x, x)` is an error (compare two parts
with a guard, `(a, b) if a == b`), and the alternatives of an
or-pattern bind the same names, so the arm reads a name whichever
alternative matched: `(1, y) | (y, 2)` is accepted, `(1, y) | (z, 2)` an
error. Both are reported before the program runs.

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
ones. Lambdas take defaults with the same rules.

A parameter may **destructure** its argument, as a `let` does: a tuple,
a list or an object pattern where a name goes. The pattern binds at the
top of the body, so an argument it does not match raises as that `let`
would. An annotation and a default apply to the whole argument:

```olang
let pairs = [(1, 2.0), (2, 3.0)]
println(map(pairs, ((i, w)) => i * w))   // [2.0, 6.0]
fn dot((a, b), (c, d)) = a * c + b * d
fn first_and_rest([h, ...rest]) = `${h} then ${len(rest)}`
fn area({ w, h }: { w: Int, h: Int }) = w * h
println(`${dot((1, 2), (3, 4))} ${first_and_rest([5, 6, 7])} ${area({ w: 2, h: 3 })}`)   // 11 5 then 2 6
```

A pattern parameter has no name of its own, so it is not passed by name.

A builtin takes names when its module registers its parameters: the
data stack (`ods`, `stats`, `plot`, `learn`), a Date's functions, and
every stdlib function with an options map (`proc.spawn`, `gui.open`,
`fs.scan`, …). The names map onto positions before the call, so the
function itself is unchanged. A short call takes the registered
defaults (`ods.head(f)` is `ods.head(f, 10)`), and a name that is no
parameter goes into the function's trailing options map, merged over a
map given by position. `:help` and the editor show each signature:

```olang
let s = ods.series([3.0, 1.0, 4.0, 1.0, 5.0])
println(to_string(ods.rolling(s, window: 2, agg: "max")))
println(to_string(stats.norm.cdf(1.0, std: 2.0)))     // mean defaults to 0.0
let chart = plot.line(s, s, title: "doubled", width: 400)
println(to_string(str.contains(chart, "<svg")))
```

An unknown name raises, naming the ones the function takes; a builtin
with no registered parameters (`map`, `str.pad_start`) takes its
arguments by position, and a name given to one is an error naming it. Functions with
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

A lambda bound by `let` may call itself by the binding's name, when no
earlier binding of that name is in scope (one that is, the lambda
captures, as it captures any name). The name is the lambda's own
wherever it is later called, as a nested `fn`'s is:

```olang
fn countdown_from(n) = {
    let go = (k) => if k == 0 => "liftoff" else => go(k - 1)
    go
}
println(countdown_from(3)(3))   // liftoff
```

Functions are first-class values: store them in variables, lists, maps, and
object fields; pass and return them freely:

```olang
let ops = #{ "inc": (x) => x + 1, "dbl": (x) => x * 2 }
let f = ops.dbl
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

The right side may be a bare function name (called with one argument), a
call with extra arguments (the piped value is prepended), or any other
expression that evaluates to a function — a lambda in parentheses, or an
element of a list or map of handlers:

```olang
fn add(a, b) = a + b
println(to_string(5 |> add(3)))
println(to_string(5 |> ((x) => x * 2)))
let steps = [(x) => x + 1, (x) => x * 10]
println(to_string(5 |> steps[1]))
```

Note the asymmetry: `x |> f(a)` is `f(x, a)`, a call with the piped value
prepended — so to pipe into the *result* of a call, bind it first
(`let f = pick(mode)` then `x |> f`).

A lambda may also be a stage without parentheses. An unparenthesized
lambda stage ends at the next `|>`, however its body ends: the `|>`
begins the next stage of the pipeline, not a pipeline inside the
lambda's body. This holds when the body ends with an `if` whose last
branch is not a block:

```olang
fn wrap(x) = [x]
println(5 |> (v) => if v > 3 => v else => 0 |> wrap)   // [5]
println(1 |> (v) => if v > 3 => v else => 0 |> wrap)   // [0]
let total = #{}
    |> (m) => map_set(m, "a", 1)
    |> (m) => map_set(m, "b", 2)
    |> map_len
println(total)   // 2
```

A pipeline meant to stay inside the lambda is written in its body's
parentheses or braces: `xs |> (v) => (v |> f)`. A lambda that is not a
stage, such as `let h = (v) => v |> f` or an argument to `map`, keeps
its whole expression as its body.

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

**Field values are checked against their declared types**, as a
parameter's are. A value of the wrong type is an error naming the field,
the type, what was expected, and what was supplied. The match is exact:
an `Int` value does **not** satisfy a `Float` field (there is no
widening at construction). A list or map field checks that the value is
a List or a Map (its elements are the static checker's concern), a
tuple field its length, a function field that the value is callable
with the annotation's parameter count, `()` that it is Unit. A field
whose type is a generic type parameter stays dynamic and accepts any
value, as do all fields of an anonymous `{ ... }` object.

```olang no-run
type Point = struct { x: Int, y: Int }
Point { x: 1 }                    // error: missing field 'y'
Point { x: 1, y: 2, z: 3 }        // error: no field 'z'
NeverDeclared { s: 42 }           // error: unknown struct type
Point { x: "hi", y: 2 }           // error: field 'x' of Point expects Int, got String
type Pair = struct { both: (Int, Int) }
Pair { both: (1, 2, 3) }          // error: field 'both' of Pair expects (Int, Int), got a 3-tuple

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

### Record shape aliases

`type Name = <annotation>` names an annotation, and the name is usable
wherever an annotation goes: a parameter, a `let`, a return type, a
list or map element, a struct field. Its common form names a record
shape, so every function handling the same record checks against one
declaration:

```olang
type Task = { title: String, done: Bool }
type Tasks = [Task]
fn title_of(t: Task) -> String = t.title
fn first_title(ts: Tasks) = title_of(head(ts))
let t: Task = #{ "title": "ship", "done": false }
println(first_title([t]))
```

An alias is not a runtime type: `t: Task` is checked as `{ title:
String, done: Bool }` is — a record shape is the checker's, unchecked
at runtime, while `Tasks` enforces "a List" at the call — and `typeof`
never answers the alias. The checker reads the shape through the alias
at every `map_get`, field, and index site with a literal key, in a
macro's output too, so a macro that splices `type Task = { ... }`
beside each declaration gives every hook's record a checked shape. An
alias may be declared below its first use.

### What type declarations cannot express

Union type *declarations* (`type Id = Int | String`) are aliases of the
union annotation, checked as `x: Int | String` is ([Types](types.md));
a discriminated union with payloads is still an enum. An enum variant
takes positional fields only: `C { n: Int }` is a parse error (declare
`C(Int)`, or a struct type and `C(Point)`). Intersection annotations
(`A & B`) are not accepted; writing one is a parse error.

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
is the field even if a trait method of the same name exists. A map's
key wins the same way: `m.value()` calls the function stored under the
key `"value"` when the map has it, and the method otherwise.

### Methods of native values

A native value's methods are its module's functions: `s.fill_null(0.0)`
is `ods.fill_null(s, 0.0)`, `d.weekday()` is `dates.weekday(d)`. One
function, three spellings — `ods.sum(s)`, `s |> ods.sum`, `s.sum()` —
so the reference, `:help` and the checker describe one thing. A
function is a method of the values its first parameter takes (a
Series' methods are the column functions, a Frame's the table verbs),
and a Series also borrows `stats` and `plot` (`s.describe()`,
`s.hist()`). A method takes names as its function does:

```olang
let f = ods.read_csv("city,price\nOslo,4.0\nLima,2.5\nRome,3.5\n")
println(to_string(f.sort_by("price", descending: true).head(2).n_rows()))
println(to_string(f["price"].mean()))
```

The order a method resolves in:

1. a struct's field or a map's key of the name (as above);
2. an `impl` for the value's type — `impl Summary for Series` wins over
   the module's function of the same name;
3. the value's module, then the modules it borrows from.

A fit from `learn` is a map whose `model` is a native model, and it
takes the model's methods: `fit.predict(test)` is
`learn.predict(fit, test)`. A method the value does not have raises
naming its type.

A field read on a native value is the value's own: `f.price` is the
column `f["price"]`. A method's name always means the method, so a
column named like one (`sum`, `head`) is read with `f["sum"]`, and
`f.head` without a call raises saying it is a method.

```olang
let f = ods.read_csv("price,sum\n4.0,1\n2.5,2\n")
println(to_string(f.price.max()))     // the column
println(to_string(f["sum"].max()))    // a column named like a method
println(to_string(len(f)))            // len counts a Frame's rows, a Series' values
```

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
has no construct that catches one mid-expression.

What exists instead is a boundary you place yourself: `attempt(f)` calls
`f()` and answers `Ok(value)`, or `Err(message)` when a runtime error
ended the call. Control flow is not failure — a `return` or a `?` inside
`f` passes through — so `attempt` is for the request handler that must
answer 500 rather than end the process, and the loop that must survive
one bad record.

```olang
println(show(attempt(() => map_get(5, "k"))))   // Err("map_get: first argument must be a map or object")
println(show(attempt(() => 41 + 1)))            // Ok(42)
```

A library raises one of its own with `raise(message)`: a runtime error
whose message is `message`, word for word. It is for the caller that
used the library wrongly — an unknown option, a missing node in a test
driver — which, by the rule above, is a bug and not a condition to
handle; a failure the caller could handle is a `Result`. `attempt`
answers it as `Err(message)`, and uncaught it is reported as any
runtime error is (`Runtime error: message`).

```olang
fn widget(kind) = if kind == "button" || kind == "label" => #{ "kind": kind } else => raise("widget: no kind " + show(kind))
println(show(widget("label")))                     // #{"kind": "label"}
println(show(attempt(() => widget("dial"))))       // Err("widget: no kind dial")
```

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
println(to_string(jobs |> map((j) => match task.join(j) { Ok(v) => v, Err(e) => -1 })))
```

Anywhere else, a runtime error stops the program.

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

The same program reports the same span, frames, and message whether it
ran on the interpreter, the bytecode VM, or through a JIT
deoptimization ([the execution tiers](ovm.md#correctness-policy)). One
shape note:
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
finishes and answers a `Result`: `Ok(value)` with what it produced
(even when that value is itself an `Err`), or `Err(message)` when it
raised:

```olang
fn heavy(n) = {
    time.sleep(20)
    n * 2
}
let a = spawn heavy(10)
let b = spawn heavy(11)
println(to_string(unwrap(task.join(a)) + unwrap(task.join(b))))   // both ran concurrently
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
println(to_string(jobs |> map((j) => unwrap(task.join(j)))))   // [2, 4, 6]
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

**Failure is a value.** A task that raises yields `Err(message)` from
`task.join` rather than aborting the program, so worker failure is
handled with the ordinary Result toolkit and one bad task never kills
the batch. A task that *returns* an `Err` joins as `Ok(Err(…))`, so the
two are never confused:

```olang
fn work(n) = if n == 1 => raise("task died") else => n * 10
let jobs = [spawn work(0), spawn work(1), spawn work(2)]
let results = jobs |> map((j) => match task.join(j) { Ok(v) => v, Err(e) => -1 })
println(to_string(results))   // [0, -1, 20]
```

### Bounding the wait

`task.join_timeout(t, ms)` returns `Ok` of what `task.join` would answer
if the task finished within `ms` milliseconds (`Ok(Ok(v))`, or
`Ok(Err(e))` when it raised) and `Err("timed out")` otherwise. The
budget can also be given as `#{ timeout_ms }`.

It bounds how long you *wait*, not how long the task *works*: olang
has no task cancellation. A timed-out task runs to completion and its
result stays collectible from the same handle:

```olang
fn slow(n) = { time.sleep(80); n }
let t = spawn slow(7)
println(show(task.join_timeout(t, 5)))                      // Err("timed out")
println(show(task.join_timeout(t, #{ "timeout_ms": 5000 })))   // Ok(Ok(7)) — it kept running
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
println(to_string(total) + " (" + show(unwrap(task.join(worker))) + ")")
```

### Data parallelism

For the common case — the same function over every element — reach for
`par_map`, `par_filter`, or `par for` instead of spawning by hand. They
run on a worker pool, carry the same snapshot semantics as `spawn`, and
give the results their sequential counterparts give, in the same order:

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

### Sharing state under `http.serve`

Cells are pinned to the thread that created them, and every request
`http.serve` handles runs on a worker thread. The natural first draft —
a module-level cell as a cache, read and written from handlers — is
therefore refused at run time ("cell escaped its thread"), correctly and
with a hint pointing at channels. The pattern that replaces it is a
**channel service**: one task owns the state, the workers talk to it
over channels, and every rebuild becomes single-flight because only the
owner performs it.

```olang no-run
// One owner. Requests arrive on `asks`; each carries its own reply channel.
let asks = chan.new()
spawn {
    let state = cell(#{ "built_at": 0, "value": () })
    loop {
        let ask = unwrap(chan.recv(asks))
        match ask.kind {
            "get" => chan.send(ask.reply, cell.get(state).value),
            "rebuild" => {
                cell.set(state, #{ "built_at": time.monotonic_ms(), "value": expensive() })
                chan.send(ask.reply, true)
            }
        }
    }
}

// Workers never touch the cell; they ask.
fn cached_value() = {
    let reply = chan.new()
    chan.send(asks, #{ "kind": "get", "reply": reply })
    unwrap(chan.recv(reply))
}

http.serve(8080, (req) => http.response(200, show(cached_value())))
```

The owner task is the only place the state changes, so no two workers
ever rebuild it at once, and a slow rebuild queues the askers rather
than duplicating the work. The same shape serves a rate limiter, a
session table, or an in-memory index — anything that would be a
module-level mutable in a single-threaded server.

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
use lib.geometry { area as region }    // aliased: bound as `region`
use lib.geometry { * }                 // wildcard: everything shared
use lib.geometry                       // sugar for the wildcard form
use lib.geometry {}                    // the namespace alone: geometry.area
use gg                                 // a stdlib module: its namespace alone

// Long import lists may span lines and end with a trailing comma:
use lib.geometry {
    area,
    Shape,
}
```

An aliased item looks up the export under its source name and binds
only the alias — `area` itself is not brought into scope by
`area as region`. `as` is contextual: it is a keyword only inside a
`use` list.

Importing a shared **enum type also imports its variant constructors**,
so `Sq(2)` constructs in the importer. A `use` also binds the module's
name as a namespace (`geometry.area(3, 4)`), and the bare form imports
everything shared *and* binds the namespace — except for a stdlib module
(`use gg`, `use ml`, `use term`, `use str`), whose bare form binds the
namespace alone (`gg.plot(...)`), as `use m {}` does for any module:
a stdlib module's short names (`plot`, `summary`, `folds`, `table`)
would otherwise land on the program's own and on other modules'.

An import never replaces what it was not asked for:

- **A wildcard skips a stdlib module's name** — `use lib.charts { * }`
  sharing a `plot` leaves `plot` the plot module (reach the export as
  `charts.plot`) — unless what it binds is itself that kind of module.
- **A wildcard skips a name the program already binds** in that scope:
  after `let folds = 3` (or a notebook param `folds`), `use lib.cv { * }`
  leaves `folds` as 3. A name an earlier import bound is the import's,
  and a later import (or the same `use` run again) rebinds it.
- **A listed name the program already binds is an error** naming it:
  `let x = 1` then `use lib.m { x }` raises — list it under another name
  (`x as m_x`). A listed stdlib module name binds: listing it is the
  choice (`use lib.effects { task }` is how Loom wraps `task`).

`olang check` says each of these where it can see both sides: a wildcard
export it skips (a warning) and a listed name that would replace the
file's own (an error). It also knows each stdlib module's members: a
member a module lacks — lowercase `pi` read off `math`, whose constant
is `PI`; a `trimm` called on `str` — is an error naming the nearest, and `gui.turn == ()` — asking whether this build has a
member — is left alone.

A namespace reads like any record: calling a member it does not have raises, naming the nearest
(`geometry.aera(3, 4)` — did you mean `area`?), and reading one is `()`,
as `map_get` answers — how a program asks a module it loaded at run time
whether it has a hook (`if plugin.init == () => …`). Because a later wildcard
import rebinds any earlier import of the same name,
`olang check` warns when it can see the collision; the alias form is
the resolution that keeps both names.

`share use other { name }` re-exports an import (transitive
sharing); `share use other { name as alias }` re-exports it under the
alias.

### How `use` resolves

Paths are dot-separated. The first segment decides where the search
goes, in this order:

1. **Embedded stdlib modules and packages** (`colx`, `mathx`; `cli`,
   `term`, `ui`, `viz`, `dash`) — olang source shipped
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
scope — no `use` needed. Embedded olang-source modules and packages
(`colx`, `mathx`, `cli`, `term`, `ui`, `viz`, `dash`) are imported with `use colx` and ship inside the binary.

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
    assert_close(0.1 + 0.2, 0.3, 0.000001)
    assert_eq(
        add(20, 22),
        42,
        "assertion arguments may span lines"
    )
}
println("tests passed")
```

A test block is its own scope, as a function body is: a `let` inside
it ends at the closing brace, so a test's `let fs = …` does not replace
the `fs` module for the blocks that follow.

These assertions (`assert`, `assert_eq`, `assert_ne`, `assert_true`,
`assert_false`, and `assert_close(actual, expected, tolerance)` for
numeric comparisons within a tolerance — each with an optional trailing
message) are language-level forms: they raise on failure, which is what
makes a failing test abort. `assert_close` accepts `Int` and `Float`
freely, fails when either side is `NaN`, and rejects a negative
tolerance.
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

Unannotated code is fully dynamic. Container annotations check
shallowly at runtime (`List<Int>` promises "a List"; element types are
[`olang check`](tooling.md#olang-check)'s), a tuple annotation checks
the length, `()` accepts only Unit, `Int` and `Float` are strict (an
`Int` does not satisfy `Float`), and generic type parameters are erased
(their trait *bounds* are checked).
[Runtime enforcement](types.md#runtime-enforcement) states each rule.

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
(`"open" | "done"` is a lightweight enum), record shapes
`{ name: Type, ... }`, generic applications `Name<T>`, and `()` unit.

A record shape names the keys a map or anonymous record carries. The
runtime does not enforce it — a map's keys are data — but
[`olang check`](tooling.md#olang-check) reads it at every `map_get`,
field, and index site with a literal key, so `r.summry`
against `r: { summary: String, count: Int }` is reported with the key
it resembles rather than answering Unit at runtime.

## Appendix: keywords and grammar

Reserved keywords — not usable as identifiers:

```text
fn let if else match for while loop break continue return
true false struct enum
await yield async defer const pub
```

Twenty-one words, and that is the whole list. The last six have no
meaning: they are reserved, and a program that names something with
one is refused with an error that says so. A member after `.` and a key are
not names, so `http.defer()` and `{ "const": 1 }` are untouched.

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
| `type` | *(contextual)* Declare a `struct`, `enum`, or alias type |
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
`Err` and `Result` are ordinary names with built-in meaning rather than
reserved words, as are `try`, `catch` and `Promise`.

`spawn`, `assert`, `assert_eq` and `assert_ne` are **statement-position
keywords**: at the start of an expression they are the language's own
form (`spawn expr`, `assert(cond)`), whatever a program binds to the
name — `fn spawn(x) = x` then `spawn(3)` starts a task. Elsewhere they
are ordinary names. Avoid binding them.

`@name(args)` invokes a [macro](macros.md) in expression position, and
`@name` above a `type`, `fn`, or `let` declaration decorates it; both
are resolved and gone before the program runs.

Statement separators are newlines or `;`. Comments are `//` to end of
line. A leading `#!` line (`#!/usr/bin/env olang`) is host metadata,
not syntax: mask-skipped by the parser with line numbers preserved, so
`chmod +x script.ol` just works and errors still point at the right
line.

The full grammar is [`grammar.pest`](../grammar.pest) at the repository
root, in pest's PEG syntax; the parser is generated from it. The precedence table in
[Operators](#operators-and-precedence) mirrors its expression hierarchy.

---

Next: [the standard library reference](stdlib.md), or how it all runs in
[Internals](internals.md).
