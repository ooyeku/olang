# Common pitfalls

Part of [the olang book](README.md) · [Language reference](language.md).

Every language has a few habits that catch people out at first, and
olang is no exception. Most of them trip up programmers who know Rust,
Python or JavaScript, where something that looks the same behaves a
little differently. This chapter gathers them in one place. Each entry
says what happens, why, and what to write instead. The [Language
reference](language.md) has the full rules.

## Table of contents

1. [Missing map keys return Unit, not a Result](#missing-map-keys-return-unit-not-a-result)
2. [Integer division truncates; overflow is an error](#integer-division-truncates-overflow-is-an-error)
3. [`to_string` prints values, not columns](#to_string-prints-values-not-columns)
4. [A condition is a Bool](#a-condition-is-a-bool)
5. [A line starting with `-x` is a new statement](#a-line-starting-with--x-is-a-new-statement)
6. [Parsing returns a `Result`](#parsing-returns-a-result)
7. [Functions can't be compared with `==`, even inside a list](#functions-cant-be-compared-with--even-inside-a-list)
8. [Closures capture by value, at creation](#closures-capture-by-value-at-creation)
9. [Definition order: functions are hoisted, values are not](#definition-order-functions-are-hoisted-values-are-not)
10. [Indexing: a negative index raises, and `[]` does not slice or write a map](#indexing-a-negative-index-raises-and--does-not-slice-or-write-a-map)
11. [`ods.read_csv` takes text; `ods.read_csv_file` takes a path](#odsread_csv-takes-text-odsread_csv_file-takes-a-path)
12. [Empty-collection builtins raise](#empty-collection-builtins-raise)
13. [`to_string` and `show` agree — a string is already its text](#to_string-and-show-agree--a-string-is-already-its-text)
14. [`_` discards; it never binds](#_-discards-it-never-binds)
15. [A bare name in a `match` pattern is a binding](#a-bare-name-in-a-match-pattern-is-a-binding)
16. [A block's bindings end with the block](#a-blocks-bindings-end-with-the-block)
17. [`os.args()` includes the program path](#osargs-includes-the-program-path)
18. [Template strings take no escape sequences and do not nest](#template-strings-take-no-escape-sequences-and-do-not-nest)
19. [A bare `use` can shadow an earlier named import](#a-bare-use-can-shadow-an-earlier-named-import)
20. [Small syntax reminders](#small-syntax-reminders)

## Missing map keys return Unit, not a Result

`m.key` and `m["key"]` (both exactly `map_get(m, "key")`) on a key that
is not present return **`Unit`** (`()`), not an error and not a
`Result`.

```olang no-run
let m = #{ "a": 1 }
m.z                   // => ()  (Unit), not Err, not None
unwrap_or(m.z, 0)     // TYPE ERROR: unwrap_or's first arg must be a Result
```

`unwrap_or` is for `Result`, so passing it a `Unit` is a type error. The
lookup-with-default is `??` — `m.z ?? 0` is `map_get_or(m, "z", 0)`,
evaluating its default only when it is used — and `map_has_key` guards
the rare case where absent and stored-null must be told apart:

```olang
let m = #{ "a": 1 }
let a = m.a ?? 0
let z = m.z ?? 0
println(show(a) + " " + show(z))   // 1 0
```

`??` replaces only `()`: `m.flag ?? true` keeps a stored `false`.

A misspelt key is `Unit` too: `p.runing` reads nothing rather than
raising (`olang check` reports a key that no map built in the module
has). An object (`{ name: v }`) and a parsed JSON object read the same
way. A declared struct is the record with fixed fields: reading a
field it does not have raises.

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

Series are the exception: `/` between two Int Series (or an Int Series
and an Int) is true division and yields a Float Series, since a ratio
of columns is what data code means. `(s / t).cast("Int")` gives the
integer quotient.

Floats never overflow into `inf`: `1e308 * 10.0`, `math.exp(1000.0)`,
and `str.parse_float("1e999")` are errors (the last an `Err`), so the
computation stops where it went out of range.

## `to_string` prints values, not columns

`to_string` (and `show`) print the shortest text that reads back to
the same float: `12.5`, not `12.50`; `1e21`, not
`1000000000000000000000`. That is right for a value and wrong for a
report, where `12.50` must line up under `3.00` and money wants
separators. `str.fixed(x, digits)` gives exactly `digits` decimals,
never in exponent form and never `-0.00`; `str.thousands(x, digits)`
adds separators.

```olang
println(str.fixed(12.5, 2))              // 12.50
println(str.fixed(-0.001, 2))            // 0.00
println(str.thousands(1234567.891, 2))   // 1,234,567.89
println("$" + str.thousands(-4.5, 2))    // $-4.50 — put the sign first yourself
```

## A condition is a Bool

An `if`, a `while` and a `match` guard take a Bool, as `&&`, `||` and
`!` do: `if xs => …` on a list, a number, a string or `()` raises "a
condition must be a Bool". There is no truthiness; say what is meant.

```olang
let xs = []
let name = ()
println(if len(xs) > 0 => "has items" else => "empty")   // empty
println(if name != () => "named" else => "anonymous")      // anonymous
```

## A line starting with `-x` is a new statement

A binary operator at the start of a line continues the expression above
it — `|>`, `&&`, `||`, `??`, the comparisons, `+`, `*`, `/` and the
bitwise operators — so a long pipeline, condition or sum breaks before
its operator. `-` continues only when it is written as a binary
operator, with a space after it. A `-` glued to its operand is a
negation, and starts a statement of its own:

```olang
let base = 10
let total = base
    - 5                   // continues: `base - 5`
println(total)            // 5
let shown = base
-1                        // a statement of its own, not `base - 1`
println(shown)            // 10
```

`olang check` reports the second case: `-1` computes a value nothing
uses (class `no_effect`). Any expression statement that is only a
literal, a name, or arithmetic or a comparison over them is reported
the same way when it is not a block's last line.

Precedence reads conventionally (`&&` above `||`, arithmetic above
ranges and pipelines), and the bitwise operators take Rust's order:
`1 << 4 * 2` is `1 << 8`. The
[operator table](language.md#operators-and-precedence) has the full
ordering.

```olang
println(show(true || false && false))    // true — && binds tighter, as expected
let xs = [10, 20, 30]
for i in 0..len(xs)-1 { print(show(xs[i]) + " ") }   // 10 20
println("")
println(show(1 << 4 * 2))                // 256 — 1 << (4 * 2)
```

## Parsing returns a `Result`

Unlike a map read (`m.key`), the `str.parse_*` family returns a `Result` you must
consume — `unwrap`, `unwrap_or`, `?`, or `match`.

```olang
println(show(unwrap_or(str.parse_int("42"), 0)))    // 42
println(show(unwrap_or(str.parse_int("oops"), -1))) // -1
match str.parse_float("3.14") {
    Ok(f) => println("got " + show(f)),
    Err(e) => println("bad: " + e)
}
```

## Functions can't be compared with `==`, even inside a list

A function, a builtin and a range have no equality: `f == g` raises, and
so does comparing two lists, maps, records or `Result`s that agree
everywhere except where they hold one (`[f] == [g]`), since the answer
would rest on the function. Two that differ in their data are simply
unequal (`[1, f] == [2, f]` is `false`). Compare the parts that hold data. A `Result` compares
structurally (`Ok(1) == Ok(1)` is `true`).

```olang
let r = str.parse_int("5")
println(r == Ok(5))                                   // true
let handlers = [#{ "name": "a", "run": () => 1 }]
println(map(handlers, (h) => h.name) == ["a"])        // compare the data
```

Two related equality quirks: struct equality includes the type name, so a
`Point` and a `Coord` with identical fields are **not** equal; and while
`1 == 1.0` is `true`, `[1] == [1.0]` is `false` (a structural compare
treats an int element and a float element as different).

## Closures capture by value, at creation

A lambda snapshots the *values* of the local variables it uses when it
is created. Reassigning one of those variables afterward does **not**
change what the closure sees — it kept its own copy.

```olang
fn counter_view() = {
    let mut n = 1
    let get = () => n
    n = 99
    get()
}
println(show(counter_view()))    // 1, not 99 — the closure captured n's value
```

A top-level name is not captured: a function reads it when it runs, so
a top-level `let mut` the program reassigns is seen as it is at the
call ([language.md](language.md#closures-capture-by-value)).

*Assigning* to a captured binding from inside a closure is therefore
an error, not a silent no-op: the write could only reach the closure's
own snapshot, so the program is refused before it runs.

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

A `par for` body is refused for the same reason with a different remedy:
each worker thread runs it against its own snapshot, and a cell made
outside the loop is confined to the calling thread, so it cannot help
there. Results come back as values from `par_map`, or over a channel.

```olang no-run
let mut tally = 0
par for x in xs { tally = tally + 1 }   // refused: the write hits a snapshot

let tally = sum(par_map(xs, (x) => 1)) // one value per item, then combine
```

## Definition order: functions are hoisted, values are not

A function *body* may call a function defined **later** in the file — call
resolution happens when the call runs, by which point everything is
defined:

```olang
fn a() = b() + 1        // b is defined below; fine
fn b() = 10
println(show(a()))      // 11
```

Top-level `fn` declarations are hoisted: `let g = f` or `let x = f()`
above `fn f` works. What is still eager is a value: `let g = h` above
`let h = 1` is `Undefined variable: h`, and inside a function body a
`let` cannot refer to a nested `fn` declared below it.

```olang no-run
let g = h               // ERROR: Undefined variable: h  (h is bound below)
let h = 1
```

Rule of thumb: functions can be referenced from anywhere in the file;
`let` values must be bound before they are read.

## Indexing: a negative index raises, and `[]` does not slice or write a map

`[i]` indexing works on lists, tuples, and strings, from 0, and an
index outside them raises — a **negative index included**: it does not
count from the end, as it does in Python, so `xs[i - 1]` at `i = 0`
stops the program instead of reading the last element. `last(xs)` and
`first(xs)` read the ends, answering `()` for an empty list.

```olang
let xs = [10, 20, 30]
println(show(last(xs)))           // 30
println(show(xs[len(xs) - 2]))    // 20
println(show(last([]) ?? 0))      // 0
```

Three limits of `[]`:

- **No slicing.** `xs[1..3]` is a runtime type error — use `take`/`skip`
  (`take(skip(xs, 1), 2)`).
- **`m[k]` reads, never writes.** On a map, an object, a struct or a
  parsed JSON object, `m[k]` is `map_get(m, k)` (`()` when absent);
  `m[k] = v` is refused. Write a key with `#{ ...m, "k": v }` or
  `map_set`.
- **Element assignment only; no field assignment.** `xs[i] = v` on a
  `let mut` list and `m.k[i] = v` one level into a map work
  ([Lists](language.md#lists)). `p.x = 5`, `m.k = v` and deeper paths do
  not parse; write `#{ ...m, "k": v }`, `map_set`, or `map_set_in`.
  A Frame's column is read with `f["a"]`, and added with
  `f.with({ a: x })` or `f.with_column("a", x)`, not `f["a"] = x`.

## `ods.read_csv` takes text; `ods.read_csv_file` takes a path

The two are one word apart and do different things: `ods.read_csv`
parses CSV *text*, and `ods.read_csv_file` reads a file.

```olang no-run
let sales = unwrap(ods.read_csv_file("data/sales.csv"))     // from disk
let sales = ods.read_csv(unwrap(fs.read_file("data/sales.csv")))  // from text
```

A single-line argument to `read_csv` ending in `.csv`, `.tsv`, or `.txt`
is refused, naming `read_csv_file` and `open_csv`. The same pairing holds
for JSON lines: `ods.read_jsonl` takes text, `ods.read_jsonl_file` takes
a path. The file-reading halves require the `fs` capability; the text
parsers reach nothing and need no grant.

## Empty-collection builtins raise

`head`, `tail`, `min`, `max`, and `average` raise on an empty list rather
than returning a `Result` — guard the length first.

```olang
let xs = [3, 1, 2]
let smallest = if len(xs) > 0 => show(min(xs)) else => "none"
println(smallest)   // 1
```

## `to_string` and `show` agree — a string is already its text

`to_string("ada")` is `ada`, exactly what `show("ada")` and `${...}`
interpolation produce. Strings inside a container print quoted in
every form, because that is the container's
printed shape: `to_string(["a", "b"])` and `show(["a", "b"])` are both
`["a", "b"]`.

```olang
let name = "ada"
println(to_string(name) == show(name))   // true
println(to_string(["a", "b"]))           // ["a", "b"]
```

When the quoted, escaped literal form of a string is wanted — a debug
dump, generated source — `meta.lit` renders it (`meta.lit("ada")` is
`"ada"`), and `json.stringify` does the same in JSON terms.

## `_` discards; it never binds

`_` is the discard everywhere it is accepted — `for _ in ...`, `let _ =
...`, and a `match` arm — and in every case it means *there is no name
here*. Referring to `_` in the body that follows is a parse error, not a
lookup of some anonymous value:

```olang
for _ in range(0, 3) { print("x") }   // fine: the element is discarded
println("")
let _unused = 41                       // a name: bound, and readable
println(show(_unused + 1))
```

A name that *begins* with an underscore and continues (`_unused`,
`_tmp`, `__gen0`) is an ordinary identifier — the convention for a
deliberately unused binding, and the namespace macro-introduced
temporaries use. Only the lone `_` is the wildcard.

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

## Template strings take no escape sequences and do not nest

Backtick strings interpolate with `${...}` but, like raw strings,
process no backslash escapes: `` `a\nb` `` contains a backslash and an
`n`, not a newline. Code that prints control characters — a carriage
return for an in-place progress line, a tab separator — must take them
from a double-quoted string:

```olang no-run
print("\r" + `progress ${done}/${total}`)
```

An interpolation cannot contain another template string. Bind the inner
template to a name first:

```olang
let inner = `${2 + 2}`
println(`the answer starts with ${inner}`)
```

`olang check` warns on an escape-looking `\n`, `\t`, or `\r` inside a
template (spell it `\\n` to mark the two characters deliberate — same
output, no warning), and the parse error a nested backtick produces
says that templates do not nest.

## A bare `use` can shadow an earlier named import

`use module` brings the module's exported names into scope unqualified.
If an earlier `use lib.x { name }` imported the same name, the later
bare `use` rebinds it, and the failure appears at run time inside
whichever function the wrong binding reaches. `olang check` warns when
it can resolve the module and see the collision. The fix that keeps
both names is an alias — `use lib.x { name as other }` — or referring
to the wildcard module's members qualified (`module.name`).

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

---

For the precise semantics behind each entry, see the
[Language reference](language.md); for the standard-library functions
mentioned here, see [the stdlib chapter](stdlib.md).
