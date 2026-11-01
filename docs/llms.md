# olang for LLMs

This chapter is for a language model, or a person in a hurry, who has
to write correct olang from the documentation alone. It lists the rules
that differ from Rust, Go and Python, the mistakes those habits lead
to, and a short working program for each everyday task: printing and
files, a type with methods, walking collections, handling errors, and
tests. Every olang block here runs exactly as shown.

olang is a programming language for small tools, data work and web
apps. It is expression-based and functional: values never change, a
block's last expression is its value, and failures that can be expected
are `Result` values rather than exceptions.

Part of [the olang book](README.md) · [Common pitfalls](pitfalls.md) ·
[Language reference](language.md) · [Standard library](stdlib.md) ·
[Error messages](errors.md).

## Table of contents

- [Running a program](#running-a-program)
- [What differs from Rust, Go and Python](#what-differs-from-rust-go-and-python)
- [Values, bindings and functions](#values-bindings-and-functions)
- [Decisions and loops](#decisions-and-loops)
- [Lists](#lists)
- [Strings and tuples](#strings-and-tuples)
- [Maps](#maps)
- [Structs, enums and methods](#structs-enums-and-methods)
- [Errors](#errors)
- [Files](#files)
- [Tests](#tests)
- [Modules](#modules)
- [Five complete programs](#five-complete-programs)

## Running a program

A program is a file ending in `.ol`. Its top-level statements run from
top to bottom; there is no `main` function to write.

```bash
olang app.ol arg1 arg2    # run it; os.args() is ["app.ol", "arg1", "arg2"]
olang test                # run every test block under the current folder
olang check               # report mistakes it can prove, without running
```

## What differs from Rust, Go and Python

Each row is a habit from another language and what olang wants instead.
The sections after the table show each one working.

| Instead of | Write | Note |
|---|---|---|
| `if x > 0 { a } else { b }` | `if x > 0 => a else => b` | `if` takes `=>` arms; `if c => { ... }` for several statements |
| `c ? a : b` | `if c => a else => b` | there is no ternary operator; `if` is an expression |
| `and`, `or`, `not` | `&&`, `\|\|`, `!` | a condition must be a Bool: no truthiness |
| `fn f(x) { ... }` | `fn f(x) = { ... }` or `fn f(x) = expr` | the body follows `=` |
| `return x` at the end | `x` | a block's last expression is its value; `return` leaves early |
| `x += 1` | `x = x + 1` | no compound assignment |
| `let x = 0` then `x = 1` | `let mut x = 0` then `x = 1` | a plain `let` cannot be reassigned |
| `null`, `None`, `nil` | `()` | absence is the unit value; test it with `x == ()` |
| `{"a": 1}` | `#{ "a": 1 }` | quote the keys: in `#{ a: 1 }`, `a` is a variable |
| `f"n={n}"`, `format!` | `` `n=${n}` `` | backtick templates; no escapes inside, no nesting |
| `"n=" + n` | `"n=" + show(n)` | `+` joins two strings only |
| `xs.len()`, `s.split(",")` | `len(xs)`, `str.split(s, ",")` | lists, strings and maps have no methods |
| `xs.map(f)` | `map(xs, f)` or `xs \|> map(f)` | data goes first, so calls chain with `\|>` |
| `xs.push(v)`, `append` | `xs = xs + [v]` | lists never change in place |
| `xs[1:3]`, `xs[-1]` | `take(skip(xs, 1), 2)`, `last(xs)` | no slicing; a negative index raises |
| `m[k] = v` | `m = map_set(m, k, v)` | maps never change in place |
| `for k, v in m.items()` | `for (k, v) in entries(m)` | `for (k, v) in m` is a type error |
| `impl Point { fn ... }` | `trait T { ... }` and `impl T for Point { ... }` | methods come only from traits |
| `P { ..p, x: 1 }` | `map_set(p, "x", 1)` | a struct literal takes no spread |
| `try`/`except`, `throw` | `Result`, `match`, `?`, `raise` | expected failures are values |
| `int("4")`, `"4".parse()` | `str.parse_int("4")` | answers `Ok(4)` or `Err(...)` |
| `int(2.7)`, `str(x)` | `to_int(2.7)`, `to_string(x)` | `show(x)` is the same text |
| `lambda x: x + 1`, `\|x\| x + 1` | `(x) => x + 1` | |
| `print(x)` | `println(x)` | `print` writes without a newline |

Semicolons are optional; a new line ends a statement. Integer division
truncates (`7 / 2` is `3`), and integer overflow is an error rather than
a wrap.

## Values, bindings and functions

```olang
let name = "ada"                      // immutable
let mut count = 0                     // can be pointed at a new value
count = count + 1

fn square(x) = x * x                  // one expression
fn describe(n) = {                    // a block: its last line is the value
    let kind = if n % 2 == 0 => "even" else => "odd"
    `${n} is ${kind}`
}
fn greet(who, greeting = "hello") = `${greeting}, ${who}`   // a default

println(square(4))                    // 16
println(describe(7))                  // 7 is odd
println(greet(name))                  // hello, ada
println(greet(name, greeting: "hi"))  // hi, ada
let add = (a, b) => a + b             // a lambda
println(add(count, 1))                // 2
```

Values are `Int`, `Float`, `String`, `Bool`, `()`, lists `[1, 2]`,
tuples `(1, "a")`, maps `#{ "k": v }`, objects `{ name: v }`, and the
structs and enums you declare. `typeof(v)` names the type.

## Decisions and loops

```olang
let t = 23.5
let feel = if t < 10.0 => "cold" else if t < 20.0 => "mild" else => "warm"
println(feel)                         // warm

let label = match 3 {
    0 => "none",
    1 => "one",
    n if n < 0 => "negative",
    _ => "many"
}
println(label)                        // many

let mut total = 0
for i in 0..5 {                       // 0, 1, 2, 3, 4
    if i == 3 => continue
    total = total + i
}
println(total)                        // 7

let mut n = 1
while n < 100 { n = n * 2 }
println(n)                            // 128
```

`match` arms are separated by commas or new lines and tried in order.
A bare lowercase name in a pattern binds whatever is there, so end a
`match` with `_` (or a name) to cover the rest; a value that matches no
arm is an error.

## Lists

```olang
let xs = [5, 3, 8, 1]
println(len(xs))                                  // 4
println(xs[0])                                    // 5
println(first(xs))                                // 5
println(last(xs))                                 // 1
println(sort(xs))                                 // [1, 3, 5, 8]
println(xs |> filter((x) => x > 2) |> map((x) => x * 10))   // [50, 30, 80]
println(fold(xs, 0, (acc, x) => acc + x))         // 17
println(sum(xs))                                  // 17
println(take(skip(xs, 1), 2))                     // [3, 8]
println(contains(xs, 8))                          // true
println(xs + [9])                                 // [5, 3, 8, 1, 9]
for (i, x) in enumerate(["a", "b"]) { println(`${i}: ${x}`) }
```

`first` and `last` answer `()` for an empty list; `min`, `max`,
`average`, `head` and `tail` raise on one, so check `len` first.
`col.sort_by(xs, f)` sorts by a key, and `reverse`, `join`, `range(a,
b)` and `zip` are there as you would expect.

## Strings and tuples

String functions live in `str` and take the string first. Indexes count
characters from 0, and a search that finds nothing answers `()`.

```olang
let s = "  Hello, World  "
let t = str.trim(s)
println(t)                                // Hello, World
println(str.to_upper(t))                  // HELLO, WORLD
println(str.to_lower(t))                  // hello, world
println(str.split(t, ", "))               // ["Hello", "World"]
println(str.contains(t, "World"))         // true
println(str.starts_with(t, "Hell"))       // true
println(str.ends_with(t, "!"))            // false
println(str.replace(t, "World", "olang")) // Hello, olang
println(str.substring(t, 0, 5))           // Hello
println(str.index_of(t, "W"))             // 7
println(str.index_of(t, "z"))             // ()
println(len(t))                           // 12
println(str.fmt("{} + {} = {}", 1, 2, 3)) // 1 + 2 = 3
let (word, n) = ("tea", 3)
println(`${word} x${n}`)                  // tea x3
```

Double-quoted strings take escapes (`"\n"`, `"\t"`); backtick templates
take none. A tuple is a fixed group, read with `t[0]` or taken apart
with `let (a, b) = t`, in a `for`, or in a `match` arm.

## Maps

A map's keys are strings, kept in the order they were first set. A map
never changes: `map_set` answers a new one. Reading a missing key gives
`()`, never an error, and `??` supplies a default.

```olang
let ages = #{ "ada": 36, "alan": 41 }
println(ages.ada)                     // 36
println(ages["alan"])                 // 41
println(ages.grace ?? 0)              // 0
println(map_has_key(ages, "ada"))     // true

let older = map_set(ages, "grace", 85)
println(map_len(ages))                // 2
println(map_len(older))               // 3

for (name, age) in entries(older) {
    println(`${name} is ${age}`)
}
println(map_keys(older))              // ["ada", "alan", "grace"]
```

Walk a map with `entries(m)`, which gives `(key, value)` pairs, or with
`map_keys(m)` and `map_values(m)`. `for (k, v) in m` does not work: the
program stops with `Type error: cannot iterate over a Map`.

To build a map in a loop, rebind a `let mut` name to the new map each
time:

```olang
let words = str.words("the cat and the hat")
let mut counts = #{}
for w in words {
    counts = map_set(counts, w, (counts[w] ?? 0) + 1)
}
println(counts)                       // #{"the": 2, "cat": 1, "and": 1, "hat": 1}
```

## Structs, enums and methods

A struct is a record type with fixed fields, checked when it is built.
Methods come only from a `trait` and an `impl Trait for Type`. A bare
`impl Type { ... }`, as in Rust, is not supported and olang refuses it;
when one type needs methods, declare a trait just for it. A method
receives the value as `self` and cannot change it, so a method that
"updates" returns a new value.

```olang
type Account = struct { owner: String, balance: Int }

trait Banking {
    fn deposit(self, amount) -> Account
    fn describe(self) -> String
}

impl Banking for Account {
    fn deposit(self, amount) = Account { owner: self.owner, balance: self.balance + amount }
    fn describe(self) = `${self.owner} has ${self.balance}`
}

fn open(owner) = Account { owner: owner, balance: 0 }

let acct = open("ada").deposit(50).deposit(25)
println(acct.describe())              // ada has 75
println(acct.balance)                 // 75
println(typeof(acct))                 // Account
```

A struct literal takes every field and no spread (`Account { ...a }` is
refused); `map_set(acct, "balance", 0)` answers a copy with one field
changed. A trait method may have a default body in the trait, which an
impl can replace.

An enum is a fixed set of variants, each optionally carrying data. Its
variants are used by their bare names:

```olang
type Shape = enum { Circle(Float), Rect(Float, Float), Dot }

fn area(s) = match s {
    Circle(r) => 3.14159 * r * r,
    Rect(w, h) => w * h,
    Dot => 0.0
}
println([Circle(1.0), Rect(2.0, 3.0), Dot] |> map(area) |> sum)   // 9.14159
```

## Errors

A function that can fail returns `Ok(value)` or `Err(error)`. The
standard library does this for anything that touches the outside world
or parses text: `fs.read_file`, `str.parse_int`, `json.parse`,
`json.stringify`. Take a `Result` apart with `match`, `unwrap_or`, or
`?`, which inside a function returns an `Err` to the caller at once.

```olang
fn parse_age(text) = {
    let n = str.parse_int(str.trim(text))?          // an Err returns here
    if n < 0 => return Err("age cannot be negative")
    Ok(n)
}

for input in ["42", " 7 ", "-3", "old"] {
    match parse_age(input) {
        Ok(age) => println(`age ${age}`),
        Err(e) => println(`rejected: ${e}`)
    }
}
println(unwrap_or(parse_age("x"), 0))               // 0
println(is_ok(parse_age("5")))                      // true
```

`unwrap(r)` takes the value out and stops the program on an `Err`, so
keep it for values that cannot fail. `unwrap_or` is only for a
`Result`; for a missing map key (`()`), use `??`.

Declared errors carry structure, and match like enum variants:

```olang
error LookupError {
    NotFound,
    Invalid: { reason: String }
}

fn find(id) = {
    if id < 0 => return Err(Invalid("ids are positive"))
    if id > 100 => return Err(NotFound)
    Ok(`user-${id}`)
}

for id in [7, 500, -1] {
    println(match find(id) {
        Ok(user) => user,
        Err(NotFound) => "no such user",
        Err(Invalid(reason)) => `bad id: ${reason}`
    })
}
```

A bug, as opposed to an expected failure, is a runtime error: a type
error, a division by zero, an index out of range, or your own
`raise("message")`. It stops the program with its message and the call
stack. `attempt(f)` turns one into an `Err` at a boundary you choose:

```olang
fn checked(n) = if n > 10 => raise(`too big: ${n}`) else => n
println(attempt(() => checked(3)))                  // Ok(3)
println(attempt(() => checked(30)))                 // Err("too big: 30")
```

[Error messages](errors.md) lists the messages olang gives.

## Files

`fs` reads and writes whole files. Every call that touches the disk
answers a `Result`.

```olang
let path = fs.join([os.temp_dir(), "olang-llms-notes.txt"])

unwrap(fs.write_file(path, "milk\neggs\n"))        // replaces the file
unwrap(fs.append_file(path, "bread\n"))

match fs.read_file(path) {
    Ok(text) => {
        let items = str.lines(str.trim(text))
        println(`${len(items)} items: ${join(items, ", ")}`)
    },
    Err(e) => println("could not read: " + e)
}
println(is_err(fs.read_file("/no/such/file")))      // true
unwrap(fs.remove_file(path))
```

`fs.exists`, `fs.is_file` and `fs.is_dir` answer a Bool, `fs.list_dir`
and `fs.walk` list a folder, and `fs.join`, `fs.dirname`,
`fs.basename` and `fs.ext` work on paths without touching the disk.
`json.parse` and `json.stringify`, `csv` and `toml` read and write those
formats. Standard input is `os.read_line()`, `os.stdin()` and
`os.stdin_lines()`.

## Tests

A `test "name" { ... }` block sits next to the code it checks.
`olang test` finds every file with test blocks under a folder, runs
each block, reports each one, and exits with status 1 if any failed.
Running the file with plain `olang file.ol` skips its test blocks.

```olang
fn mean(xs) = if len(xs) == 0 => Err("mean of an empty list") else => Ok(sum(xs) / len(xs))

test "mean of whole numbers" {
    assert_eq(mean([2, 4, 6]), Ok(4))
}

test "mean of floats" {
    assert_close(unwrap(mean([1.0, 2.0, 2.5])), 1.8333, 0.001)
}

test "an empty list is an error" {
    assert_true(is_err(mean([])))
}
```

Saved as `stats.ol`, `olang test stats.ol` prints:

```text
stats.ol
  ✓ mean of whole numbers
  ✓ mean of floats
  ✓ an empty list is an error
────────────────────────────────────────
  3 passed, 0 failed  (1 test file, 0ms)
```

The assertions are `assert(cond)`, `assert_eq(a, b)`, `assert_ne(a, b)`,
`assert_true(x)`, `assert_false(x)` and `assert_close(actual, expected,
tolerance)`, each with an optional last argument, a message. A failed
one fails its block and the run goes on to the next. `olang test
--only "mean"` runs the blocks whose name contains the text.

## Modules

Every `.ol` file is a module. `share` exports a name, and `use` imports
names by path from the project root, with dots for folders:

```olang no-run
// lib/geometry.ol
share fn area(w, h) = w * h

// main.ol
use lib.geometry { area }
println(area(3, 4))
```

The standard library's modules (`str`, `fs`, `json`, `math`, `col`,
`os`, `http`, `db` and the rest) need no `use`. [Packages and
dependencies](packages.md) covers projects and `olang.toml`.

## Five complete programs

Each of these is a whole program, written only from the rules above.

Reading and writing a file:

```olang
let path = fs.join([os.temp_dir(), "olang-llms-greeting.txt"])
println("hello, olang")
match fs.write_file(path, "line one\nline two\n") {
    Ok(_) => println("saved"),
    Err(e) => println(`could not save: ${e}`)
}
let lines = str.lines(str.trim(unwrap(fs.read_file(path))))
for (i, line) in enumerate(lines) {
    println(`${i + 1}: ${line}`)
}
unwrap(fs.remove_file(path))
```

A struct with methods:

```olang
type Rect = struct { w: Float, h: Float }

trait Shape {
    fn area(self) -> Float
    fn scaled(self, k) -> Rect
}

impl Shape for Rect {
    fn area(self) = self.w * self.h
    fn scaled(self, k) = Rect { w: self.w * k, h: self.h * k }
}

let r = Rect { w: 2.0, h: 3.0 }
println(r.area())                     // 6.0
println(r.scaled(2.0).area())         // 24.0
```

Walking a map and a list:

```olang
let prices = #{ "tea": 3, "cake": 5, "soup": 7 }
let order = ["tea", "cake", "tea", "bread"]

let mut bill = 0
for item in order {
    let price = prices[item] ?? 0
    if price == 0 => println(`no price for ${item}`)
    bill = bill + price
}
println(`bill: ${bill}`)

for (item, price) in entries(prices) {
    println(`${item} costs ${price}`)
}
let cheap = entries(prices) |> filter((pair) => pair[1] < 6) |> map((pair) => pair[0])
println(cheap)                        // ["tea", "cake"]
```

Handling errors:

```olang
fn parse_point(text) = {
    let parts = str.split(text, ",")
    if len(parts) != 2 => return Err(`expected x,y but got '${text}'`)
    let x = str.parse_int(str.trim(parts[0]))?
    let y = str.parse_int(str.trim(parts[1]))?
    Ok((x, y))
}

let inputs = ["1,2", "3, 4", "5", "a,6"]
let good = inputs |> map(parse_point) |> filter(is_ok) |> map(unwrap)
println(good)                         // [(1, 2), (3, 4)]
for input in inputs {
    match parse_point(input) {
        Ok((x, y)) => println(`${x} + ${y} = ${x + y}`),
        Err(e) => println(`error: ${e}`)
    }
}
```

A tested function, run with `olang test`:

```olang
fn fizz(n) = match (n % 3, n % 5) {
    (0, 0) => "FizzBuzz",
    (0, _) => "Fizz",
    (_, 0) => "Buzz",
    _ => to_string(n)
}

test "multiples" {
    assert_eq(fizz(9), "Fizz")
    assert_eq(fizz(10), "Buzz")
    assert_eq(fizz(30), "FizzBuzz")
}

test "other numbers" {
    assert_eq(fizz(7), "7")
}
```
