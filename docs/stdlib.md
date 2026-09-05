# Standard library reference

Part of [the olang book](README.md) · [A tour of olang](tour.md) ·
[Language reference](language.md) · [The data stack](ods.md) ·
[olang in the browser](wasm.md) ·
[Packages and dependencies](packages.md) ·
[Stability and compatibility](stability.md)

This chapter documents the standard library: the global builtins, which are
always in scope, and the modules — those implemented in Rust and those
written in olang and compiled into the binary. As in the
[language reference](language.md), every `olang` block is executed by the
test suite. Blocks marked `no-run` are parse-checked only, because they need
a file system, a network, or a browser.

## Table of contents

- [Conventions](#conventions)
- [Global builtins](#global-builtins)
- [`str` — strings](#str--strings)
- [`col` / `colx` — collections](#col--colx--collections)
- [`math` / `mathx` — mathematics](#math--mathx--mathematics)
- [`bigint` — arbitrary precision](#bigint--arbitrary-precision)
- [Collections, in olang](#collections-in-olang)
- [`json` — JSON](#json--json)
- [`toml` — TOML](#toml--toml)
- [`csv` — CSV](#csv--csv)
- [`re` — regular expressions](#re--regular-expressions)
- [`dates` — dates and times](#dates--dates-and-times)
- [`time` — clocks and sleeping](#time--clocks-and-sleeping)
- [`task` — background threads](#task--background-threads)
- [`cell` — mutable locations](#cell--mutable-locations)
- [`chan` — channels](#chan--channels)
- [`random` — randomness](#random--randomness)
- [`crypto` — hashing and encryption](#crypto--hashing-and-encryption)
- [`bytes` — binary data](#bytes--binary-data)
- [`base64` — base64](#base64--base64)
- [`fs` — file system](#fs--file-system)
- [`meta` — the program as data](#meta--the-program-as-data-the-open-ast)
- [`os` — operating system](#os--operating-system)
- [`proc` — child processes and pipelines](#proc--child-processes-and-pipelines)
- [`cli` — command-line argument parsing](#cli--command-line-argument-parsing)
- [`term` — the terminal toolkit](#term--the-terminal-toolkit)
- [`http` — HTTP](#http--http)
- [`db` — SQLite](#db--sqlite)
- [`dom` — the browser](#dom--the-browser)
- [`testing` — assertions](#testing--assertions)
- [`ods` — Series and Frames](#ods--series-and-frames)
- [`stats` — statistical inference](#stats--statistical-inference)
- [`plot` — charts as SVG text](#plot--charts-as-svg-text)

## Conventions

**Modules are always in scope.** `str.trim(...)`, `json.parse(...)`, and the
rest work without any `use`. The exception is everything written in olang
itself and embedded in the binary — the modules `colx` and `mathx` and
the packages `cli`, `term`, `ui`, `viz`, and `dash` — which are imported
with a bare `use colx` / `use term`. A `use str` still works — useful
when you want the import list of a file to be explicit — but is never
required for the native modules.

**One environment note:** `fs`, `os`, `http`, and `db` need an operating
system and are absent from the browser playground build, where calling
them reports exactly that; `dom` is the reverse — browser-only, an error
everywhere else.

### What a function returns, and when it raises

Three rules, settled in 0.64. They hold across every module, so you can
predict a function's shape from what it does rather than looking it up.

**1. An operation that cannot fail returns its value.** `os.args()`,
`math.sqrt(x)`, `str.trim(s)`, `fs.exists(p)`, `crypto.sha256(s)` — no
wrapper, nothing to unwrap:

```olang
let args = os.args()
println(to_string(len(args)) + " " + os.arch())
```

**2. An operation that can fail for reasons you could handle returns
`Result`.** A missing file, an absent environment variable, malformed
input, a network that is down. Unwrap it, pattern match it, or propagate
with `?`:

```olang
let n = unwrap(str.parse_int("42"))          // trust it
let m = unwrap_or(str.parse_int("nope"), 0)  // with a default
println(to_string(n + m))
```

**3. An operation *called wrongly* raises.** A wrong argument count, a
wrong type, an index outside the collection — these are bugs in the
calling program, and no caller can sensibly recover from its own bug.
Returning `Err` for them would invite exactly that:

```text
os.arch(1, 2)
// error: os.arch expects 0 arguments, got 2
```

The third rule is why the second one is trustworthy. If misuse also came
back as `Err`, then `unwrap_or(fs.read_file(path), "")` would swallow a
typo'd call exactly the way it swallows a missing file, and the default
would hide the bug indefinitely. Keeping them apart means a `Result` you
see is always a real condition worth handling.

### Absence is Unit

A fourth rule, settled in 0.69, covers *lookups* — operations where "not
there" is an ordinary answer rather than a failure: **a lookup returns
the value, or `Unit` when there is nothing there.** `map_get` on a
missing key, `str.index_of` on an absent substring, a JSON `null`
crossing in — all produce the same `()`, tested with `== ()` / `!= ()`.
(Before 0.69, `str.index_of` returned `-1` — a sentinel that negative
indexing made genuinely dangerous, since `s[str.index_of(s, x)]` on a
miss silently read the last character.)

The three shapes never mix: a lookup says `Unit`, a parse or I/O
operation says `Result`, a misused call raises. And because Unit *is*
absence, a stored Unit and a missing key are deliberately the same
answer — when the distinction matters, ask `map_has_key`. For the common
lookup-with-fallback, `map_get_or` is the one-call form:

```olang
let m = #{ "a": 1 }
println(to_string(map_get_or(m, "a", 0)))   // 1
println(to_string(map_get_or(m, "z", 0)))   // 0 — absent, so the default
```

## Global builtins

### Output

| Function | Description |
|---|---|
| `print(v)` | write without newline |
| `println(...)` | write with newline; several arguments print space-separated |

### Introspection and conversion

| Function | Description |
|---|---|
| `typeof(v)` | runtime type name as a string |
| `show(v)` | display rendering: strings bare, others as `to_string` |
| `to_string(v)` | repr rendering: strings quoted |
| `to_int(v)` | string/float → Int (floats truncate) |
| `to_float(v)` | string/int → Float |
| `len(v)` | length of a list, string, tuple, or map |
| `implements(v, "Trait")` | does the value's type implement the trait? |

`to_string` and `show` render the same text: a string is returned as
itself (`to_string("hi")` is `hi`, never `"hi"`), and every other value
takes its printed form — the form `println` and template interpolation
use. Strings *inside* a container keep their quotes in that form, so
`to_string(["hi"])` is `["hi"]`:

```olang
println(to_string("hi") + " " + show("hi") + " " + to_string(["hi"]))
println(typeof(3.5) + " " + show(to_int(3.9)) + " " + show(to_float("2.5")))
println(show(len([1, 2, 3])) + " " + show(len("abcd")))
```

### Lists

| Function | Description |
|---|---|
| `range(a, b)` | list of integers `a` up to (excluding) `b` |
| `head(xs)` / `tail(xs)` | first element / all but the first |
| `take(xs, n)` / `skip(xs, n)` | first `n` / all but the first `n` (`drop` is `skip` under the name most languages pair with `take`) |
| `reverse(xs)` / `sort(xs)` | reversed / ascending copy |
| `contains(xs, v)` | membership test |
| `concat(a, b)` | join two lists (or use `a + b`) |
| `cons(v, xs)` | prepend an element |
| `chunk(xs, n)` | split into sublists of size `n` |
| `flatten(xss)` | flatten one level of nesting |
| `enumerate(xs)` | pair each element with its index: `[i, v]` |
| `zip(a, b)` | pair elements: list of tuples, shortest wins |
| `group_by(xs, f)` | list of `(key, [members])` tuples |

```olang
println(to_string(range(1, 5)))
println(`${cons(0, [1, 2])} ${flatten([[1], [2, 3]])}`)
println(to_string(chunk([1, 2, 3, 4, 5], 2)))
println(to_string(zip([1, 2], ["a", "b"])))
println(to_string(group_by([1, 2, 3, 4], (x) => x % 2)))
```

Every list helper here is also reachable under `col.` (`col.take`,
`col.drop`, `col.contains`, `col.map`, `col.fold`, …), so there is one
rule to remember rather than two namespaces to guess between: the
global spelling is the short form, `col.` always works. Two helpers
exist only there — `col.index_of(xs, v)` (position, or Unit) and
`col.slice(xs, from, to)` (a half-open, clamped sub-list, the shape of
`str.substring`):

```olang
let xs = [10, 20, 30, 40]
println(`${col.index_of(xs, 30)} ${col.slice(xs, 1, 3)} ${drop(xs, 3)}`)
```

### Aggregates

| Function | Description |
|---|---|
| `sum(xs)` / `min(xs)` / `max(xs)` | numeric aggregates over a list — `min` and `max` also take two or more scalars (`max(0, n - 1)`, the clamp idiom) |
| `average(xs)` | mean of a numeric list |
| `clamp(v, lo, hi)` | bound a number to a range |

```olang
let xs = [4, 1, 7]
println(`${sum(xs)} ${min(xs)} ${max(xs)}`)
println(to_string(clamp(15, 0, 10)))
```

### Higher-order functions

| Function | Description |
|---|---|
| `map(xs, f)` | transform each element |
| `filter(xs, pred)` | keep matching elements |
| `par_map(xs, f)` | `map` fanned out across OS threads |
| `par_filter(xs, pred)` | `filter` fanned out across OS threads |
| `fold(xs, init, f)` | reduce left with an accumulator |
| `reduce(xs, init, f)` | same shape as `fold` |
| `find(xs, pred)` | `Ok(first match)` or `Err` |
| `map_filtered(xs, pred, f)` | fused filter-then-map in one pass |

All take the list first, so they chain with `|>`:

```olang
let total = [1, 2, 3, 4, 5]
    |> filter((x) => x % 2 == 1)
    |> map((x) => x * x)
    |> fold(0, (a, x) => a + x)
println(to_string(total))                       // 1 + 9 + 25
println(to_string(unwrap(find([3, 8, 2], (x) => x > 5))))
```

`par_map` and `par_filter` are the parallel twins of `map` and `filter`:
same arguments, same results in the same order, but the work fans out
across OS threads — one interpreter (with its own bytecode tier) per
worker, no GIL. Use them when `f` does real computation per element;
`examples/concurrency/parmap/` measures the speedup. One deliberate difference: like
`spawn`, the function runs against worker snapshots, so mutating enclosing
state from inside it is not visible to the caller. If several elements
would fail, the error reported is the one `map` would have hit first.
For per-element *effects* rather than values, the language has a
loop-construct twin:
[`par for`](language.md#par-for--parallel-iteration) — the same
fan-out and snapshot semantics, with an implicit barrier. Assigning to an
enclosing binding from inside either one is refused before the program
runs; values come back as results, or over a channel.

```olang
fn weight(n) = {
    let mut acc = 0
    for i in 0..200 { acc = acc + (n * i) % 13 }
    acc
}
let par = 1..50 |> par_map(weight)
println(to_string(par == (1..50 |> map(weight))))   // identical results
println(to_string(par_filter(1..10, (x) => x % 3 == 0)))
```

### Strings (global shortcuts)

| Function | Description |
|---|---|
| `split(s, sep)` | string → list |
| `join(xs, sep)` | list → string (string elements render bare) |
| `starts_with(s, p)` / `ends_with(s, p)` | prefix/suffix test |

```olang
println(join(split("a,b,c", ","), " | "))
println(to_string(starts_with("olang", "o")))
```

The full string toolkit lives in [`str`](#str--strings).

### The `map_*` family

Read and update maps — and **any struct-like value** (anonymous objects,
structs, parsed JSON objects) works the same way, reading *and* writing:
updating a parsed JSON object yields a new JSON object, updating a struct a
new struct of the same type. Everything is immutable — writers return a new
value.

| Function | Description |
|---|---|
| `map_get(m, k)` | value, or `Unit` when absent |
| `map_get_or(m, k, default)` | value, or `default` when absent (or stored Unit) |
| `map_path(m, [k1, k2, …])` | a nested read in one call — the value at the end of the key path, or Unit at the first missing hop; every hop reads like `map_get` |
| `map_has_key(m, k)` | presence test (distinguishes absent from null) |
| `map_keys(m)` / `map_values(m)` | key/value lists (unordered) |
| `entries(m)` | `(key, value)` tuples, sorted by key — for `for (k, v) in` |
| `map_len(m)` | entry count |
| `map_set(m, k, v)` | new value of the same kind with `k` set |
| `map_remove(m, k)` | new value of the same kind without `k` |
| `map_merge(a, b)` | new map, `b`'s entries winning |
| `map_clear(m)` | fresh empty map |

```olang
let m = #{ "a": 1, "b": 2 }
let m2 = map_set(m, "c", 3)
println(show(map_len(m)) + " -> " + show(map_len(m2)))
println(show(map_get({ x: 42 }, "x")))            // objects read like maps
let d = map_set(unwrap(json.parse("{\"a\": 1}")), "b", 2)
println(typeof(d) + " " + show(map_get(d, "b")))  // JSON updates stay JSON
for (k, v) in entries(m2) { print(k + "=" + show(v) + " ") }
println("")
```

### Results

| Function | Description |
|---|---|
| `unwrap(r)` | value of `Ok`, aborts on `Err` |
| `unwrap_or(r, default)` | value or default |
| `unwrap_or_else(r, f)` | value or `f(error)` |
| `is_ok(r)` / `is_err(r)` | tests |
| `result_map(r, f)` | transform the `Ok` value |
| `result_map_err(r, f)` | transform the `Err` value |

```olang
let r = Ok(10)
println(to_string(result_map(r, (v) => v * 2)))
println(to_string(unwrap_or(Err("nope"), -1)))
```

### Evaluation helpers

| Function | Description |
|---|---|
| `lazy(v)` | identity — olang evaluates eagerly (see note) |
| `force(v)` | identity — the counterpart of `lazy` |
| `set_parallel(b)` | enable or disable the parallel machinery — explicit (`par_map`) and automatic alike |

**olang is eager.** A builtin receives its argument already evaluated, so
`lazy` cannot defer anything — both functions are the identity, kept for
source compatibility with code written against them.

```olang
let deferred = lazy([1, 2, 3] |> map((x) => x * 10))
println(to_string(force(deferred)))
```

## `str` — strings

The complete string toolkit. The module's one convention does a lot of
work: pure transformations (`trim`, `replace`, `to_upper`, ...) return
their value directly and never fail — out-of-range positions clamp,
absent substrings report `-1` — while the two genuine *parsers*
(`parse_int`, `parse_float`) return `Result`, because "this text is not
a number" is the caller's decision to make. Strings are immutable, so
every function returns a new string.

| Function | Description |
|---|---|
| `str.length(s)` | character count |
| `str.char_at(s, i)` | 1-char string, `""` out of bounds |
| `str.chars(s)` | list of characters (codepoints) |
| `str.graphemes(s)` | list of visible characters (UAX #29 clusters) |
| `str.substring(s, from, to)` | half-open slice, clamped |
| `str.index_of(s, sub)` / `str.last_index_of` | position, or `()` when absent |
| `str.contains(s, sub)` / `str.count(s, sub)` | search |
| `str.starts_with` / `str.ends_with` | affix tests |
| `str.split(s, sep)` / `str.join(xs, sep)` | list conversion |
| `str.lines(s)` / `str.words(s)` | split on newlines / whitespace |
| `str.trim` / `str.trim_start` / `str.trim_end` | strip whitespace |
| `str.pad_start(s, n, fill)` / `str.pad_end` | pad to width |
| `str.repeat(s, n)` | repetition |
| `str.replace(s, from, to)` / `str.replace_first` | substitution |
| `str.to_upper` / `str.to_lower` / `str.capitalize` | case |
| `str.reverse(s)` | reversed by visible character — clusters stay whole |
| `str.is_empty(s)` | `""` test |
| `str.parse_int(s)` / `str.parse_float(s)` | `Result` parses |
| `str.fmt(template, ...)` | fill `{}` placeholders, display form; `{{`/`}}` escape |
| `str.fixed(x, digits)` | exactly `digits` decimals, never exponent form, never `-0.00` — the column form `to_string` is not |
| `str.thousands(x, digits)` | `str.fixed` with thousands separators: `1,234,567.89` |

```olang
println(str.pad_start("7", 3, "0"))                 // 007
println(str.substring("olang", 1, 4))               // lan
println(str.replace("a-b-c", "-", "+"))
println(to_string(str.chars("ok")))
println(to_string(unwrap(str.parse_float("2.5")) * 2))
println(str.fmt("{} scored {} ({}%)", "ada", 99, 97.5))
```

## `col` / `colx` — collections

Higher-order operations beyond the global builtins. The design principle:
the *global* builtins cover what nearly every program touches (`map`,
`filter`, `fold`, `sort`); `col` holds the next ring out — quantifiers,
partitions, key-function variants — so the global namespace stays small
while the full toolkit stays one dot away. `colx` is the same API
implemented *in olang* and embedded in the binary (`use colx`) — the two
are differential-tested against each other, so the language is exercised
by its own standard library.

| Function | Description |
|---|---|
| `col.all(xs, p)` / `col.any(xs, p)` | quantifiers (short-circuit) |
| `col.index_of(xs, v)` / `col.slice(xs, from, to)` | position of a value (or Unit) / a half-open, clamped sub-list |
| `col.take`, `col.drop`, `col.map`, … | every global list helper, mirrored here — one namespace that always works |
| `col.unique(xs)` | dedupe, first-seen order |
| `col.partition(xs, p)` | `([matching], [rest])` |
| `col.sum_by(xs, f)` | total of a projection |
| `col.count_by(xs, f)` / `col.frequencies(xs)` | maps of counts |
| `col.min_by(xs, f)` / `col.max_by(xs, f)` | extremes by key function |
| `col.sort_by(xs, f)` | ascending by key function |
| `col.take_while(xs, p)` / `col.drop_while(xs, p)` | prefix operations |
| `col.flat_map(xs, f)` | map then flatten one level |
| `col.window(xs, n)` | sliding windows |
| `col.zip_with(a, b, f)` | combine pairwise |
| `col.last(xs)` | final element |

```olang
println(to_string(col.unique([3, 1, 3, 2, 1])))
println(to_string(col.partition([1, 2, 3, 4], (x) => x % 2 == 0)))
println(to_string(col.sum_by([{ v: 2 }, { v: 5 }], (r) => r.v)))
println(to_string(map_get(col.frequencies(["a", "b", "a"]), "a")))
println(to_string(col.window([1, 2, 3, 4], 2)))
println(to_string(col.zip_with([1, 2], [10, 20], (a, b) => a + b)))
```

The quantifiers short-circuit, and the `_by` family takes a key function —
so sorting or picking extremes by a projection needs no comparator:

```olang
println(to_string(col.all([2, 4, 6], (x) => x % 2 == 0)))
println(to_string(col.any([1, 3, 5], (x) => x > 4)))
println(to_string(col.sort_by(["ccc", "a", "bb"], (s) => str.length(s))))
let top = col.max_by([{ name: "a", p: 3 }, { name: "b", p: 9 }], (r) => r.p)
println(top.name)
println(to_string(col.take_while([1, 2, 9, 1], (x) => x < 5)))
println(to_string(col.flat_map([1, 2], (x) => [x, x * 10])))
println(to_string(col.last([1, 2, 3])))
```

## `math` / `mathx` — mathematics

Pure numeric functions — every one takes numbers to numbers with no
`Result` wrapping, which lets them ride the fastest execution paths (the
float functions compile all the way to native code in hot loops; see
[the OVM chapter](ovm.md#the-jit)).

| Group | Functions |
|---|---|
| Basics | `abs` `sign` `min` `max` `pow` `sqrt` `cbrt` |
| Rounding | `floor` `ceil` `round` `trunc` `fract` |
| Integers | `gcd` `lcm` `factorial` |
| Exp/log | `exp` `exp2` `ln` `log` `log2` `log10` |
| Trig | `sin` `cos` `tan` `asin` `acos` `atan` `atan2` |
| Hyperbolic | `sinh` `cosh` `tanh` |
| Angles | `radians` `degrees` |
| Constants | `math.PI` `math.E` `math.TAU` `math.SQRT_2` `math.SQRT_3` `math.LN_2` `math.LN_10` `math.LOG2_E` `math.LOG10_E` |

```olang
println(`${math.pow(2, 10)} ${math.sqrt(2.25)}`)
println(`${math.gcd(12, 18)} ${math.factorial(5)}`)
println(`${math.round(2.6)} ${math.fract(2.75)}`)
println(to_string((math.PI > 3.14159) && (math.TAU > 6.28)))
```

`mathx` (`use mathx`) is an olang-source module covering the integer and
rounding core (`abs` through `sqrt`, plus `PI`) — smaller in scope than
`math`, and differential-tested against it.

## `vec` — vectors over lists

The kernels numeric code otherwise spells as index loops, over lists of
Int or Float: `vec.dot(a, b)`, `vec.add(a, b)`, `vec.sub(a, b)`,
`vec.scale(a, k)`, `vec.sum(a)`, `vec.norm(a)`, `vec.mean(a)`. Results
are Float (or Float lists); lengths must agree; a non-finite result
raises like any float overflow. `ods.to_matrix(frame, names)` hands a
frame's columns to them as a list of Float lists.

```olang
let x = [1.0, 2.0, 3.0]
let g = [0.5, 0.5, 0.5]
println(to_string(vec.dot(x, g)))                     // 3.0
println(to_string(vec.add(x, vec.scale(g, -2.0))))    // [0.0, 1.0, 2.0]
println(to_string(vec.norm([3.0, 4.0])))              // 5.0
```

## `bigint` — arbitrary precision

Integers without a ceiling. `bigint.of` takes an Int or a string of
decimal digits; the value then uses the ordinary operators — `+ - * / %`
and the comparisons — and a plain Int operand promotes to BigInt on
contact. The arithmetic follows the Int rules exactly (division
truncates toward zero, the remainder takes the dividend's sign, the same
zero-divisor errors), so promoting a computation changes its range and
nothing else. When an Int computation does overflow, the error itself
points here.

| Function | Does |
|---|---|
| `of(x)` | Int or digit-string → BigInt; raises on a malformed string |
| `parse(s)` | the `Result`-returning twin of `of`, for data you don't control |
| `to_int(b)` | back to a 64-bit Int, as a `Result` — the value may not fit |
| `to_float(b)` | to a Float, rounding to 53 bits — the only door to Float |
| `abs(b)` `neg(b)` | absolute value; negation (unary `-` does not apply to BigInt) |
| `pow(b, n)` | `b` to a non-negative Int power |
| `mod_pow(b, e, m)` | `bᵉ mod m` without materializing `bᵉ` |
| `gcd(a, b)` | greatest common divisor, always non-negative |

```olang
let f = fold(range(1, 51), bigint.of(1), (acc, i) => acc * i)
println(f)                        // 50! — 65 digits
println(bigint.of(2) + 1)         // 3: Int promotes on contact
println(bigint.mod_pow(bigint.of(7), bigint.of(560), bigint.of(561)))
```

Floats never mix implicitly: a Float has 53 bits of mantissa, so
`big * 0.5` would silently round the very digits BigInt exists to keep.
The operator refuses with a pointer to `bigint.to_float`, the one
sanctioned, explicitly lossy conversion.

## Collections, in olang

One module — `collections` — of six submodules, written entirely in
olang, compiled into the binary, and **automatically available**: the
name loads on first touch, so `collections.heap.push(...)` works in a
bare script with no `use`, one name is all the library claims on the
global surface, and a program that never reaches for it never pays for
it. (A binding of your own named `collections` always wins.)

| Submodule | Structure | For |
|---|---|---|
| `collections.heap` | binary min-heap of (priority, item) pairs | Dijkstra, schedulers, top-k, event queues |
| `collections.deque` | double-ended queue over a ring buffer | BFS frontiers, sliding windows, work lists |
| `collections.table` | flat hash table, open addressing | counting, indexing, hot single-table loops |
| `collections.dsu` | disjoint sets (union–find), by rank + compression | connectivity, Kruskal, clustering |
| `collections.bitset` | dense integer set, 63 members per word | sieves, visited-sets, set algebra |
| `collections.alg` | sort / bisect / select / graphs / Dijkstra | the classic algorithms over the above |

Code that leans on a few submodules imports their short names — the
idiomatic form:

```olang
use collections { heap, table }
```

**One calling convention.** Operations that write take the handle first
and return the new handle, and you rebind the same name:

```olang
use collections { heap }
let mut h = heap.new()
h = heap.push(h, 3, "job-a")
h = heap.push(h, 1, "job-b")
println(heap.top_item(h))     // job-b
h = heap.pop(h)
```

The rebind is load-bearing: it lets the runtime pass the handle *by
move* and write in place, so pushes, puts, and unions are O(1)–O(log n)
memory operations, not copies. Reads (`heap.size`, `table.get`,
`dsu.connected`, ...) take the handle without rebinding. Holding an
older handle is always legal and always safe — it is a snapshot, and
the next write through either handle pays a one-time copy; the
language's aliasing guarantee is never suspended, only the speed
changes.

**The representation discipline.** Every handle is one flat list with a
small metadata prefix — a heap is `[size, p0, i0, p1, i1, ...]`, a
union-find is `[n, parents..., ranks...]`, a graph is CSR offsets and
targets — so operations are index arithmetic over contiguous memory.
Each module's source documents its own layout; they read as worked
examples of writing fast olang.

`alg` prefers **keys to comparators**: `alg.sort_by_key(xs, key_fn)`
calls `key_fn` once per element and sorts on the extracted keys with
native comparisons. `alg.dijkstra` is the composed showcase — `heap`
over a flat weighted graph, entirely in olang.

The primitives underneath — `col.set`, `col.swap`, `col.filled`, and
the by-move calling convention — are ordinary language surface: your
own data-structure libraries get the same speed by following the same
rebind discipline.

**Measured** (release build, M5 Pro, 200k operations): `table` runs a
put-and-probe workload ~46× faster than driving the builtin persistent
map through the same loop (5.8s against 269s — flat probing against
per-write structural copies); `dsu` unions cost ~6µs each; `heap`
push-and-pop ~36µs a pair. Two honest boundaries: for *plain* sorting
of a list, the native `sort` builtin remains hundreds of times faster
than `alg.sort` — reach for `alg` when you need stability, keys, the
bisect family, or selection — and structure operations run on the
interpreter by design (the tier boundary would convert a whole handle
per call), so they are memory-fused microsecond operations, not
nanosecond ones. Where the builtins already cover a shape natively,
they stay the fast path; these modules cover the shapes the builtins
do not.

## `json` — JSON

`json.parse` turns text into olang values: objects become dot-accessible
(and `map_*`-readable) `JsonObject`s, arrays become lists, `null` becomes
`Unit`. `json.stringify` goes the other way for any olang value.

| Function | Description |
|---|---|
| `json.parse(s)` | `Result` of the parsed value |
| `json.stringify(v)` | `Result` of compact JSON text |
| `json.prettify(s)` / `json.minify(s)` | reformat JSON text |
| `json.validate(s)` | is the text valid JSON? |
| `json.get(s, path)` / `json.set(s, path, v)` | path access on raw text |
| `json.has_key(s, k)` / `json.remove(s, k)` | text-level operations |
| `json.get_keys` `json.get_values` `json.get_type` | text-level inspection |
| `json.array_get` `json.array_length` `json.array_push` | text-level arrays |
| `json.merge(a, b)` / `json.deep_clone(s)` | combine / copy |

```olang
let doc = unwrap(json.parse("{\"name\": \"ada\", \"tags\": [\"x\", \"y\"], \"age\": 36}"))
println(`${doc.name} has ${len(doc.tags)} tags`)
println(to_string(map_get(doc, "age")))          // dynamic key access
let out = unwrap(json.stringify({ ok: true, n: 1 }))
println(out)
```

## `toml` — TOML

The config format olang's own `olang.toml` manifests use, with `json`'s
core surface: `parse`, `stringify`, `validate`. Both modules bridge
values through the same conversions, so a table parses to the same Map
shapes JSON objects do — tables become Maps, arrays become Lists,
datetimes become their string rendering. A TOML document is a table at
the top level, so `stringify` accepts a Map (or struct-like value) and
returns `Err` for anything else.

```olang
let text = "name = \"cfg\"\n[server]\nport = 7317"
let doc = unwrap(toml.parse(text))
println(show(map_get(doc, "name")) + " on " + show(map_get(map_get(doc, "server"), "port")))

let out = unwrap(toml.stringify(#{ "retries": 3 }))
println(str.trim(out))
println(to_string(toml.validate("not [ valid")))
```

## `csv` — CSV

Quoted fields, embedded commas, and header-aware parsing.
`parse_with_headers` yields rows whose cells are dot-accessible by column
name (all cells are strings — convert explicitly).

| Function | Description |
|---|---|
| `csv.parse(s)` | rows as lists of strings |
| `csv.parse_with_headers(s)` | rows as named records |
| `csv.stringify(rows)` / `csv.stringify_with_headers` | write CSV text |
| `csv.get_headers(s)` / `csv.set_headers` | header row |
| `csv.row_count(rows)` / `csv.column_count(rows)` | dimensions |
| `csv.read_row` `csv.read_column` `csv.read_cell` `csv.set_cell` | access |
| `csv.add_row` `csv.add_column` | build tables |
| `csv.filter_rows` `csv.sort_by_column` | table operations |
| `csv.to_json(rows, with_headers)` | parsed rows → JSON text (`with_headers`: treat row 0 as column names) |
| `csv.from_json(json, headers)` | JSON text + header list → CSV rows (headers first) |

```olang
let raw = "name,score\nada,99\nbob,82"
let rows = unwrap(csv.parse_with_headers(raw))
let total = rows |> fold(0, (acc, r) => acc + unwrap(str.parse_int(r.score)))
println(`${rows[0].name}, total ${total}`)
```

## `re` — regular expressions

Rust regex syntax (no backtracking, so patterns run in linear time and a
hostile input cannot hang a program). Matching functions return `Result`
because the *pattern* can be malformed — the usual shape is one
`unwrap` around a pattern you wrote yourself, or `re.is_valid` first
for a pattern that arrives at runtime.

| Function | Description |
|---|---|
| `re.is_match(pat, s)` | boolean test |
| `re.find(pat, s)` | first match text |
| `re.find_all(pat, s)` | all match texts |
| `re.captures(pat, s)` | full match + capture groups |
| `re.replace(pat, s, rep)` / `re.replace_all` | substitution |
| `re.split(pat, s)` | split by pattern |
| `re.is_valid(pat)` | is the pattern itself well-formed? |

```olang
println(to_string(unwrap(re.find_all("\\d+", "a1 b22 c333"))))
println(to_string(unwrap(re.captures("(\\w+)@(\\w+)", "ada@lovelace"))))
println(unwrap(re.replace_all("\\s+", "too   many spaces", " ")))
```

## `dates` — dates and times

ISO-8601 strings in, ISO-8601 strings out; fallible operations return
`Result`.

| Group | Functions |
|---|---|
| Now | `now` `utc_now` `today` · `stamp` — the storage form: UTC, second precision, `Z` suffix (`2026-08-31T23:40:06Z`), sortable as text · `stamp_ms` — the same at millisecond precision (`…06.503Z`), the one to key a row's version on: two edits inside one second are identical under `stamp` |
| Build | `date(y, m, d)` → `Date` · `datetime(y, m, d, h, mi, s)` `time(h, mi, s)` |
| Parse/format | `parse(s)` → `Date` · `parse_date` `parse_datetime` `parse_time` `format_date` `format_datetime` `format_time` |
| Fields | `year` `month` `day` `hour` `minute` `second` `weekday` |
| Arithmetic | `add_days` `add_weeks` `add_months` `add_years` `diff_days` — or Date operators |
| Epoch | `timestamp(dt)` `from_timestamp(n)` |
| Facts | `is_leap_year(y)` `days_in_month(y, m)` |

Since 0.69 a date is a first-class **`Date` value**: `dates.date(y, m, d)`
and `dates.parse(s)` build one, `typeof` says `Date`, it displays as ISO
(`2026-08-07`), comparisons order chronologically, `d2 - d1` is the day
difference, and `d + n` / `d - n` shift by days. Every date-taking
function accepts a `Date` *or* a date string and answers in kind — string
in, string out (the pre-0.69 behavior, unchanged), `Date` in, `Date`
out — so existing string-based code keeps working while new code gets a
real type:

```olang
let d = unwrap(dates.date(2026, 8, 7))
let later = d + 30                        // Date + days is a Date
println(`${d} + 30d = ${later}`)
println(`ordered: ${d < later}`)
println(`days apart: ${later - d}`)       // Date - Date is days
println(`leap 2028: ${dates.is_leap_year(2028)}`)
println(`weekday: ${unwrap(dates.weekday(d))}`)

let s = unwrap(dates.add_days("2026-08-07", 30))   // strings still work
println(s)
```

`unwrap(dates.timestamp(dates.now()))` is the idiom for "seconds since
the epoch, now" (`timestamp` parses its argument, so it returns a
`Result`; for a plain millisecond clock, `time.now_ms()` is simpler).

## `time` — clocks and sleeping

`dates` is calendars; `time` is measurement and pacing.

| Function | Description |
|---|---|
| `time.now_ms()` | milliseconds since the Unix epoch |
| `time.monotonic_ms()` | monotonic milliseconds (never goes backwards) — the clock for durations |
| `time.sleep(ms)` | block for `ms` milliseconds |

```olang
let t0 = time.monotonic_ms()
time.sleep(25)
println(show(time.monotonic_ms() - t0 >= 20))   // true
```

## `task` — background threads

`spawn expr` runs the expression on its own OS thread and returns a
**task handle** immediately; this module is how you collect the result.
The model is covered in
[the language reference](language.md#spawn-and-taskjoin).

| Function | Returns |
|---|---|
| `task.join(t)` | the task's value, or `Err(e)` if it failed — blocks until it finishes |
| `task.join_timeout(t, ms)` | `Ok(v)` if it finished within `ms`, else `Err("timed out")` |
| `task.list()` | every task a live handle still watches: `#{ id, state, elapsed_ms }` |
| `task.parked()` | threads blocked on unbounded waits: `#{ thread, on, waited_ms }` |

Neither wraps success in `Result` unnecessarily: `task.join` hands back
the value itself, and reports failure as `Err(e)` so one bad worker is
a value to handle rather than a crash. `task.join_timeout` *does* wrap,
because it must distinguish "the task produced `Err`" from "we stopped
waiting" — two very different things.

`task.list` reports each watched task's state — `"running"`,
`"joining"` (a thread is mid-collect), or `"done"` (finished, its
memoized result still collectible). `task.parked` is the live view
behind [the stall detector](#the-stall-detector)'s report: every thread
currently blocked in an unbounded wait, and what it is waiting on.

```olang
fn work(n) = { time.sleep(10); n * n }

let jobs = [spawn work(2), spawn work(3), spawn work(4)]
println(to_string(jobs |> map(task.join)))   // [4, 9, 16]
```

**A timeout bounds the wait, not the work.** An OS thread cannot be
cancelled from outside without leaving whatever it touched in an
unknown state, so a timed-out task keeps running to completion. Its
result is memoized, so a later `task.join` on the same handle still
collects it. For work that genuinely stops early, give the task
something to check — a channel, or a value it re-reads each pass.

Joining the same handle twice returns the memoized result rather than
re-running anything. A handle dropped without ever being joined is
fire-and-forget: the thread detaches and finishes on its own.

## `cell` — mutable locations

A cell holds a value that can be replaced in place. It is the language's
only mutable location, and exists for the case that returning the new
value handles badly: state updated from deep in a call chain, or from a
callback whose signature you do not control. The
[language reference](language.md#cells-the-one-mutable-location) covers
the model; this is the API.

| Function | Returns |
|---|---|
| `cell(v)` / `cell.new(v)` | a new cell holding `v` (the module is callable, so both spellings are the same function) |
| `cell.get(c)` | the current value |
| `cell.set(c, v)` | `()` — replaces the contents |
| `cell.update(c, f)` | the new value, after applying `f` to the current one and storing the result |

None of these return `Result`: there is no expected failure to report.
They raise on misuse — reading a cell from a thread that does not own
it, or touching a cell from inside its own `cell.update`.

```olang
let seen = cell(#{})
fn remember(word) = {
    let counts = cell.get(seen)
    let n = if map_has_key(counts, word) => map_get(counts, word) else => 0
    cell.set(seen, map_set(counts, word, n + 1))
    n + 1
}
for w in ["ok", "err", "ok", "ok"] { remember(w) }
println(to_string(map_get(cell.get(seen), "ok")))   // 3
```

**Confinement.** A cell belongs to the thread that created it. Reading
or writing it from another thread raises, and `chan.send` refuses to
send one (including one nested inside a list, map, struct, or `Result`)
— the language's no-shared-mutable-state guarantee is what makes
`spawn` and `par_map` lock-free, and cells do not weaken it. Send the
*contents* instead:

```olang
let ch = chan.new()
let c = cell(11)
chan.send(ch, cell.get(c))
println(show(chan.recv(ch)))   // Ok(11)
```

A cell created inside a task and used only there is unremarkable; the
rule concerns crossing, not tasks.

## `chan` — channels

Message passing between `spawn`ed tasks: multi-producer multi-consumer
queues of olang values. `chan.new()` makes an unbounded channel;
`chan.bounded(n)` holds at most `n` in-flight messages, and senders
block when it is full (`chan.bounded(0)` is a rendezvous — a send waits
for its receiver). Handles are plain values, so they cross the `spawn`
boundary like anything else.

| Function | Returns |
|---|---|
| `chan.new()` | a channel |
| `chan.bounded(n)` | a channel holding at most `n` messages |
| `chan.send(c, v)` | `Ok(())`, or `Err` when the channel is closed |
| `chan.recv(c)` | blocks; `Ok(value)`, or `Err` when closed and drained |
| `chan.try_recv(c)` | `Ok(value)`, `Err("channel is empty")`, or `Err("channel is closed")` |
| `chan.recv_timeout(c, ms)` | like `recv`, plus `Err("timed out")` |
| `chan.stat(c)` | a snapshot map: `id`, `queued`, `closed`, `recv_waiting`, `send_waiting` |
| `chan.close(c)` | closes the sending side (idempotent) |

Closing is cooperative and drains: after `chan.close(c)` new sends
fail, but messages already queued are still received before `recv`
starts reporting the close — so a consumer loop can simply `match` on
`recv` and stop on `Err`.

### The stall detector

A `chan.recv` that no live thread can ever satisfy does not hang the
program. The runtime keeps a census of every thread running olang code
and a registry of every unbounded wait — blocking receives, sends
against a full bounded channel, and `task.join`. When every counted
thread has sat blocked in such a wait for two consecutive quarter-second
samples, no internal wake is possible and none can arrive from outside,
so the runtime prints each blocked site and aborts with exit code 101:

```text
deadlock: every live thread is blocked on an unbounded wait — nothing can ever send
  main: task.join on task 1 (waiting 0.5s)
  olang-spawn-1: chan.recv on channel #1 (waiting 0.5s)
```

A program that can still make progress is never interrupted: a thread
that is computing, sleeping, or waiting with a bound (`recv_timeout`,
`join_timeout`, `try_recv`) counts as live but not parked, and a
running `http.serve` disables the abort entirely, because an incoming
request can wake a worker at any time. Setting `OLANG_STALL_ABORT=0`
restores the old behavior (the program hangs) for embedding hosts that
manage their own threads.

```olang
fn producer(ch, n) = {
    let mut i = 1
    while i <= n {
        chan.send(ch, i * i)
        i = i + 1
    }
    chan.close(ch)
    n
}

let pipe = chan.new()
let worker = spawn producer(pipe, 4)
let mut total = 0
let mut going = true
while going {
    match chan.recv(pipe) {
        Ok(v) => { total = total + v }
        Err(e) => { going = false }
    }
}
println(`${total} from ${task.join(worker)} squares`)
```

## `random` — randomness

One global, seedable generator. `random.seed(n)` makes every subsequent
draw deterministic — which turns a simulation, a shuffled test fixture,
or a [`stats` sample](#stats--statistical-inference) into something you
can reproduce exactly, so randomized code stays debuggable and
testable.

| Function | Description |
|---|---|
| `random.seed(n)` | make subsequent draws deterministic |
| `random.random()` | float in `[0, 1)` |
| `random.uniform(a, b)` / `random.gauss(mu, sigma)` | distributions |
| `random.randint(a, b)` | integer in `[a, b]` |
| `random.randbool()` | coin flip |
| `random.choice(xs)` / `random.choices(xs, k)` / `random.sample(xs, k)` | pick from a list |
| `random.shuffle(xs)` | new shuffled list |
| `random.randstr(n)` (+ `_alpha` `_alnum` `_numeric`) | random strings |

```olang
random.seed(7)
let first = random.randint(1, 100)
random.seed(7)
println(to_string(first == random.randint(1, 100)))   // deterministic

let deck = random.shuffle(range(1, 53))
println(to_string(len(deck)) + " cards, top: " + typeof(random.choice(deck)))
println(to_string(len(random.sample(deck, 5))))
println(to_string(str.length(random.randstr_alpha(8))))
```

## `crypto` — hashing and encryption

The `crypto` module provides digests, HMACs, password hashing, and AES and
RSA encryption, with string-friendly conventions: digests return lowercase
hexadecimal strings, and key operations return `Result`. `secure_compare`
compares two strings in constant time; use it in place of `==` when comparing
secrets, because `==` can leak information through its timing.

| Group | Functions |
|---|---|
| Digests | `md5` `sha1` `sha256` `sha512` |
| MACs | `hmac_sha256` `hmac_sha512` |
| Passwords | `hash_password` `verify_password` `derive_key` |
| Symmetric | `encrypt_aes` `decrypt_aes` |
| Asymmetric | `generate_key_pair` `encrypt_rsa` `decrypt_rsa` `sign_data` `verify_signature` `export_public_key` `import_public_key` |
| Utilities | `random_bytes` `random_hex` `hex_encode` `hex_decode` `secure_compare` |

```olang
println(str.substring(crypto.sha256("olang"), 0, 16))
println(to_string(crypto.secure_compare("abc", "abc")))
```

## `bytes` — binary data

Strings are UTF-8 text; `Bytes` (new in 0.69) is the value for everything
that isn't — an image, an archive, a hash's raw output, a non-UTF-8
file. A `Bytes` value is immutable, `typeof` says `Bytes`, `len(b)` is
the byte count, `b[i]` is the byte at `i` as an Int (negative counts
from the end), equality is structural, and it displays as a capped hex
preview. `fs.read_bytes`/`fs.write_bytes` move it to and from disk;
`base64` and the `crypto` hashes accept it.

| Function | Description |
|---|---|
| `bytes.from_list(ints)` / `bytes.to_list(b)` | to and from a list of Ints 0..=255 |
| `bytes.from_string(s)` | a string's UTF-8 bytes |
| `bytes.to_string(b)` | decode as UTF-8 — `Result`, since bytes may not be text |
| `bytes.len(b)` | byte count (the global `len` works too) |
| `bytes.slice(b, from, to)` | half-open, clamped — the shape of `str.substring` |
| `bytes.concat(a, b)` | concatenation |

```olang
let b = bytes.from_list([104, 105, 33])
println(`${len(b)} bytes, first ${b[0]}, last ${b[-1]}`)
println(unwrap(bytes.to_string(b)))                    // hi!
println(show(bytes.to_list(bytes.slice(b, 0, 2))))     // [104, 105]
println(base64.encode(b))                              // aGkh
println(show(bytes.from_string("hi!") == b))           // true
```

## `base64` — base64

Binary-safe text encoding in its three practical variants — standard,
URL-safe, and unpadded. Encoding always succeeds and takes a string or
[`Bytes`](#bytes--binary-data); decoding returns `Result`, since
arbitrary text may not be valid base64 — `decode` yields text,
`decode_bytes` yields raw bytes.

| Function | Description |
|---|---|
| `base64.encode(s)` / `base64.decode(s)` | standard alphabet (decode returns `Result`) |
| `base64.decode_bytes(s)` | decode to `Bytes` for non-text payloads |
| `base64.encode_url_safe` / `decode_url_safe` | URL-safe alphabet |
| `base64.encode_no_pad` / `decode_no_pad` | without `=` padding |
| `base64.is_valid(s)` / `base64.validate(s)` | checks |

```olang
let enc = base64.encode("olang")
println(enc + " -> " + unwrap(base64.decode(enc)))
println(show(len(unwrap(base64.decode_bytes(enc)))))   // 5
```

## `fs` — file system

Whole-value I/O: `read_file` returns the entire contents as one string,
`write_file` replaces them — no file handles, no open/close lifecycle to
manage, which fits a language whose programs transform values. Every
function returns `Result`, because the file system is the classic source
of failures that are the *caller's* business (missing file, permissions)
rather than bugs. (Examples are `no-run`: they touch the disk.)

| Group | Functions |
|---|---|
| Files | `read_file` `write_file` `append_file` `copy_file` `move_file` `remove_file` — and `read_bytes` / `write_bytes` for [binary data](#bytes--binary-data) |
| Directories | `create_dir` `create_dir_all` `list_dir` `walk` `glob` `remove_dir` `remove_dir_all` |
| Queries | `exists` `is_file` `is_dir` `file_size` `file_info` |
| Paths | `join(parts)` `dirname` `basename` `ext` `abs_path` — pure string surgery (except `abs_path`, which resolves against the current directory and normalizes `.`/`..` without requiring the file to exist) |

`fs.join(["logs", name + ".txt"])` joins segments with the platform
separator (an absolute segment restarts the path, standard join
semantics); `fs.dirname`/`fs.basename`/`fs.ext` decompose without
touching the disk. `fs.walk(dir)` lists every file below a directory
(recursive, sorted);
`fs.glob(pattern)` filters by a pattern where `*` matches within a path
segment, `?` one character, and `**` any number of segments.

```olang no-run
let text = unwrap(fs.read_file("data.txt"))
unwrap(fs.write_file("out.txt", str.to_upper(text)))
for source in unwrap(fs.glob("src/**/*.ol")) {
    println(source)
}
println(show(len(unwrap(fs.walk("docs")))) + " files under docs/")
```

## `meta` — the program as data (the Open AST)

`meta.parse(source)` parses olang source and hands the program back as
ordinary olang values: a list of statement maps, each tagged with a
`"kind"`, that you walk with the same `map`/`filter`/`fold`/`match` you
use on any data. Because the syntax is stable, these node shapes are a
stable public format — linters, codemods, and import extractors are
olang scripts, not compiler changes.

| Function | Result |
|---|---|
| `meta.parse(source)` | `Ok(list of node maps)` \| `Err(message)` — a syntax error is a normal `Err`, never a crash |
| `meta.eval(source, options?)` | evaluate source; `options` bounds untrusted code — `#{ "max_steps": n }` (loop iterations and calls, deterministic) and `#{ "timeout_ms": ms }` (wall clock) — returning `Err("budget exceeded …")` instead of a hung thread |
| `meta.expand(source)` | `Ok(text)` \| `Err(message)` — the program after macro expansion, as source: every `meta fn` removed and every `@` site replaced by what it generated (what `olang expand FILE` prints) |
| `meta.encode(source)` | `Ok(bytes)` \| `Err(message)` — the parsed program (macros expanded) as a program image: bytes the runtime loads without parsing, behind a header naming the olang version that wrote it, which is the only version that loads it. What the web SDK's `serve` hands the browser in place of the source bundle |

Nodes are discriminated-union maps. Top-level statements carry `line` and
`column`; expressions nest (a `call`'s `callee` and `args` are themselves
node maps). Common kinds: `use` (`path`, `items`), `fn` (`name`, `params`,
`shared`, `body`), `let`, `type`, `test`, `call` (`target` is the dotted
callee like `fs.read_file`, `args`), `field`, `binop`, `if`, `match`,
`for`, `pipeline`, and the literals (`int`, `float`, `str`, `bool`,
`ident`).

```olang
// A dependency lister — a file's imports — in four lines over meta.parse.
let program = unwrap(meta.parse("use geometry { area }\nuse fmt\nfn f() = 1"))
for node in program |> filter((n) => map_get(n, "kind") == "use") {
    println(map_get(node, "path") + " " + show(map_get(node, "items")))
}
// geometry ["area"]
// fmt ["*"]
```

Every sub-expression is emitted — a `match`'s `arms`, a map's `entries`, a
struct's `fields`, a template's `parts`, and the `spawn`/`assert` interiors
are all walkable node maps — so a tool that filters the node tree cannot
silently miss a call hidden in a subtree. What is summarized (not dropped)
is non-expression detail: patterns collapse to their bound names, type
annotations to source text. Enough to *analyze* a program, not to perfectly
reconstruct one. See
[`examples/language/metatool`](../examples/language/metatool/main.ol) for a linter that
counts bare `unwrap()` calls per function.

Three functions serve [the macro system](macros.md) and stand on their
own: `meta.eval(source)` evaluates source text in a child interpreter
and returns `Result`. At ordinary runtime effects are allowed — the
child inherits the run's capability table, so evaluated code is judged
by the same grants, and modules resolve from the current file. Inside a
meta fn the same call runs in the pure expansion sandbox (no
filesystem, network, processes, clock, or randomness), keeping macro
expansion deterministic. It takes source text, not `meta.parse` nodes —
nodes summarize patterns and types and cannot be turned back into a
program. `meta.lit(value)` renders a
value as source text that evaluates back to it, with strings escaped and
map keys sorted so the output is reproducible; and `meta.fresh(prefix)`
yields a name no program writes by hand, for generated temporaries.

```olang
println(to_string(unwrap(meta.eval("2 + 3"))))     // 5
println(meta.lit([1, "a"]))                        // [1, "a"]
```

## `os` — operating system

The process's view of its world: arguments, environment, directories,
stdin, and the ability to run other programs. Together with `fs` it is
what makes olang a scripting language in the practical sense — the
[task CLI](../examples/) and the examples harness are built on little
else.

| Group | Functions |
|---|---|
| Process | `args` `exit(code)` `pid` `exe_path` `exec(program, args)` |
| Terminal | `is_tty()` — is stdout a terminal? (`Bool`); `flush()` — flush buffered stdout, for progress bars |
| Input | `read_line()` — one line from stdin as `Ok(line)`, `Err("eof")` at end; `stdin()` — everything to end-of-file as one string; `stdin_lines()` — everything as a list of lines, endings stripped. The stdin pair is what makes olang pipe-friendly: `cat access.log \| olang analyze.ol` |
| Environment | `get_env` `set_env` `remove_env` `has_env` `list_env` |
| Directories | `cwd` `chdir` `home_dir` `temp_dir` |
| System | `hostname` `username` `os_type` `arch` `family` `path_separator` |
| Signals | `on_interrupt()` — trap Ctrl-C (SIGINT) instead of terminating (`Result`: installing the handler can fail); `interrupted()` — has it been pressed? (`Bool`); `reset_interrupt()` — clear the flag |

`os.exec` runs an external program to completion and returns
`Ok({ code, stdout, stderr })` — or `Err` if it could not be launched at
all. An optional third argument configures the child:
`#{ "cwd": dir, "stdin": text, "env": #{ name: value } }` (any subset).
`os.args()` is the program's argv (`[script, arg1, ...]`).

Most of `os` cannot fail, and as of 0.64 says so: `args`, `arch`,
`os_type`, `family`, `pid`, `path_separator`, `temp_dir`, `username`,
`is_tty`, `flush`, `has_env`, `list_env`, `set_env`, `remove_env`,
`interrupted`, and `reset_interrupt` all return their value directly.
The ones that keep `Result` are the ones that genuinely can fail:
`get_env` (the variable may be absent), `cwd`, `chdir`, `home_dir`,
`hostname`, `exe_path`, `exec`, `read_line`, `stdin`, `stdin_lines`, and
`on_interrupt`.

```olang no-run
let args = os.args()
let target = if len(args) > 1 => args[1] else => "."

let r = unwrap(os.exec("git", ["status", "--short"], #{ "cwd": target }))
if r.code == 0 => print(r.stdout)
else => println("git failed: " + r.stderr)
```

For a long-running loop or server, `os.on_interrupt()` traps Ctrl-C so it
sets a flag instead of killing the process; poll it to shut down cleanly:

```olang no-run
unwrap(os.on_interrupt())
while os.interrupted() == false {
    serve_one_request()
}
println("draining and exiting")
```

## `proc` — child processes and pipelines

Where `os.exec` runs a command to completion and hands back its whole
output, `proc` keeps the child *live*: feed its stdin, read its stdout a
line at a time, wait for its exit code, or kill it. It is the primitive
for streaming filters and long-running tools. Handles are `Process`
values into a process-wide registry (the same pattern as `chan`); stdout
and stderr are drained on background threads, so a chatty child never
deadlocks against a caller reading only one stream. Native-only.

| Group | Functions |
|---|---|
| Spawn | `spawn(program, args)` / `spawn(program, args, #{ cwd, env })` → `Ok(Process)` |
| Input | `write(p, s)` · `write_line(p, s)` · `close_stdin(p)` (signals EOF) |
| Output | `read_line(p)` → `Ok(line)` \| `Err("eof")`; `read_all(p)` → the rest of stdout; `stderr(p)` → all stderr (complete after exit) |
| Lifecycle | `wait(p)` → `Ok(#{ code })` · `kill(p)` · `pid(p)` |
| Pipeline | `pipeline(stages)` / `pipeline(stages, #{ cwd, env, stdin })` |

```olang no-run
// Stream a filter: feed lines in, read matches back out.
let p = unwrap(proc.spawn("grep", ["olang"]))
proc.write_line(p, "olang rules")
proc.write_line(p, "nope")
proc.close_stdin(p)
println(unwrap(proc.read_line(p)))     // "olang rules"
println(show(unwrap(proc.wait(p)).code))
```

`proc.pipeline` chains commands the way the shell's `a | b | c` does —
each stage is a list `[program, ...args]`, and each one's stdout is wired
to the next one's stdin. It returns `Ok(#{ code, stdout, stderr, codes })`:
the last stage's stdout, all stderr concatenated, the final exit code, and
the per-stage codes list. `opts.stdin` feeds the first stage.

```olang no-run
let r = unwrap(proc.pipeline([
    ["printf", "one\ntwo\nfour\n"],
    ["grep", "o"],
    ["wc", "-l"]
]))
println(str.trim(r.stdout))    // "3"
println(show(r.codes))         // [0, 0, 0]
```

The [`watch` example](../examples/tools/watch/) is the flagship: it streams a
command's output, runs pipelines, and shuts down gracefully on Ctrl-C.

## `cli` — command-line argument parsing

`use cli` — an embedded olang package — turns a command-line
program's argument surface into a **declarative spec** and does the
parsing for you: typed flags, positional arguments, subcommands,
`--help`, and precise error messages, in the spirit of what `argparse`
or `clap` provide. A spec is a plain map; `cli.parse(spec, argv)`
returns `Ok(values)` — a map holding every flag and argument by name,
plus a boolean `help` — or `Err(message)`. `cli.help(spec)` renders
the usage text, and `cli.args()` is `os.args()` with the program path
already dropped.

| Function | Description |
|---|---|
| `cli.parse(spec, argv)` | parse an argv list; `Ok(values)` or `Err(diagnostic)` |
| `cli.help(spec)` | the usage/help text as a string |
| `cli.args()` | the program's own arguments (program path removed) |

A spec's `flags` each carry a `name` (the long form and result key), an
optional `short`, a `type` (`"bool"`, `"int"`, `"float"`, or the
default `"string"`), an optional `default`, `required`, `env` (an
environment-variable fallback), and `help`. `args` are positional, in
order, each with `name`, `required`, `default`, and `help`. A spec with
`commands` becomes a subcommand dispatcher — the first token selects the
command, and the result carries `command`. Flags accept both
`--count 3` and `--count=3` (and `-n 3` / `-n=3`); `-h`/`--help`
short-circuits with `help` set true.

```olang no-run
use cli
let spec = #{
    "name": "greet", "about": "Greet someone",
    "flags": [
        #{ "name": "loud", "short": "l", "type": "bool", "help": "SHOUT it" },
        #{ "name": "count", "short": "n", "type": "int", "default": 1,
           "help": "repeat N times" }
    ],
    "args": [ #{ "name": "who", "required": true, "help": "who to greet" } ]
}
match cli.parse(spec, cli.args()) {
    Err(e) => { println("greet: " + e); os.exit(2) },
    Ok(a) => if map_get(a, "help") => println(cli.help(spec))
        else => {
            let line = map_get(a, "who")
            let shout = if map_get(a, "loud") => str.to_upper(line) else => line
            for i in range(0, map_get(a, "count")) { println("Hello, " + shout + "!") }
        }
}
```

[`examples/tools/taskcli`](../examples/tools/taskcli/) is the worked example — a
small task tracker whose entire command surface (`list`, `open`,
`stats`, `add --priority`) is one `cli` spec, with `--help` and clean
exit codes for free.

## `term` — the terminal toolkit

`use term` — an embedded olang package — is what `plot`/`viz` are for
the browser, for the terminal: color and text styling, aligned tables
and rules, progress bars, and interactive prompts. Styling is emitted
**only when it will render** — standard output is a TTY and `NO_COLOR`
is unset — or when `CLICOLOR_FORCE` is set (the convention that also
makes styled output testable through a pipe). The check is per call, so
a program's output is colored on a terminal and plain in a pipe or file
with no extra logic.

| Group | Functions |
|---|---|
| Color | `red` `green` `yellow` `blue` `magenta` `cyan` `white` `black` `gray` — each wraps a string |
| Attributes | `bold` `dim` `italic` `underline` |
| General | `style(s, opts)` — `opts` is `#{ "fg": ..., "bg": ..., "bold": ..., "dim": ..., "italic": ..., "underline": ... }`; `color()` — is styling active right now? |
| Structure | `rule(width)` — a horizontal line; `table(headers, rows)` — columns aligned to their widest plain cell, header bold |
| Progress | `bar(fraction, width)` — a `[████░░░░]  50%` bar string; print it with a leading `\r` and `os.flush()` to redraw in place |
| Input | `prompt(question)` — a line from stdin; `confirm(question)` — yes/no → bool; `select(question, options)` — a numbered menu → `Ok(chosen)` \| `Err` |

Because color falls back to plain automatically, the same program is
correct piped or interactive:

```olang no-run
use term
println(term.green("✓") + " built " + term.bold("olang") + " in " + term.cyan("1.2s"))
println(term.table(["name", "commits"], [["ada", "128"], ["evelyn", "12"]]))

// A download loop redrawing one line in place:
for i in range(0, 21) {
    print("\r" + term.bar(to_float(i) / 20.0, 24))
    os.flush()
    time.sleep(30)
}
println("")

if term.confirm("deploy now?") => run_deploy()
```

The [`taskcli` example](../examples/tools/taskcli/) uses it for a colored
summary and a `term.table` breakdown — both of which print plain when
its output is piped (which is why the examples harness still sees clean
text).

## `http` — HTTP

Both sides of HTTP in one module: a client for calling APIs, and
`http.serve` — a real, blocking HTTP/1.1 server with keep-alive and a
bounded worker pool. The design leans on the rest of the language rather
than inventing its own idioms: requests are fallible so they return
`Result`, a response is an ordinary struct-like value, and a server
handler is just a function from request to response. (`no-run`: network.)

A client response carries `status` (Int), `body` (String), `headers`
(a map of lowercased header name to first value), and `success` (Bool,
true for 2xx):

| Function | Description |
|---|---|
| `http.get(url[, opts])` / `http.post(url, body[, opts])` / `http.put` / `http.delete` | requests |
| `http.request(method, url, body[, opts])` | any method |
| `http.parse_url(url)` | split a URL into parts |
| `http.encode_query(map)` / `http.decode_query(s)` | query strings |
| `http.serve(port, handler[, options])` | serve `handler(request)` on a bounded worker pool; blocks the calling program |
| `http.response(status, body)` / `http.response_with_headers(status, body, headers)` | build responses |

Every client verb takes an optional trailing **options map** — this is
how a request carries headers, a timeout, and authentication:

| Option | Meaning |
|---|---|
| `"headers"` | map of header name → string value |
| `"timeout_ms"` | whole-request timeout in milliseconds |
| `"bearer"` | sets `Authorization: Bearer <token>` |
| `"basic"` | `(user, password)` tuple: HTTP basic auth |

An unknown option key raises rather than being ignored — a typo'd option
must not become a request that quietly lacked its auth header.

```olang no-run
let resp = unwrap(http.get("https://api.example.com/me", #{
    "bearer": os.get_env("API_TOKEN") |> unwrap,
    "timeout_ms": 5000,
    "headers": #{ "accept": "application/json" },
}))
println(to_string(resp.status))
println(show(map_get(resp.headers, "content-type")))
let data = unwrap(json.parse(resp.body))
println(data.message)
```

### Serving

`http.serve(port, handler)` binds `127.0.0.1:port` (port `0` picks a free
one, reported on stdout as `listening on http://127.0.0.1:PORT`; the
`bind` option names another address, `"0.0.0.0"` for the machines on
the network) and blocks
the calling program while a bounded worker pool handles independent
connections concurrently. The default worker count is the host's available
parallelism; `OLANG_HTTP_WORKERS` overrides it. Each worker owns an isolated
interpreter clone, while stateful native handles such as SQLite connections
synchronize their own access.

HTTP/1.1 keep-alive is honored. Idle connections time out, each connection
has a request cap, and a full bounded queue receives `503` rather than growing
threads or memory without limit. The handler receives a request struct:

| Field | Contents |
|---|---|
| `req.method` | `"GET"`, `"POST"`, ... (uppercased) |
| `req.path` | the path, without the query string |
| `req.query` | map of decoded query parameters |
| `req.headers` | map of headers, keys lowercased |
| `req.body` | the request body as a string |
| `req.remote_addr` | client socket address (`ip:port`) |

An optional map/object configures the bounded server:

| Option | Default |
|---|---|
| `workers` | available host parallelism / `OLANG_HTTP_WORKERS` |
| `queue_capacity` | `workers * 64` (minimum 64) |
| `max_requests_per_connection` | `100` |
| `idle_timeout_ms` | `5000` |
| `write_timeout_ms` | `10000` |
| `max_header_bytes` | `65536` |
| `max_body_bytes` | `10485760` (10 MB) |
| `request_timeout_ms` | `30000` |
| `bind` | `"127.0.0.1"` — the address to listen on |

The handler returns either a bare string (a `200 text/plain`) or a response
built with `http.response`/`http.response_with_headers` — pass headers as a
map literal so keys like `Content-Type` can contain `-`. A handler error
becomes a `500`; the server keeps running through both.

Abuse is answered precisely, with the connection closed: a malformed
request line or unparseable `Content-Length` is a `400`, a header block
past `max_header_bytes` a `431`, a declared body past `max_body_bytes` a
`413`, and a request that has not arrived in full within
`request_timeout_ms` a `408` — the whole-request deadline is what stops
a client dribbling one byte at a time from holding a worker forever,
which per-read timeouts alone cannot. Header values a handler returns
are flattened to one line (CR and LF become spaces), so interpolating
untrusted text into a header cannot split the response or inject
headers. The abuse suite in `tests/http_abuse_test.rs` pins all of
this.

```olang no-run
fn handle(req) = {
    if req.path == "/hello" => "hi, " + to_string(map_get(req.query, "name"))
    else if req.method == "POST" =>
        http.response_with_headers(201, req.body, #{ "Content-Type": "application/json" })
    else => http.response(404, "no route for " + req.path)
}
http.serve(8080, handle, #{ "workers": 8, "queue_capacity": 512 })
// blocks the calling program; Ctrl-C to stop
```

See [`examples/web/webserver/`](../examples/web/webserver/) for a complete JSON API
with a router (`:id` path parameters) over a SQLite store, and
[`examples/web/app/`](../examples/web/app/) for a full-stack issue tracker — the
same server also delivering its own browser frontend from disk.

## `db` — SQLite

A connection handle flows through every call. `:memory:` gives a fresh
in-process database — the example below really runs.

| Function | Description |
|---|---|
| `db.open(path)` | open or create (`":memory:"` for in-memory) |
| `db.execute(conn, sql)` / `db.execute(conn, sql, params)` | run a statement; `?` placeholders |
| `db.query(conn, sql)` / with `params` | rows as a list of maps |
| `db.query_one(conn, sql)` | the first row as a map, or `Ok(())` when there is none — unambiguous, since a row is always a map |
| `db.transaction(conn, f)` | `f(conn)` inside a transaction: commit unless `f` returns an `Err` or raises, which roll back; `f`'s result is handed through |
| `db.migrate(conn, steps)` | bring the schema to the head of `steps` (a list of versions, each a list of SQL); a `schema_version` table records progress, each version runs in its own transaction, a failing statement is named — `Ok(version)` |
| `db.begin(conn)` / `db.commit(conn)` / `db.rollback(conn)` | transactions by hand |
| `db.close(conn)` | close the handle |

```olang
let conn = unwrap(db.open(":memory:"))
unwrap(db.execute(conn, "CREATE TABLE scores (name TEXT, points INTEGER)"))
unwrap(db.begin(conn))
unwrap(db.execute(conn, "INSERT INTO scores VALUES (?, ?)", ["ada", 99]))
unwrap(db.execute(conn, "INSERT INTO scores VALUES (?, ?)", ["bob", 82]))
unwrap(db.commit(conn))

let rows = unwrap(db.query(conn, "SELECT name, points FROM scores ORDER BY points DESC"))
for row in rows {
    println(`${map_get(row, "name")}: ${map_get(row, "points")}`)
}
unwrap(db.close(conn))
```

## `dom` — the browser

When olang runs in a browser (as the WebAssembly build behind the
playground), the page itself becomes a device the program can drive:
`dom` is that device's API. olang does not wrap the DOM object model:
it treats the page as a *rendering target with controls*. You query
elements, wire events, render by writing HTML, reach for node-level
operations when a full re-render would be too blunt, and schedule work
with timers and animation frames; everything else is ordinary olang.

| Function | Description |
|---|---|
| `dom.query(sel)` | first element matching a CSS selector — an error if none matches |
| `dom.get_text(el)` / `dom.set_text(el, s)` | read / write an element's text content |
| `dom.set_html(el, html)` | replace an element's inner HTML — the render primitive |
| `dom.value(el)` / `dom.set_value(el, s)` | read / write a form control's value |
| `dom.focus(el)` | focus an element |
| `dom.active_id()` | the id of the focused element, or `""` — the read side of `focus`, so a repaint can leave an in-progress edit alone |
| `dom.prefers_dark()` | does the page prefer a dark color scheme — what a themed canvas or SVG needs to pick its palette |
| `dom.confirm(message)` | the browser's yes/no dialog, `true` on accept |
| `dom.read_file(el, callback)` | the first file a file input holds, delivered to the callback as `#{ "name", "size", "type", "base64" }` (or `#{ "error": … }`) |
| `dom.on(el, event, handler)` | attach an event handler (see below) |
| `dom.fetch(method, path, body, callback)` | asynchronous HTTP from the page — the callback receives the response text |
| `dom.fetch_json(method, path, body, callback)` | `dom.fetch`, but the callback receives the parsed value directly |
| `dom.request(method, path, body, callback)` | the whole response — `#{ "status", "headers", "body" }`, status `0` with an `"error"` when no server answered — so a handler tells a 404 from a 500 from a network failure |
| `dom.request_with(method, path, body, headers, callback)` | `dom.request` with request headers — a bearer token, another content type |
| `dom.find(selector)` | the first match, or `()` when nothing matches — the lookup for an element that may be absent (`dom.query` raises on a miss, and a raise inside a handler takes the page down) |
| `dom.query_all(selector)` | every match, as a list of handles |
| `dom.get_attr(el, name)` / `set_attr(el, name, v)` / `remove_attr(el, name)` | attributes |
| `dom.class_add(el, c)` / `class_remove(el, c)` / `class_toggle(el, c)` | class list ops (`set_class` replaces wholesale) |
| `dom.set_style(el, prop, v)` | set one style property |
| `dom.measure(el)` | bounding rect as a Map: `x`, `y`, `width`, `height` |
| `dom.create(tag)` / `append(parent, child)` / `remove(el)` | surgical structure edits |
| `dom.scroll_into_view(el)` | scroll an element into view |
| `dom.set_timeout(ms, fn)` / `set_interval(ms, fn)` / `clear_interval(t)` | timers |
| `dom.request_frame(fn)` | one animation frame; re-arm inside the handler for a loop |
| `dom.on_frame(fn)` | the persistent animation loop: register once, called every frame with a millisecond `delta` |
| `dom.draw(canvas, ops)` | replay a draw-list onto a canvas — the whole scene crosses the boundary once |
| `dom.draw_points(canvas, xs, ys, style)` | the bulk path: coordinates cross as one packed binary buffer (Series or lists; nulls drop pairwise); style takes `mode` (`"points"`/`"path"`), `color`, `size`, `alpha`, a rotation `rot` (radians), and an affine `sx`/`sy`/`tx`/`ty` — all applied host-side |
| `dom.insert_before(parent, child, before)` | position a child (`0` appends) |
| `dom.push_state(path)` / `dom.location()` | SPA navigation; location is a Map of `path` and `query` |
| `dom.on_route(fn)` | the back/forward listener — a `route` event Map with `path` and `query` |
| `dom.storage_get(k)` / `storage_set(k, v)` / `storage_remove(k)` | localStorage (missing keys read as `""`) |
| `dom.state_get(k)` / `state_set(k, v)` | session state — a JSON-typed, page-lifetime store (a Map/list round-trips; missing keys read as Unit; not persisted) |
| `dom.worker(path)` | boot a second olang program in a Web Worker; returns a worker handle |
| `dom.worker_send(w, value)` / `dom.worker_on(w, handler)` | send a value to / receive values from a worker |
| `dom.worker_close(w)` | terminate a worker |
| `dom.post(value)` / `dom.on_message(handler)` | the worker-side mirrors: post a value to the page / receive values from it |

**Every event handler receives a structured event Map** — the same
shape for every event type, so handlers pick the fields they need:
`type`, the target's `id` and `value`, `key`, pointer `x`/`y`, the
modifier flags `alt`/`ctrl`/`shift`/`meta`, and `data` (a Map of the
target's `data-*` attributes). Any DOM event name works — `click`,
`input`, `keydown`, `pointermove`, `submit`, `focus`, `wheel`, … — plus
`enter`, the keydown-filtered alias. Delegation is the natural style:
one listener on a container, dispatch on `id` or `data`, re-render
freely without rebinding.

**Rich graphics are a draw-list.** A scene is plain olang data — a
list of op maps — submitted with one `dom.draw` call per frame; the
page replays it onto the canvas 2D context. Ops: `clear` (with a color
for motion trails, or without to wipe), `rect`, `circle`, `line`,
`path` (a `points` list, optionally closed), `text`, and the
transforms `save`/`restore`/`translate`/`rotate`/`scale`. Fill and
stroke take any CSS color; `line_width` sets stroke width. Paired with
`dom.on_frame`, that is a 60fps rendering loop in ordinary olang — see
`examples/web/app/static/orbit.ol`, an animated orbital system served by
the tracker at `/orbit.html`, where a click adds a body at the clicked
radius (structured event coordinates + `dom.measure`).

**Parallelism is a second program.** `dom.worker(path)` fetches an
olang source file and boots it in a Web Worker — its own thread, its
own wasm instance, no DOM. The two sides exchange plain values
(anything `json.stringify` can carry): the page speaks
`dom.worker_send` / `dom.worker_on`, the worker speaks `dom.post` /
`dom.on_message`. Closures do not cross — programs and messages do,
which is the same discipline as `chan` on native. A worker may `post`
mid-computation, so long jobs stream progress while the page's frame
loop never misses a beat. See `examples/web/app/static/primes.ol` and
`primes-worker.ol` — served by the tracker at `/primes.html`, a prime
counter whose progress bar fills while an animation dial proves the
main thread stayed live.

**Declarative views: the `ui` module.** `use ui` (an embedded olang
package) builds pages as values: `h(tag, attrs, children)` makes a node,
strings are text (escaped on render), `hk` adds a reconciliation key,
and `html(tree)` renders to a string — pure and testable anywhere.
`ui.render(el, children)` mounts a keyed list and reconciles against
the previous render: unchanged children are untouched (input state and
focus survive), changed ones re-render in place, added and removed keys
insert and remove surgically, and reorders reposition without
rebuilding. See `examples/web/app/static/notes.ol` — a small SPA at
`/notes.html` combining `ui.render`, `push_state`/`on_route`
navigation, and localStorage persistence.

| Function | Purpose |
|---|---|
| `h(tag, attrs, children)` | Build a virtual node — a tag, an attribute map, and a list of child nodes or text strings |
| `hk(key, tag, attrs, children)` | Like `h`, plus a stable reconciliation key for keyed lists |
| `esc(s)` | HTML-escape a string (`&` `<` `>` `"` → entities); applied automatically to text on render |
| `html(node)` | Render a node tree to an HTML string — pure, testable without a browser |
| `render(el, children)` | Mount and reconcile a keyed child list into a live DOM element (browser only) |

`h`, `hk`, `esc`, and `html` are pure and run anywhere; only `render`
needs the browser.

`dom` is the one browser-only module: in a native build every call
reports that it needs the wasm build (mirroring how `fs`, `os`, `http`,
and `db` are absent from the browser). The example is therefore
`no-run` — it runs on a page, not in the test harness:

```olang no-run
dom.set_html(dom.query("#list"),
    ["a", "b"] |> map((s) => "<li>" + s + "</li>") |> join(""))
dom.on(dom.query("#list"), "click", (e) => select(map_get(e, "id")))
dom.on(dom.query("#new-title"), "enter", (e) => add_item(map_get(e, "value")))
dom.fetch("GET", "/api/items", "", (resp) => render(unwrap(json.parse(resp))))
dom.on_frame((f) => {
    dom.draw(canvas, [
        #{ "op": "clear", "color": "rgba(11,14,20,0.35)" },
        #{ "op": "circle", "x": 360.0, "y": 240.0, "r": 16.0, "fill": "#f5c542" }
    ])
})
```

The module has its own chapter, **[olang in the Browser](wasm.md)**:
element handles and their lifetime, the event payload conventions that
make delegation the natural style, `dom.fetch`'s callback contract, the
stateless-frontend architecture, and a guided reading of
[`examples/web/app/`](../examples/web/app/) — the issue tracker whose frontend
is olang running as WebAssembly.

## `testing` — assertions

olang has two assertion surfaces, and the difference matters. Inside
[`test` blocks](language.md#testing), `assert_eq(a, b, "msg")` and
friends are *language-level* forms that **raise** on failure — that is
what makes a failing test fail. The `testing` module is the other
surface: assertion *functions* that return the outcome as a `Result`
value (`Ok(())` on success, `Err(message)` on failure) so a program can
inspect, collect, and report failures itself — the raw material for a
custom harness. A returned `Err` does nothing on its own; `unwrap` it
(or match it) to act on it.

| Function | Description |
|---|---|
| `testing.assert_eq(a, b)` / `assert_ne(a, b)` | equality checks |
| `testing.assert_true(x)` / `assert_false(x)` | boolean checks (non-boolean input is an `Err`) |
| `testing.assert_ok(r)` / `assert_err(r)` | Result checks |
| `testing.fail(msg)` | unconditional `Err` |
| `testing.snapshot(name, value)` | the snapshot test: `value`'s display form against `__snapshots__/<name>.snap` beside the file under test — written on the first run, compared after, refreshed by `OLANG_UPDATE_SNAPSHOTS=1` |
| `testing.test_summary()` / `testing.reset_tests()` | the session's assertion tally: `#{ "passed", "failed", "total" }` over every `assert_*` outcome on this thread; `reset_tests()` zeroes it |
| `testing.run_test(name, f)` | deliberately a redirect: a builtin cannot re-enter the interpreter to run `f`, and the error says to use a `test` block instead |

```olang
println(to_string(is_ok(testing.assert_eq(2 + 2, 4))))
println(to_string(is_err(testing.assert_eq(2 + 2, 5))))
unwrap(testing.assert_ok(str.parse_int("5")))   // unwrap makes it fatal
println("assertions passed")
```

For ordinary tests, prefer `test` blocks and the language-level
assertions — [`olang test`](tooling.md#olang-test) discovers and runs
them with reporting built in.

## `ods` — Series and Frames

The heart of the data stack: a **Series** is a typed, null-aware 1-D
column of `Int`, `Float`, `Bool`, or `String` over a contiguous native
buffer; a **Frame** is a table of named, equal-length Series. Operators
are vectorized (`s * 2.0` runs one kernel over the whole column;
`s > 2` yields a Bool mask for `ods.filter`), nulls propagate through
arithmetic and are skipped by reductions, and the Frame verbs —
`read_csv`, `select`, `drop`, `rename`, `with_column`, `filter`,
`distinct`, `sort_by`, `group_by`, `join`, `to_records` — all chain
with `|>`. CSV and JSON lines each have
a text parser, a file reader, a streaming reader, and a serializer; the
calls that reach the filesystem are the module's only ones that demand
the `fs` capability, and the text parsers and serializers stay pure.

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n")
let full = ods.with_column(sales, "revenue", sales["amount"] * sales["qty"])
let summary = full
    |> ods.group_by("region", [["total", "sum", "revenue"], ["n", "count"]])
    |> ods.sort_by("total", true)
for rec in ods.to_records(summary) {
    println(`${map_get(rec, "region")}: ${map_get(rec, "total")}`)
}
```

**Bulk operations parallelize themselves when it is invisible to do
so.** A `map` or `filter` over 50,000 or more elements whose kernel is
*provably pure* — a conservative whitelist over its AST: arithmetic,
control flow, the pure builtins and modules (`math`, `str`, `json`,
`re`, …), no user-function calls, no effects — fans out across every
core automatically. Because the kernel is pure and the results are
joined in order, the outcome is bit-identical to the sequential run:
same values, same order, same first error, nothing for record/replay to
see. Anything the proof cannot vouch for stays sequential, so effects
always happen in element order; `set_parallel(false)` disables the
fan-out entirely, and `OLANG_DEBUG_AUTOPAR=1` explains any decision.
Measured on 18 cores: a 3M-element pure map runs ~3.7× faster with no
change to the program.

The data stack fans out the same way. `ods.read_csv` splits large text
at record boundaries (quote-aware, so embedded newlines never split a
field) and parses the runs concurrently; `ods.read_jsonl` parses lines
concurrently and reports the lowest-numbered bad line, exactly as a
sequential scan would; `ods.sort_by`, joins, and `ods.group_by` sort,
probe, gather, and aggregate across cores above ~100k rows. Every
fan-out joins in a fixed order, so results are cell-identical to the
sequential run (a grouped float `sum` may differ in the last ULPs, as
its reference already notes), and `set_parallel(false)` turns all of it
off. Measured at 1M rows: JSONL parse 4.3× faster, CSV parse 2.7×,
sort_by ~1.9×.

### Series verbs

Arithmetic, comparison, and math *operators* are vectorized directly
(`s * 2.0`, `s > 2`, `s + t`); the named verbs are:

| Group | Verbs |
|---|---|
| Create | `series(list\|range)` · `try_series(list)` (the Result form: a mixed-type list is `Err`, not a raise) · `zeros(n)` (n float zeros) · `linspace(a, b, n)` (n evenly spaced floats, inclusive) |
| Elementwise | `map(s, "sin")` — apply a `math.*` unary fn over the column in one kernel pass |
| Comparisons | `eq(a, b)` · `ne(a, b)` — elementwise masks between two Series (against a *scalar*, use the `s == v` / `s != v` operators) |
| Nulls | `is_null(s)` (mask) · `fill_null(s, v)` · `null_count(s)` |
| Reductions | `sum` `mean` `var` `std` `min` `max` `median` (skip nulls; `var`/`std` are sample) · `quantile(s, q)` · `cumsum(s)` · `dot(a, b)` |
| Windows | `shift(s, by)` (nulls at the edge) · `cum_max` · `cum_min` · `rank(s, method = "min")` — min/max/average/ordinal/dense · `rolling(s, window, agg)`, null until the window fills |
| Distinct | `unique(s)` (first-seen order) · `n_unique(s)` · `value_counts(s)` → a value/count Frame, most frequent first |
| Dates | `date_part(s, "year"\|"month"\|"day")` — the prefix of an ISO date or timestamp String Series, as a String Series: the derived key for a by-month `group_by` |
| Convert type | `cast(s, "Float"\|"Int"\|"Bool"\|"String")` — what the target cannot hold becomes null, so `null_count` reports what was lost |
| Order / select | `sort(s)` (nulls last) · `argsort(s)` (sorting indices) · `take(s, idx)` (gather) · `get(s, i)` (negative counts from end) |
| Convert | `to_list(s)` (nulls → `()`) · `len(s)` |

### Frame verbs

| Group | Verbs |
|---|---|
| Build | `frame(columns)` · `frame_from_records(records)` · `try_frame_from_records(records)` (the Result form, for records from a file, a request, or a user) · `read_csv(text)` |
| Files | `read_csv_file(path)` · `write_csv(f, path)` · `to_csv(f)` · `read_jsonl(text)` · `read_jsonl_file(path)` · `write_jsonl(f, path)` · `to_jsonl(f)` |
| Streaming | `open_csv(path)` · `open_jsonl(path)` — then `next_chunk(r, n)` · `rows_read(r)` · `at_end(r)`, the same verbs for either |
| Native format | `write_frame(f, path)` · `read_frame(path, columns)` · `frame_info(path)` — exact types, faster loads, one column at a time |
| Inspect | `columns(f)` · `n_rows(f)` · `n_cols(f)` · `head(f, n)` · `tail(f, n)` · `describe(f)` · `schema(f)` · `to_records(f)` · `to_matrix(f, names)` — Float/Int columns as a list of Float lists, for `vec` |
| Splits | `split(f, frac)` — a deterministic holdout under `random.seed`, `[kept, rest]` · `split_at(f, column, frac)` — the forward-in-time split: sorted by `column`, the first `frac` first |
| Version | `ods.version()` — the engine's version string, worth quoting in a bug report |
| Subscript | `f["name"]` a column · `f[mask]` the rows a Bool Series keeps · `s[i]` an element (negatives from the end) · `column(f, name)` is the same as `f[name]`, for when a call reads better than a subscript |
| Shape | `select(f, names)` · `drop(f, names)` · `rename(f, mapping)` · `with_column(f, name, series)` · `filter(f, mask)` · `sort_by(f, name, descending)` |
| Whole rows | `distinct(f, names = all)` · `drop_null(f, names = all)` — duplicates and missing data, first occurrence kept |
| Reshape | `pivot(f, index, columns, values, agg)` long→wide · `unpivot(f, ids, value_columns = the rest)` wide→long, emitting `name`/`value` |
| Sample | `sample(f, n)` (also on a Series) — n random rows, without replacement, in original order; seeded by `random.seed` |
| Aggregate / join | `group_by(f, key, aggs)` · `join` (inner) · `join_left` · `join_full` · `join_semi` · `join_anti` — all `(a, b, on, on_b = on)` · `concat(frames)` |
| Masks | `eq(s, v)` · `ne(s, v)` · `all_of(masks)` · `any_of(masks)` · `not(mask)` — three-valued, like SQL |

The stack has its own chapter, **[The Data Stack](ods.md)**: why the
columnar model wins (with the measured 50× rewrite behind it), every
Series and Frame verb with its semantics, null handling, joins and
grouped aggregation, and the performance characteristics — all taught
rather than merely listed. Engineering history and benchmark method
live in [that chapter's design record](ods.md#the-design-record).

## `stats` — statistical inference

Distributions (`norm`, `t`, `chi2`, `f` — each with `pdf`/`cdf`/`ppf`/
`sample`), `describe`, correlation and covariance, t-tests (one- and
two-sample), the χ² goodness-of-fit test, and OLS regression via
`stats.lm` — every statistic pinned against scipy reference values in
the test suite. Tests and fits return maps; sampling draws from the
`random` module's seeded stream, so `random.seed` makes simulations
reproducible.

```olang
let a = ods.series([5.1, 4.9, 6.2, 5.7, 5.5, 4.8, 5.9, 6.1])
let b = ods.series([4.2, 4.8, 4.5, 5.0, 4.4, 4.1, 4.9])
let t = stats.t_test(a, b)
println(`p = ${map_get(t, "p_value")}`)
println(to_string(stats.norm.ppf(0.975, 0.0, 1.0)))   // 1.9599...
```

**[The Data Stack](ods.md#stats--from-description-to-inference)**
teaches the module end to end — including a complete regression
workflow and how to read p-values and `r2` honestly.

## `plot` — charts as SVG text

Charts render to complete standalone SVG documents as strings — write
one with `fs.write_file`, serve it over `http`, or land it on a page
with `dom.set_html`. `plot.line`, `plot.scatter`, `plot.area`,
`plot.lines` (multi-series with legend), `plot.xy` (layered marks over
shared scales), `plot.bar`, `plot.bars` (grouped), `plot.stacked`,
`plot.hist`, `plot.heatmap`, and `plot.box` take Series data plus one
options map (`title`, `x_label`, `y_label`, `width`, `height`, `theme`
— `"dark"` re-tunes every color for a dark surface — `responsive`,
which sizes the SVG to its container, `font_size` and `font` for the
text — the default 12.5 suits a 720×440 document, a chart drawn into a
narrow column wants less — plus the color system below; unknown keys
are errors). Defaults follow a colorblind-validated
ten-hue palette, so a chart is presentable with `#{}` — and color is
an option, not a fate: `colors` gives a chart its own palette, `vary`
colors bar categories individually, `scale` picks a heatmap ramp
(`ocean`/`ember`/`thermal`/`diverging`), and `plot.ramp(name, t)`
hands the same ramps to your code. In `viz`, `"color_by": "column"`
maps values onto a ramp per point — continuous color encoding on both
the SVG and canvas targets.

One level up, **`use viz`** (an embedded olang package) makes a chart
a *value*: a spec map with data (records or a Frame), a mark, and
column-name encodings — `color` splits series, `layers` composes
marks, and `viz.draw` compiles the same specs to canvas draw-lists in
the browser. A spec's keys are checked: `w` and `h` are accepted as the
short forms of `width` and `height`, and any key the grammar does not
know is an error naming it, so a misspelled option never renders
silently at the default. With `"interactive": true` marks carry their datum as
`data-*` attributes, and `viz.tooltip` / `viz.on_mark` / `viz.brush`
turn hover, click-to-filter, and brush-to-zoom into one-liners. See
[the Data Stack](ods.md#the-viz-grammar).

| `viz` function | Purpose |
|---|---|
| `chart(spec)` | Render a spec (data + mark + encodings) to a standalone SVG string |
| `draw(el, spec)` | Compile the same spec to canvas draw-lists on a browser element |
| `tooltip(el)` | Attach hover tooltips reading each mark's `data-*` datum |
| `on_mark(el, event, handler)` | Call `handler` with the datum when a mark fires `event` (e.g. click-to-filter) |
| `brush(el, handler)` | Drag-select a range on the chart, calling `handler` with the bounds |

Above both sits **`use dash`** — the dashboard kit. Its helpers build a
dashboard shell as pure, escaped HTML strings (natively tested), and
`dash.styles()` ships the CSS so a page needs none of its own:

| `dash` function | Purpose |
|---|---|
| `kpi(label, value, note)` | A single metric tile |
| `stat(label, value, note, accent)` | Like `kpi`, with an accent color |
| `card(title, inner)` | A titled panel wrapping arbitrary HTML |
| `half(title, inner)` | A two-column-span card for wide content |
| `wide(title, inner)` | A full-width card |
| `grid(cards, columns)` | Lay a list of cards out in an N-column grid |
| `styles()` | The `<style>` block the shell needs |

The wiring pattern stays in your program: fetch, compute, `dom.set_html`
the shell, fill the chart mounts with `viz.chart`. The tracker's
`/board.html` is the flagship: a KPI row, five charts, a URL-carried
status filter, and a 5-second auto-refresh in one source file.

```olang
let x = ods.linspace(0.0, 6.28, 50)
let y = ods.series(map(ods.to_list(x), (v) => math.sin(v)))
let svg = plot.line(x, y, #{ "title": "sin(t)", "x_label": "t" })
println(to_string(str.contains(svg, "<svg")))
```

**[The Data Stack](ods.md#plot--charts-as-svg-text)** covers the
SVG-as-text philosophy, each chart type, and null handling per chart.

---

Next: [Packages](packages.md) for multi-file programs and dependencies, or
[Internals](internals.md) for how the runtime executes all of this.
