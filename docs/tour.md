# A Tour of olang

Fifteen minutes from installation to a working program. Everything here runs
exactly as shown — the code blocks are executed by the test suite.

Part of [the olang book](README.md) ·
[Language](language.md) · [Standard Library](stdlib.md) ·
[Packages](packages.md)

## Installing

Build from source with Cargo:

```bash
git clone https://github.com/ooyeku/olang.git
cd olang
cargo install --path .
```

Two ways to run:

```bash
olang program.ol          # run a file (args after the file reach os.args())
olang                     # start the REPL (:help lists commands)
```

## Hello

```olang
println("hello, olang")
```

Newlines end statements — no semicolons needed. `//` starts a comment.

## Values and bindings

```olang
let name = "ada"
let year = 2026
let ratio = 1.5
let mut count = 0        // `mut` documents intent; bindings are assignable
count = count + 1

println(`${name} in ${year}: ratio ${ratio}, count ${count}`)
```

Template strings (backticks) interpolate with `${...}`. Lists, tuples, maps,
and objects are literals:

```olang
let list = [1, 2, 3]
let pair = (40, 2)
let scores = #{ "ada": 99 }          // map: arbitrary string keys
let user = { name: "ada", age: 36 }  // object: fixed fields, dot access

println(to_string(list[0] + pair[0] + pair[1]))
println(user.name + " scored " + to_string(map_get(scores, "ada")))
```

## Everything is an expression

`if` chains produce values; blocks yield their last expression:

```olang
let hour = 14
let phase = if hour < 12 => "morning"
    else if hour < 18 => "afternoon"
    else => "evening"
println(phase)
```

## Functions and pipelines

Functions are `fn name(args) = expression`; a block is an expression too.
The pipeline operator `|>` feeds a value into the next call's first
argument:

```olang
fn square(x) = x * x

let result = range(1, 11)
    |> filter((x) => x % 2 == 0)
    |> map(square)
    |> fold(0, (acc, x) => acc + x)

println("sum of even squares below 11: " + to_string(result))
```

## Data with shape: structs and enums

Enums construct and pattern-match; `match` must handle every case you send
it:

```olang
type Shape = enum { Circle(Float), Rect(Float, Float) }

fn area(s) = match s {
    Circle(r) => 3.14159 * r * r,
    Rect(w, h) => w * h
}

let shapes = [Circle(1.0), Rect(3.0, 4.0)]
println(to_string(shapes |> map(area)))
```

## Errors are values

Fallible functions return `Ok(...)` or `Err(...)`. Unwrap, default, match,
or propagate with `?`:

```olang
fn parse_pair(a, b) = {
    let x = str.parse_int(a)?      // on Err: return it to the caller
    let y = str.parse_int(b)?
    Ok(x + y)
}

println(to_string(parse_pair("20", "22")))
println(to_string(unwrap_or(parse_pair("20", "oops"), -1)))
```

## A real little program

Word frequency over a string — the shape of many real olang programs:

```olang
let text = "the quick brown fox jumps over the lazy dog the end"

fn bump(acc, w) = {
    let seen = if map_has_key(acc, w) => map_get(acc, w) else => 0
    map_set(acc, w, seen + 1)
}
let counts = str.words(text) |> fold(#{}, bump)

for word in sort(map_keys(counts)) {
    let n = map_get(counts, word)
    if n > 1 => println(word + ": " + to_string(n))
}
```

## Where to next

- [The Language Reference](language.md) — every construct, precisely.
- [The Standard Library](stdlib.md) — every builtin and module.
- [Packages](packages.md) — multi-file programs and dependencies.
- [`examples/`](../examples/) — complete programs: a task CLI, a template
  engine, a regex engine, a parser combinator library, and more. Run them
  all with `cd examples && olang run_all.ol`.
