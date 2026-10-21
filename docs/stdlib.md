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
- [`vec` — vectors over lists](#vec--vectors-over-lists)
- [`bigint` — arbitrary precision](#bigint--arbitrary-precision)
- [`asset` — a package's files](#asset--a-packages-files)
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
- [`compress` — gzip and deflate](#compress--gzip-and-deflate)
- [`base64` — base64](#base64--base64)
- [`fs` — file system](#fs--file-system)
- [`meta` — the program as data](#meta--the-program-as-data-the-open-ast)
- [`runtime` — what this binary carries](#runtime--what-this-binary-carries)
- [`caps` — what this code was granted](#caps--what-this-code-was-granted)
- [`os` — operating system](#os--operating-system)
- [`proc` — child processes and pipelines](#proc--child-processes-and-pipelines)
- [`pty` — a child on a pseudo-terminal](#pty--a-child-on-a-pseudo-terminal)
- [`vt` — a terminal's screen](#vt--a-terminals-screen)
- [`cli` — command-line argument parsing](#cli--command-line-argument-parsing)
- [`term` — the terminal toolkit](#term--the-terminal-toolkit)
- [`ui`, `viz` and `dash` — views, charts and dashboards](#ui-viz-and-dash--views-charts-and-dashboards)
- [`tty` — the terminal as an interactive device](#tty--the-terminal-as-an-interactive-device)
- [`gui` — windows drawn by olang](#gui--windows-drawn-by-olang)
- [`http` — HTTP](#http--http)
- [`db` — SQLite](#db--sqlite)
- [`dom` — the browser](#dom--the-browser)
- [`testing` — assertions](#testing--assertions)
- [`ods` — Series and Frames](#ods--series-and-frames)
- [`mat` — matrices](#mat--matrices)
- [`stats` — statistical inference](#stats--statistical-inference)
- [`learn` and `ml` — machine learning](#learn-and-ml--machine-learning)
- [`plot` — charts as SVG text](#plot--charts-as-svg-text)
- [`gg` — a grammar of graphics](#gg--a-grammar-of-graphics)

## Conventions

**Modules are always in scope.** `str.trim(...)`, `json.parse(...)`, and the
rest work without any `use`. The exception is the code written in olang
itself and embedded in the binary: the modules `colx` and `mathx` and
the packages `cli`, `term`, `ui`, `viz`, `dash`, `ml` and `gg` are
imported with a bare `use colx` / `use term`, which binds the namespace
alone: `term.bold(...)`, `gg.plot(...)`. List names for short ones
(`use gg { aes, labs }`, `use gg { plot as ggplot }`); an import never
rebinds a stdlib module's name (`plot` stays the plot module) unless it
lists it. The one embedded module that needs no
`use` is [`collections`](#collections-in-olang), which loads on first
touch. A `use str` is accepted, and keeps a file's import list
explicit, but is never required for the native modules.

**One environment note:** `fs`, `os`, `proc`, `pty`, `tty`, `gui`,
`http`, and `db` need an operating system and are absent from the
browser playground build, where calling them reports exactly that.
`dom` is the reverse: browser-only, an error everywhere else.

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

The same holds where an operation looks for something and finds
nothing: `re.find` with no match is `Ok(())`, a capture group that took
no part is `()`, `str.char_at` past the end is `()`, `http.parse_url`'s
missing port is `()`. And **"nothing yet" is `Ok(())` everywhere**: a
`chan.try_recv` with nothing waiting, a `pty.read` that timed out, a
`pty.wait` while the process runs. Three strings are the contract
where a wait or a stream ends instead, and stay as they are:

| Sentinel | Meaning | From |
|---|---|---|
| `Err("eof")` | the stream ended | `os.read_line`, `proc.read_line`, `pty.read` |
| `Err("timed out")` | the wait's time ran out | `task.join_timeout`, `chan.recv_timeout` |
| `Err("channel is closed")` | the channel is closed and drained | `chan.recv`, `chan.try_recv`, `chan.recv_timeout`, `chan.send` |

A program may match these strings; any other error's text is for
people and may change. `os.get_env(name)` keeps its `Err` for a
variable that is not set (a configuration that must be there), and
`os.env(name)` is the lookup: the value, or `()`.

The three shapes never mix: a lookup says `Unit`, a parse or I/O
operation says `Result`, a misused call raises. And because Unit *is*
absence, a stored Unit and a missing key are deliberately the same
answer — when the distinction matters, ask `map_has_key`. For the common
lookup-with-fallback, `m.key ?? d` is the one-expression form, and
`map_get_or(m, k, d)` the form for a computed key:

```olang
let m = #{ "a": 1 }
println(to_string(m.a ?? 0))   // 1
println(to_string(m.z ?? 0))   // 0
```

**Keys mean one thing everywhere.** A record's and an option's keys
are the same words in every module: `line` and `column` are 1-based (a
search hit, a frame, a hover in a styled field, a lens), a function is
`function`, a file on disk is `path`, a wait's limit is `timeout_ms`
(every function that waits with a limit takes it as a number of
milliseconds or as `#{ timeout_ms }`),
and an environment is `env` (a name given `()` removed) with
`clear_env`.

**Names follow three rules.** A function that raises on bad input and
has a legitimate use over data the program does not control gets a
`try_` twin returning `Result` (`ods.series` / `ods.try_series`,
`ods.frame_from_records` / `ods.try_frame_from_records`); a function that
already returns `Result` gets no twin. (`chan.try_recv` is not such a
twin: `chan.recv` already answers a `Result`, and `try_recv` is the
form that does not wait.)
A precision or unit variant carries its unit as a suffix
(`dates.stamp` / `dates.stamp_ms`, `time.sleep` in milliseconds,
`_bytes` for the binary form of a text function). And a name lives in
exactly one module — the `col.` prefixes of the global collection
helpers are aliases of the same function, never a second definition.
The 0.82 audit of every documented name against the docs and examples
found no duplicates beyond those aliases; the names used once or never
outside their own chapter are the RSA and certificate family in
`crypto`, the cell-editing family in `csv`, and the component parts of
`dates` (`hour`, `minute`, `add_weeks`, …). They are kept, because each
is the one place its job is done, and are listed here as the record for
any future review of the surface.

## Global builtins

### Output

| Function | Description |
|---|---|
| `print(v)` | write without newline |
| `println(...)` | write with newline; several arguments print space-separated |
| `eprint(v)` / `eprintln(...)` | the same, to standard error: diagnostics stay out of `prog > out.csv` (in the browser they join the output) |

### Introspection and conversion

| Function | Description |
|---|---|
| `typeof(v)` | runtime type name as a string |
| `show(v)` | the value as olang source writes it ([How values print](language.md#how-values-print)); a String is its text |
| `to_string(v)` | the same text as `show` (a String is its text) |
| `to_int(v)` | string/float → Int (floats truncate toward zero); a string that is not a number raises (`str.parse_int` is the `Result` form), and so does a Float outside the Int range (`to_int(1e30)`) or not finite |
| `to_float(v)` | string/int → Float; a string that is not a number raises (`str.parse_float` is the `Result` form) |
| `len(v)` | length of a list, a string (in characters), a tuple, or `Bytes`; a map raises, and its entry count is `map_len` |
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
| `range(n)` / `range(a, b)` / `range(a, b, step)` | list of integers from `a` (default 0) up to, excluding, `b`, by `step` (default 1) |
| `head(xs)` / `tail(xs)` | first element / all but the first |
| `first(xs)` / `last(xs)` | the first / last element, or `()` when the list is empty (also of a tuple, and the first / last character of a string) |
| `take(xs, n)` / `skip(xs, n)` | first `n` / all but the first `n` (`drop` is `skip` under the name most languages pair with `take`) |
| `reverse(xs)` / `sort(xs)` | reversed / ascending copy (numbers, strings, or tuples, which order lexicographically); the sort is stable: elements that compare equal keep their order |
| `contains(xs, v)` | membership test |
| `concat(a, b)` | join two lists (or use `a + b`) |
| `cons(v, xs)` | prepend an element |
| `chunk(xs, n)` | split into sublists of size `n` |
| `flatten(xss)` | flatten one level of nesting |
| `enumerate(xs)` | pair each element with its index: a list of `(i, v)` tuples |
| `zip(a, b)` | pair elements: list of tuples, shortest wins |
| `group_by(xs, f)` | list of `(key, [members])` tuples |

```olang
println(to_string(range(1, 5)))
println(`${cons(0, [1, 2])} ${flatten([[1], [2, 3]])}`)
println(to_string(chunk([1, 2, 3, 4, 5], 2)))
println(to_string(zip([1, 2], ["a", "b"])))
println(to_string(group_by([1, 2, 3, 4], (x) => x % 2)))
```

Most list helpers are also reachable under `col.` (`col.take`,
`col.drop`, `col.contains`, `col.map`, `col.fold`, …); the global
spelling is the short form. The ones that exist only as globals are
`range`, `chunk`, `cons`, `concat`, `enumerate`, `group_by`, `find`,
`reduce`, `par_map`, `par_filter`, `map_filtered`, `average`, `clamp`,
and `abs`. Two helpers exist only under `col.`: `col.index_of(xs, v)`
(position, or Unit) and `col.slice(xs, from, to)` (a half-open sub-list,
by the slices' one rule, `str.substring`'s):

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
| `abs(x)` | absolute value, an Int for an Int (`math.abs`) |

```olang
let xs = [4, 1, 7]
println(`${sum(xs)} ${min(xs)} ${max(xs)}`)
println(to_string(clamp(15, 0, 10)))
println(`${abs(-3)} ${abs(2.5)}`)   // 3 2.5
```

### Higher-order functions

| Function | Description |
|---|---|
| `map(xs, f)` | transform each element |
| `filter(xs, pred)` | keep matching elements |
| `par_map(xs, f)` | `map` fanned out across OS threads |
| `par_filter(xs, pred)` | `filter` fanned out across OS threads |
| `fold(xs, init, f)` | combine left to right from the accumulator `init`: `f(acc, x)` for each element |
| `reduce(xs, f)` | `fold` from the first element, with no initial value: `reduce([3, 9, 4], max_of)` is `max_of(max_of(3, 9), 4)`; a one-element list answers its element, an empty one `()` |
| `find(xs, pred)` | the first element `pred` accepts, or `()` when none does |
| `all(xs, pred)` / `any(xs, pred)` | does every / some element match? Stops at the first that settles it (`col.all` / `col.any`) |
| `map_filtered(xs, pred, f)` | fused filter-then-map in one pass |

All take the list first, so they chain with `|>`:

```olang
let total = [1, 2, 3, 4, 5]
    |> filter((x) => x % 2 == 1)
    |> map((x) => x * x)
    |> fold(0, (a, x) => a + x)
println(to_string(total))                       // 1 + 9 + 25
println(to_string(find([3, 8, 2], (x) => x > 5)))       // 8
println(to_string(find([3, 8, 2], (x) => x > 9) ?? 0))  // 0: none matched
println(to_string(reduce([3, 9, 4], (a, b) => if a > b => a else => b)))   // 9
println(`${all([2, 4], (x) => x % 2 == 0)} ${any([1, 3], (x) => x > 5)}`)   // true false
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

Read and update maps. **Every builtin here takes a map, an anonymous
object and a parsed JSON object alike** (and a declared struct), reading
*and* writing: updating a parsed JSON object yields a new JSON object,
updating an object an object, a struct a new struct of the same type:
a writer answers the kind of its first argument. Everything is
immutable, so writers return a new value. A value nothing else holds is
updated in place, so `m = map_set(m, k, v)`, `m = map_set_in(m, path, v)`,
`m = map_update(m, k, f)` and `m = map_merge(m, other)` cost the keys
they write, not the map's size. An identifier key reads as `m.key`
(exactly `map_get(m, "key")`) and defaults with `m.key ?? d` (exactly
`map_get_or(m, "key", d)`, the default evaluated only when used).

| Function | Description |
|---|---|
| `map_get(m, k)` | value, or `Unit` when absent (and `Unit` when `m` is `()`, as `().k` is) |
| `map_get_or(m, k, default)` | value, or `default` when absent (or stored Unit) |
| `map_get_in(m, [k1, k2, …])` | a nested read in one call — the value at the end of the key path, or Unit at the first missing hop; every hop reads like `map_get` |
| `map_set_in(m, [k1, k2, …], v)` | new value with the end of the key path set — each level a new value of its own kind, a missing level a new map |
| `map_update(m, k, f)` | new value with `k` set to `f(m.k)` (`f` is given `()` for an absent key) |
| `map_has_key(m, k)` | presence test (distinguishes absent from null) |
| `map_keys(m)` / `map_values(m)` | key/value lists, in the map's order (each key where it was first set; the order `entries` uses) |
| `entries(m)` | `(key, value)` tuples, in the map's order — for `for (k, v) in` |
| `map_len(m)` | entry count (an object's or a JSON object's fields) |
| `map_set(m, k, v)` | new value of the same kind with `k` set |
| `map_remove(m, k)` | new value of the same kind without `k` |
| `map_merge(a, b)` | new value of `a`'s kind, `b`'s entries winning (`b` any of the kinds); `#{ ...a, ...b }` is the literal form |
| `map_clear(m)` | an empty value of the same kind |

```olang
let m = #{ "a": 1, "b": 2 }
let m2 = #{ ...m, "c": 3 }
println(show(map_len(m)) + " -> " + show(map_len(m2)))
println(show(map_get({ x: 42 }, "x")))            // objects read like maps
let d = map_set(unwrap(json.parse("{\"a\": 1}")), "b", 2)
println(typeof(d) + " " + show(d.b))              // JSON updates stay JSON
let o = map_merge({ x: 1 }, #{ "y": 2 })
println(typeof(o) + " " + show(map_len(o)))       // Object 2
let s = #{ "pp": #{ "cur": 1 } }
println(show(map_set_in(s, ["pp", "cur"], 2)))    // #{"pp": #{"cur": 2}}
println(show(map_update(s, "n", (v) => (v ?? 0) + 1)))
for (k, v) in entries(m2) { print(k + "=" + show(v) + " ") }
println("")
```

### Results

| Function | Description |
|---|---|
| `unwrap(r)` | value of `Ok`; raises on `Err` (a runtime error, which `attempt` catches) |
| `unwrap_or(r, default)` | value or default |
| `unwrap_or_else(r, f)` | value or `f(error)` |
| `is_ok(r)` / `is_err(r)` | tests |
| `result_map(r, f)` | transform the `Ok` value |
| `result_map_err(r, f)` | transform the `Err` value |
| `attempt(f)` | call `f()`: `Ok(value)`, or `Err(message)` when a runtime error ended it |
| `raise(message)` | stop with a runtime error whose message is `message`, word for word (a value other than a String as `show` writes it) |

```olang
let r = Ok(10)
println(to_string(result_map(r, (v) => v * 2)))
println(to_string(unwrap_or(Err("nope"), -1)))
println(show(attempt(() => raise("widget: no kind dial"))))   // Err("widget: no kind dial")
```

### Assertions

| Function | Description |
|---|---|
| `assert(cond[, msg])` · `assert_eq(a, b[, msg])` · `assert_ne(a, b[, msg])` · `assert_true(x[, msg])` · `assert_false(x[, msg])` · `assert_close(actual, expected, tolerance[, msg])` | language-level assertions: each raises on failure, which is what fails a [`test` block](language.md#testing); the [`testing`](#testing--assertions) module holds the `Result`-returning forms |

### Evaluation helpers

| Function | Description |
|---|---|
| `set_parallel(on)` | `true` or `false`: enable or disable the parallel machinery — explicit (`par_map`) and automatic alike (how many threads: `OLANG_PARALLELISM`) |

## `str` — strings

The complete string toolkit. The module's one convention does a lot of
work: pure transformations (`trim`, `replace`, `to_upper`, ...) return
their value directly and never fail on well-formed arguments
(out-of-range positions clamp, absent substrings report `()`), while
the two genuine *parsers*
(`parse_int`, `parse_float`) return `Result`, because "this text is not
a number" is the caller's decision to make. Strings are immutable, so
every function returns a new string.

| Function | Description |
|---|---|
| `str.length(s)` | character count |
| `str.char_at(s, i)` | 1-char string, `()` past the end; a negative `i` raises |
| `str.char_code(s)` | the Unicode code point of the first character, `()` for `""` |
| `str.from_char_code(n)` | the character of code point `n`, or the string of a list of them (`[27, 91, 72]` is ESC `[` `H`); a number that is no character raises |
| `str.chars(s)` | list of characters (codepoints) |
| `str.graphemes(s)` | list of visible characters (UAX #29 clusters) |
| `str.width(s)` | display width in terminal cells (see below) |
| `str.cell_width(g)` | the cells one grapheme occupies: 0, 1 or 2; more than one grapheme raises |
| `str.substring(s, from, to)` | half-open slice by the slices' one rule: a position past the end stops at the end, `to` at or before `from` is `""`, a negative position raises (`col.slice` and `bytes.slice` alike) |
| `str.index_of(s, sub)` / `str.last_index_of` | position, or `()` when absent |
| `str.contains(s, sub)` / `str.count(s, sub)` | search |
| `str.fuzzy_score(query, text)` | a fuzzy finder's score (every query character in order; runs, word starts, the last path segment count more), or `()` |
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
| `str.is_ascii(s)` | every character ASCII? (true for `""`) — one pass over the bytes |
| `str.parse_int(s)` / `str.parse_float(s)` | `Result` parses |
| `str.fmt(template, ...)` | fill `{}` placeholders, display form; `{{`/`}}` escape |
| `str.fixed(x, digits)` | exactly `digits` decimals, never exponent form, never `-0.00` — the column form `to_string` is not |
| `str.thousands(x, digits)` | `str.fixed` with thousands separators: `1,234,567.89` |
| `str.fmt(x, spec)` | a number by a format spec, Python's mini-language: `".2f"` (`3.14`), `",.0f"` (`1,234,568`), `".1%"` (`12.3%`), `"$,.0f"` (`-$1,234`), `"e"`, `"g"`, `"n"`, `"d"`, `"x"` `"o"` `"b"` (`"X"` `"E"` `"F"` `"G"` upper case), with `[[fill]align][sign][z][#][0][width][,\|_]`: `">8.1f"`, `"02d"` (`05`: the `0` flag pads with zeros after the sign, `"+05d"` `+0005`, grouped too: `"08,d"` `0,001,234`), `"=+8.1f"`, `"#x"` (`0xff`), `"_d"`; `$` before the digits is ours. A Series of numbers gives a String Series (`ods.format`). In a template, `{:.2f}` formats its argument by the spec |
| `str.pct(x, digits = 1)` | a fraction as a percentage: `str.pct(0.1234)` is `"12.3%"` |
| `str.money(x, symbol = "$", digits = 0)` | an amount with separators: `str.money(-1234.5, digits: 2)` is `"-$1,234.50"` |

```olang
println(str.pad_start("7", 3, "0"))                 // 007
println(str.substring("olang", 1, 4))               // lan
println(str.replace("a-b-c", "-", "+"))
println(to_string(str.chars("ok")))
println(to_string(unwrap(str.parse_float("2.5")) * 2))
println(str.fmt("{} scored {} ({}%)", "ada", 99, 97.5))
println(str.fmt(0.0731, ".1%"))                       // 7.3%
println(str.fmt("{} of {:,d} ({:.1%})", "won", 12500, 0.42))   // won of 12,500 (42.0%)
println(str.money(1234567.8))                         // $1,234,568
println(str.pct(0.25, 0))                             // 25%
```

**Display width.** `str.length` counts code points and `str.graphemes`
counts what a reader sees as characters; a terminal lays text out in
*cells*, and `str.width` counts those — what padding, truncation, and
column layout must use. The unit is the grapheme cluster, and each is
0, 1, or 2 cells:

- **0** if the cluster contains a control character (C0, DEL, C1 —
  tab and newline included: expand tabs and escape controls before
  measuring);
- otherwise the width the `unicode-width` crate (0.2, Unicode 17.0
  tables) gives it: East Asian Wide and Fullwidth are 2; emoji with
  default emoji presentation, emoji presentation sequences (VS16),
  emoji modifier sequences (skin tones), and fully-qualified ZWJ
  sequences are 2; a text presentation sequence (VS15) of an emoji is
  1; combining marks, default-ignorables (ZWJ, ZWSP, soft hyphen,
  variation selectors), and Hangul medial vowels and final consonants
  are 0; everything else — East Asian Ambiguous included, as terminals
  treat it by default — is 1;
- **capped at 2**, so a flag (two regional indicators) or any other
  multi-character cluster takes the two cells a terminal gives it.

`str.width(s)` is the sum of `str.cell_width(g)` over `str.graphemes(s)`.
Both are pure and available in the browser runtime too.

```olang
println(str.width("中文 ok"))      // 7
println(str.width("e\u{301}"))     // 1
println(str.cell_width("🇯🇵"))      // 2
```

## `col` / `colx` — collections

Higher-order operations beyond the global builtins. The design principle:
the *global* builtins cover what nearly every program touches (`map`,
`filter`, `fold`, `sort`); `col` holds the next ring out — quantifiers,
partitions, key-function variants — so the global namespace stays small
while the full toolkit stays one dot away. `colx` (`use colx`)
implements the higher-order core of `col` *in olang*, embedded in the
binary: `all`, `any`, `count_by`, `drop_while`, `flat_map`,
`frequencies`, `last`, `max_by`, `min_by`, `partition`, `sort_by`,
`sum_by`, `take_while`, `unique`, `window`, and `zip_with`. It is not
the whole `col` API; the functions it shares are differential-tested
against their native twins, so the language is exercised by its own
standard library.

| Function | Description |
|---|---|
| `col.all(xs, p)` / `col.any(xs, p)` | quantifiers (short-circuit) |
| `col.index_of(xs, v)` / `col.slice(xs, from, to)` | position of a value (or Unit) / a half-open sub-list, by `str.substring`'s rule |
| `col.splice(xs, at, n, items)` | the `n` elements from `at` replaced by `items`, in one copy |
| `col.map` `col.filter` `col.fold` `col.sort` `col.contains` `col.reverse` `col.flatten` `col.zip` `col.take` `col.drop` `col.skip` `col.head` `col.tail` `col.sum` `col.min` `col.max` | the global list helpers of the same names, mirrored here: each behaves exactly as its global ([Lists](#lists), [Aggregates](#aggregates), [Higher-order functions](#higher-order-functions)) |
| `col.set(xs, i, v)` / `col.swap(xs, i, j)` | the list with element `i` replaced / elements `i` and `j` exchanged; `xs = col.set(xs, i, v)` on a list nothing else holds writes in place; an index outside the list, a negative one included, raises |
| `col.filled(n, v)` | a list of `n` copies of `v`, the preallocation primitive |
| `col.unique(xs)` | dedupe, first-seen order |
| `col.partition(xs, p)` | `([matching], [rest])` |
| `col.sum_by(xs, f)` | total of a projection |
| `col.count_by(xs, f)` / `col.frequencies(xs)` | maps of counts |
| `col.min_by(xs, f)` / `col.max_by(xs, f)` | extremes by key function |
| `col.sort_by(xs, f)` | ascending by key function, stable: elements with equal keys keep their order |
| `col.take_while(xs, p)` / `col.drop_while(xs, p)` | prefix operations |
| `col.flat_map(xs, f)` | map then flatten one level |
| `col.window(xs, n)` | sliding windows |
| `col.zip_with(a, b, f)` | combine pairwise |
| `col.last(xs)` | final element |

```olang
println(to_string(col.unique([3, 1, 3, 2, 1])))
println(to_string(col.partition([1, 2, 3, 4], (x) => x % 2 == 0)))
println(to_string(col.sum_by([{ v: 2 }, { v: 5 }], (r) => r.v)))
println(to_string(col.frequencies(["a", "b", "a"]).a))
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

What a function answers:

| Function | Answers |
|---|---|
| `floor` `ceil` `round` `trunc` | an Int: an Int argument is itself, a Float is rounded (a Float outside the Int range, or NaN, raises) |
| `round` | a half away from zero: `math.round(2.5)` is 3, `math.round(-2.5)` is -3. `str.fixed` rounds a half to even (`str.fixed(2.5, 0)` is `"2"`), the way printed decimals usually are |
| `pow` | an Int for two Ints (an overflow raises, as `*` does; a negative exponent raises — pass a Float), else a Float |
| `sign` | an Int, -1, 0 or 1, for an Int or a Float |
| `abs` | the kind it is given |
| `min` `max` | as the global `min` and `max`, for numbers: two or more numbers or one list, the first of equal values (`math.min(1, 2.5)` is 1) |
| `fract` and the rest | a Float |

`mathx` (`use mathx`) is an olang-source module covering the integer and
rounding core (its rounding answers an Int, and its `min` and `max`
take two numbers): `abs`, `sign`, `min`, `max`, `sqrt`, `floor`, `ceil`,
`round`, `trunc`, `fract`, `gcd`, `lcm`, `factorial`, `radians`,
`degrees`, and the constants `PI`, `E`, and `TAU`. It is smaller in
scope than `math`, and differential-tested against it.

## `vec` — vectors over lists

The kernels numeric code otherwise spells as index loops, over lists of
Int or Float: `vec.dot(a, b)`, `vec.add(a, b)`, `vec.sub(a, b)`,
`vec.scale(a, k)`, `vec.sum(a)`, `vec.norm(a)`, `vec.mean(a)`. Results
are Float (or Float lists), but `vec.sum` of a list of Ints is an Int
(0 for an empty list), as the global `sum` is; lengths must agree; a non-finite result
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

| Function | Description |
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

## `asset` — a package's files

The files a package ships beside its code — fonts, images, data — read
by the package's name: from its directory when the program runs from
source, and from the application itself when it was built with `olang
build --app` (the package lists them under `assets` in `olang.toml`).
A path stays inside its package.

| Function | Description |
|---|---|
| `asset.read(package, path)` | `Ok(Bytes)`, or `Err` naming the package or file it could not find |
| `asset.exists(package, path)` | whether the package carries the file |
| `asset.path(package, path)` | where the file is (the application's own copy has the path it was built from) |

```olang no-run
let font = unwrap(asset.read("loom", "fonts/Inter.ttf.br"))
```

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

**Each submodule's functions.** A write answers the new handle, to be
rebound; a read answers a value.

| Submodule | Functions |
|---|---|
| `heap` | `new()` · `size(h)` · `is_empty(h)` · `top_prio(h)` / `top_item(h)` — the smallest priority and its item · `push(h, prio, item)` · `pop(h)` — without its smallest pair · `from_lists(prios, items)` — built in O(n) from parallel lists |
| `deque` | `new()` · `size(q)` · `is_empty(q)` · `front(q)` / `back(q)` · `push_back(q, x)` / `push_front(q, x)` · `pop_front(q)` / `pop_back(q)` · `to_list(q)` — front to back |
| `table` | `new()` · `size(t)` · `has(t, k)` · `get(t, k)` — the value or `()` (a stored `()` reads as absent; `has` answers presence) · `get_or(t, k, fallback)` · `put(t, k, v)` · `remove(t, k)` — a no-op when absent · `keys(t)` / `values(t)` — in slot order, stable between writes |
| `dsu` | `new(n)` — `n` elements, each its own group · `size(d)` — the element count · `find(d, x)` — the group's representative · `connected(d, a, b)` · `union(d, a, b)` · `groups(d)` — the number of groups |
| `bitset` | `new(n)` — members `0..n-1` · `capacity(b)` · `has(b, i)` · `add(b, i)` · `remove(b, i)` · `count(b)` · `union(a, b)` / `intersect(a, b)` / `difference(a, b)` — of equal capacity · `to_list(b)` — ascending; a position outside the capacity raises |
| `alg` | `sort(xs)` — stable merge sort · `sort_by_key(xs, key_fn)` · `lower_bound(xs, x)` / `upper_bound(xs, x)` — insertion points in a sorted list · `bin_search(xs, x)` — the index or `()` · `select_kth(xs, k)` — the k-th smallest, 0-based · `graph(n, edges)` from `[[u, v], …]` and `wgraph(n, edges)` from `[[u, v, w], …]` — flat adjacency handles · `bfs(g, src)` — hop distances, -1 unreachable · `topo_sort(g)` — `Ok(order)` or `Err("cycle")` · `dijkstra(g, src)` — distances over a `wgraph`, -1 unreachable |

**Empty structures.** A read of an element that is not there answers
`()`: `heap.top_prio`, `heap.top_item`, `deque.front` and `deque.back`
on an empty structure. A removal from an empty structure (`heap.pop`,
`deque.pop_front`, `deque.pop_back`) answers the handle unchanged, so a
drain loop may test `is_empty` or the read, whichever it holds.

**The representation discipline.** A handle is to be used only through
its submodule's functions: its layout is internal, outside the
[compatibility contract](stability.md), and may change in any release.
Every handle is one flat list with a
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
push-and-pop ~36µs a pair. Two limits apply. For *plain* sorting of a
list, the native `sort` builtin remains hundreds of times faster than
`alg.sort`; reach for `alg` when you need stability, keys, the bisect
family, or selection. And structure operations run on the interpreter
by design (the tier boundary would convert a whole handle
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
| `json.stringify(v)` | `Result` of compact JSON text; an object's keys in its order (a map's as they were first set) |
| `json.stringify_pairs(pairs)` | `Result` of one JSON object whose keys are in the order of `pairs`, a list of `(key, value)`, a key repeated written twice (each value as `stringify` writes it) |
| `json.prettify(s)` / `json.minify(s)` | reformat JSON text |
| `json.is_valid(s)` | is the text valid JSON? |
| `json.get(s, key)` / `json.set(s, key, v)` | read or replace one top-level key of JSON text: `Result` of the value / of the new text |
| `json.has_key(s, k)` / `json.remove(s, k)` | text-level operations |
| `json.get_keys` `json.get_values` `json.get_type` | text-level inspection |
| `json.array_get` `json.array_length` `json.array_push` | text-level arrays |
| `json.merge(a, b)` / `json.deep_clone(s)` | combine / copy |

```olang
let doc = unwrap(json.parse("{\"name\": \"ada\", \"tags\": [\"x\", \"y\"], \"age\": 36}"))
println(`${doc.name} has ${len(doc.tags)} tags`)
let field = "age"
println(to_string(map_get(doc, field)))          // a computed key
let out = unwrap(json.stringify({ ok: true, n: 1 }))
println(out)                                     // {"ok":true,"n":1}
let row = unwrap(json.stringify_pairs([("id", 7), ("name", "ada"), ("tags", ["x"])]))
println(row)                                     // {"id":7,"name":"ada","tags":["x"]}
```

## `toml` — TOML

The config format olang's own `olang.toml` manifests use, with `json`'s
core surface: `parse`, `stringify`, `is_valid`. Both modules bridge
values through the same conversions, so a table parses to the same Map
shapes JSON objects do — tables become Maps, arrays become Lists,
datetimes become their string rendering. A TOML document is a table at
the top level, so `stringify` accepts a Map (or struct-like value) and
returns `Err` for anything else.

```olang
let text = "name = \"cfg\"\n[server]\nport = 7317"
let doc = unwrap(toml.parse(text))
println(show(doc.name) + " on " + show(doc.server.port))

let out = unwrap(toml.stringify(#{ "retries": 3 }))
println(str.trim(out))
println(to_string(toml.is_valid("not [ valid")))
```

## `csv` — CSV

Quoted fields, embedded commas, and header-aware parsing.
`parse_with_headers` yields rows as maps keyed by the header (`#{ "name":
"Ada" }`, read `row.name` or `map_get(row, "first name")`; all cells
are strings — convert explicitly).

| Function | Description |
|---|---|
| `csv.parse(s)` | rows as lists of strings |
| `csv.parse_with_headers(s)` | rows as maps keyed by the header |
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

The browser runtime the web SDK ships omits this module (the engine and
its Unicode tables are a fifth of the runtime's code); a page that calls
`re.*` hears so. The website's playground carries it.

| Function | Description |
|---|---|
| `re.is_match(pat, s)` | boolean test |
| `re.find(pat, s)` | first match text, `Ok(())` when nothing matches |
| `re.find_all(pat, s)` | all match texts |
| `re.find_spans(pat, s)` | where every match is: `(start, end)` character offsets |
| `re.captures(pat, s)` | full match + capture groups (`()` for a group that took no part), `Ok(())` when nothing matches |
| `re.replace(pat, s, rep)` | every match replaced, as `str.replace` replaces every occurrence; `$1` and `${name}` in `rep` name a capture |
| `re.replace_first(pat, s, rep)` | the first match replaced |
| `re.split(pat, s)` | split by pattern |
| `re.is_valid(pat)` | is the pattern itself well-formed? |

```olang
println(to_string(unwrap(re.find_all("\\d+", "a1 b22 c333"))))
println(to_string(unwrap(re.captures("(\\w+)@(\\w+)", "ada@lovelace"))))
println(unwrap(re.replace("\\s+", "too   many spaces", " ")))
```

## `dates` — dates and times

A moment is written in RFC 3339 with its offset —
`2026-01-02T03:04:05+00:00`, fractional seconds only when the moment
has them — by `now`, `utc_now`, `datetime`, `parse_datetime` and
`from_timestamp`, and every one of them reads back what the others
write (and `2026-01-02 03:04:05`; text without an offset is UTC). A day
is `2026-01-02`. Fallible operations return `Result`.

The rule for kinds: **what reads the clock answers text** (`now`,
`utc_now`, `today`, `stamp`, `stamp_ms`: Strings, to store and print),
**what builds or parses a day answers a `Date`** (`date`, `parse`,
`from_epoch_day` — each inside a `Result`, since `date(2026, 2, 30)`,
`parse("soon")` and a day past the calendar's range are no day; and
`epoch_day`'s Int likewise, for text that names no day), and a function that takes a day takes either and
answers in kind. `dates.parse` answers the day a text names, as written:
a time in it is dropped (`dates.parse("2026-01-02T23:30:00-05:00")` is
2026-01-02); keep the text, or `timestamp` it, when the time matters.

| Group | Functions |
|---|---|
| Now | `now` `utc_now` `today` · `stamp` — the storage form: UTC, second precision, `Z` suffix (`2026-08-31T23:40:06Z`), sortable as text · `stamp_ms` — the same at millisecond precision (`…06.503Z`), the one to key a row's version on: two edits inside one second are identical under `stamp` |
| Build | `date(y, m, d)` → `Result` of a `Date` · `datetime(y, m, d, h, mi, s)` → a UTC moment's text · `time(h, mi, s)` |
| Parse/format | `parse(s)` → `Result` of a `Date` · `parse_date` `parse_datetime` `parse_time` · `format_date(d, fmt)` `format_datetime(dt, fmt)` `format_time(t, fmt)` — `fmt` in strftime's `%` form (`"%B %d, %Y"`); a specifier it does not know, a trailing `%`, or a field the value lacks (`%H` of a date) is an `Err` |
| Fields | `year` `month` `day` `hour` `minute` `second` (a moment's own, in its offset) · `weekday` — 0 Sunday, 1 Monday, … 6 Saturday · `iso_weekday` — 1 Monday … 7 Sunday, what a Date column's `weekday()` answers |
| Arithmetic | `add_days` `add_weeks` `add_months` `add_years` · `diff_days(a, b)` — `a` minus `b` in days, positive when `a` is later (`d2 - d1` the same) |
| Calendar | `start_of_week(d)` — the Monday on or before `d` · `start_of_month(d)` — its first day |
| Day numbers | `epoch_day(d)` → `Result` of the Int days since 1970-01-01 (a stamp's day is the one it is written in; `Err` for text that is no day) · `from_epoch_day(n)` → `Result` of a `Date` (`Err` past the calendar's range) |
| Epoch | `timestamp(dt)` — Unix milliseconds, the unit of every other clock (`time.now_ms`, `modified_ms`); an offset is honoured, a bare day is its midnight UTC · `timestamp_s(dt)` — Unix seconds · `from_timestamp(ms)` / `from_timestamp_s(s)` → the UTC moment's text |
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

`unwrap(dates.timestamp(dates.now()))` is "milliseconds since the
epoch, now" (`timestamp` parses its argument, so it returns a
`Result`; for a plain millisecond clock, `time.now_ms()` is simpler).
A stamp's offset is part of it: `…T16:30:00+02:00` and `…T14:30:00Z`
are the same second.

A **day number** is a day as a coordinate — what a calendar or a
timeline lays out, pans, and snaps to. `epoch_day` and
`from_epoch_day` convert, and the week and the month have their edges:

```olang
let d = unwrap(dates.epoch_day("2026-10-08"))    // an Int
let week = unwrap(dates.epoch_day(unwrap(dates.start_of_week("2026-10-08"))))
println(`${d - week} days into the week`)        // 3 (weeks start on Monday)
println(unwrap(dates.from_epoch_day(d + 30)))    // 2026-11-07, a Date
println(unwrap(dates.start_of_month("2026-10-08")))  // 2026-10-01
```

## `time` — clocks and sleeping

`dates` is calendars; `time` is measurement and pacing.

| Function | Description |
|---|---|
| `time.now_ms()` | milliseconds since the Unix epoch |
| `time.monotonic_ms()` | monotonic milliseconds (never goes backwards) — the clock for durations. Natively it reads from the process's start (as the kernel recorded it), so a program can time its own start |
| `time.monotonic()` | the same clock as a Float with its fraction, for a duration under a millisecond; in the browser both read the page's `performance.now()`, so a reading and a stamp the page made (a `dom.request` reply's timings) are one clock |
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
| `task.join(t)` | `Ok(value)` when the task returned (a value that is itself an `Err` included), `Err(message)` when it raised — blocks until it finishes |
| `task.join_timeout(t, ms)` / `task.join_timeout(t, #{ timeout_ms })` | `Ok` of what `task.join` answers (`Ok(Ok(v))`, `Ok(Err(e))`) if it finished within `ms`, else `Err("timed out")` |
| `task.watch(t, c)` | the channel dies with the task: when `t` ends, however it ends, `c` is closed, so a `chan.recv` on it returns `Err` instead of waiting forever. Ownership is declared here, not inferred — any task may hold either end of a channel |
| `task.list()` | every task a live handle still watches: `#{ id, state, elapsed_ms }` |
| `task.parked()` | threads blocked on unbounded waits: `#{ thread, on, waited_ms }` |

`task.join` always answers a `Result`, so a task that raised
(`Err(message)`) is told from one that returned an `Err` (`Ok(Err(…))`),
and one bad worker is a value to handle rather than a crash.
`task.join_timeout` wraps that once more, because "we stopped waiting"
(`Err("timed out")`) is a third thing.

`task.list` reports each watched task's state — `"running"`,
`"joining"` (a thread is mid-collect), or `"done"` (finished, its
memoized result still collectible). `task.parked` is the live view
behind [the stall detector](#the-stall-detector)'s report: every thread
currently blocked in an unbounded wait, and what it is waiting on.

```olang
fn work(n) = { time.sleep(10); n * n }

let jobs = [spawn work(2), spawn work(3), spawn work(4)]
println(to_string(jobs |> map((j) => unwrap(task.join(j)))))   // [4, 9, 16]
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
| `cell.take(c)` | the value, moved out: the cell holds `()` until set again. Nothing else holds what it answers, so extending it is in place — `cell.set(c, map_set(cell.take(c), k, v))` grows a table in time proportional to its size, where `map_set(cell.get(c), k, v)` copies it on every insert |

None of these return `Result`: there is no expected failure to report.
They raise on misuse — reading a cell from a thread that does not own
it, or touching a cell from inside its own `cell.update`.

```olang
let seen = cell(#{})
fn remember(word) = {
    let counts = cell.get(seen)
    let n = map_get_or(counts, word, 0)
    cell.set(seen, map_set(counts, word, n + 1))
    n + 1
}
for w in ["ok", "err", "ok", "ok"] { remember(w) }
println(to_string(cell.get(seen).ok))   // 3
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
| `chan.try_recv(c)` | `Ok(value)`; `Ok(())` when nothing is waiting; `Err("channel is closed")` once closed and drained. A channel that carries `()` cannot tell it from empty this way: use `chan.recv_timeout` |
| `chan.recv_timeout(c, ms)` / `chan.recv_timeout(c, #{ timeout_ms })` | like `recv`, plus `Err("timed out")` once `ms` pass — receivers on one channel wait side by side, so the bound is this wait's own, never another waiter's |
| `chan.ask(service, request)` | the reply pattern in one call: sends `#{ "req": request, "reply": r }` on a fresh reply channel and answers `Ok(reply)` — spinning briefly before it parks, so a service that answers in microseconds costs no thread wake-up |
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
println(`${total} from ${unwrap(task.join(worker))} squares`)
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
| `random.randstr(n)` | a random string of `n` letters and digits |
| `random.randstr_alpha(n)` / `random.randstr_alnum(n)` / `random.randstr_numeric(n)` | a random string of `n` letters / letters and digits / digits |

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

### Generators: `random.rng`

`random.rng(seed)` is a generator of its own (PCG64), independent of the
global stream: drawing from it never moves `random.random()`, and the
global `random.seed` never moves it. Its draws with a count are Series,
made in one native pass (in parallel blocks past a million values, the
values the same on any number of threads); without a count, one value.
`random.rng()` with no seed takes one from the global stream.

Its methods are the functions of `random.Rng`: `g.normal(100)` is
`random.Rng.normal(g, 100)`.

| Method of a generator `g` | Description |
|---|---|
| `normal(n?, mean = 0.0, std = 1.0)` | normals |
| `uniform(n?, low = 0.0, high = 1.0)` | uniforms in `[low, high)` |
| `integers(n?, low, high)` | integers in `[low, high)` (`high` excluded, as `range`) |
| `poisson(n?, lam = 1.0)` | Poisson counts |
| `lognormal(n?, mean = 0.0, sigma = 1.0)` | `exp` of normals with this mean and sd |
| `binomial(n?, trials, p)` | binomial counts |
| `choice(values, n?, weights?, replace = true)` | picks from a List or Series, by `weights` when given; `replace: false` draws distinct positions (by weight, Efraimidis–Spirakis). A Series back when `values` is one or a list a Series can hold |
| `shuffle(values)` | the values in a random order (a List stays a List) |
| `permutation(n)` | `0..n` in a random order, an Int Series |
| `spawn(k)` | `k` independent child generators, on streams of their own: hand one to each `par_map` task and the results are the same on every run |

The data stack's random verbs take a generator where they take a seed:
`ods.sample(f, n, rng: g)`, `learn.split(f, test: 0.25, rng: g)`,
`stats.norm.sample(n, rng: g)`, `stats.bootstrap(s, rng: g)`. An Int is
accepted as `rng:` too, for a fresh generator of that seed.

```olang
let g = random.rng(42)
let heights = g.normal(1000, 170.0, 8.0)
println(heights.len())                                          // 1000
let tiers = g.choice(["free", "pro", "team"], 5000, weights: [0.7, 0.2, 0.1])
println(tiers.value_counts().n_rows())                          // 3
let orders = g.poisson(5000, 3.0)
println(orders.min() >= 0)                                      // true
let streams = random.rng(7).spawn(4)
let means = par_map(streams, (r) => r.normal(10000).mean())
println(means == par_map(random.rng(7).spawn(4), (r) => r.normal(10000).mean()))   // true
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
| Asymmetric | `generate_key_pair` `encrypt_rsa` `decrypt_rsa` `sign_data` `verify_signature` `export_public_key` `import_public_key` — not in the browser runtime the web SDK ships, which says so when one is called |
| Utilities | `random_bytes` `random_hex` `hex_encode` `hex_decode` `secure_compare` |

```olang
println(str.substring(crypto.sha256("olang"), 0, 16))
println(to_string(crypto.secure_compare("abc", "abc")))
```

`crypto.random_hex(n)` answers `n` random bytes as `2n` hexadecimal
characters (at most 1,024 bytes a call), and `crypto.random_bytes(n)` the
same bytes as `Bytes`. Both draw from a cryptographically
secure generator — ChaCha12, seeded from the operating system's entropy
source and reseeded from it periodically; in the browser the seed comes
from the page's `getRandomValues` — so they are the functions to mint a
session secret, a share token, or a nonce. `random`, the module, is the
seedable generator for simulations and is not.

`crypto.hmac_sha256(key, message)` answers the HMAC as a hexadecimal
string; compare a received MAC against it with `secure_compare`, never
`==`.

```olang no-run
let token = crypto.random_hex(32)                       // 64 hex characters
let mac = crypto.hmac_sha256(secret, payload)
let genuine = crypto.secure_compare(mac, received_mac)
```

`crypto.encrypt_aes(plaintext, key)` is AES-256-GCM. The key is 32 bytes
written as 64 hexadecimal characters (`crypto.random_hex(32)` makes one;
`derive_key` makes one from a password). The answer is `Ok` of one
hexadecimal string: the 12-byte nonce (24 characters), then the
ciphertext, then the 16-byte authentication tag — so a ciphertext is 56
hexadecimal characters longer than twice its plaintext's bytes, and a
fresh random nonce makes every encryption of the same text different.
`crypto.decrypt_aes(data, key)` takes that string whole, or `(ciphertext,
key, nonce)` with the nonce apart (both hexadecimal); a wrong key or a
changed byte is an `Err`, never a wrong plaintext. The plaintext is text
(UTF-8) both ways.

## `bytes` — binary data

Strings are UTF-8 text; `Bytes` (new in 0.69) is the value for everything
that isn't — an image, an archive, a hash's raw output, a non-UTF-8
file. A `Bytes` value is immutable, `typeof` says `Bytes`, `len(b)` is
the byte count, `b[i]` is the byte at `i` as an Int (a negative `i`
raises, as a list's does), equality is structural, and it displays as a capped hex
preview. `fs.read_bytes`/`fs.write_bytes` move it to and from disk;
`base64` and the `crypto` hashes accept it.

| Function | Description |
|---|---|
| `bytes.from_list(ints)` / `bytes.to_list(b)` | to and from a list of Ints 0..=255 |
| `bytes.from_string(s)` | a string's UTF-8 bytes |
| `bytes.to_string(b)` | decode as UTF-8 — `Result`, since bytes may not be text |
| `bytes.len(b)` | byte count (the global `len` works too) |
| `bytes.slice(b, from, to)` | half-open, by `str.substring`'s rule: past the end stops at the end, a negative position raises |
| `bytes.concat(a, b)` | concatenation |
| `bytes.to_hex(b)` / `bytes.from_hex(s)` | lowercase hex, two digits a byte; `from_hex` reads either case and answers a `Result` (an odd count or a character that is not a digit is `Err`) |

```olang
let b = bytes.from_list([104, 105, 33])
println(`${len(b)} bytes, first ${b[0]}, last ${last(bytes.to_list(b))}`)
println(unwrap(bytes.to_string(b)))                    // hi!
println(show(bytes.to_list(bytes.slice(b, 0, 2))))     // [104, 105]
println(base64.encode(b))                              // aGkh
println(show(bytes.from_string("hi!") == b))           // true
```

## `compress` — gzip and deflate

In-memory compression for the wire and the disk, on every target: the
codec is pure Rust, so the browser runtime carries it too. Every
function takes a string or `Bytes` and answers `Bytes`; the decoders
return `Result`, since arbitrary input may not be compressed data.

| Function | Description |
|---|---|
| `compress.gzip(data)` | gzip at the default level (6) |
| `compress.gzip_level(data, n)` | gzip at level `n`, 0..=9 |
| `compress.brotli(data)` | brotli at quality 5 — what `Content-Encoding: br` carries; a quarter of the bytes for a wasm |
| `compress.gunzip(b)` | `Ok(Bytes)` or `Err(message)` |
| `compress.deflate(data)` / `compress.inflate(b)` | raw deflate, no gzip header, and its inverse |

An `http.serve` handler that compresses a large JSON body sets
`Content-Encoding` itself; the web SDK's `serve` does this for every
text response of a kilobyte or more when the client accepts gzip (its
`compress` option turns it off).

```olang
let body = str.repeat("- [ ] item\n", 500)
let packed = compress.gzip(body)
println(`${str.length(body)} -> ${len(packed)} bytes`)
println(show(unwrap(bytes.to_string(unwrap(compress.gunzip(packed)))) == body))   // true
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
| `base64.is_valid(s)` | checks |

```olang
let enc = base64.encode("olang")
println(enc + " -> " + unwrap(base64.decode(enc)))
println(show(len(unwrap(base64.decode_bytes(enc)))))   // 5
```

## `fs` — file system

Whole-value I/O: `read_file` returns the entire contents as one string,
and `write_file` replaces them. There are no file handles and no
open/close lifecycle to manage, which fits a language whose programs
transform values. Every function that touches the disk and can fail
returns `Result`, because the file system is the classic source of
failures that are the *caller's* business (missing file, permissions)
rather than bugs. The queries `exists`, `is_file`, and `is_dir` answer
a `Bool`, and the path functions `join`, `dirname`, `basename`, and
`ext` answer a `String`. (Examples are `no-run`: they touch the disk.)

| Group | Functions |
|---|---|
| Files | `read_file` `write_file` `append_file` `copy_file` `move_file` `remove_file` `trash` — and `read_bytes` / `write_bytes` for [binary data](#bytes--binary-data) |
| Directories | `create_dir` `create_dir_all` `list_dir` `walk` `glob` `remove_dir` `remove_dir_all` |
| Queries | `exists` `is_file` `is_dir` `file_size` `file_info` |
| Watching | `watch` `watch_next` `watch_close`: a folder's whole tree followed as it changes |
| Paths | `join(parts)` `dirname` `basename` `ext`: pure string surgery, answering the string itself. `abs_path` answers a **`Result`** (`Ok(path)`): it resolves against the current directory, which can fail, and normalizes `.`/`..` without requiring the file to exist. `real_path` (a `Result` too) also resolves symbolic links (`/tmp/x` is `/private/tmp/x` on macOS); for a path that does not exist yet it makes the nearest existing folder real and keeps the rest |

`fs.trash(path)` moves a file or a folder to the Trash as the Finder's
Move to Trash does (macOS's NSFileManager) — never a permanent delete —
and answers `Ok(path in the Trash)`; moving it back from there with
`fs.move_file` puts it back. Elsewhere it is an `Err` that says so.

`fs.file_size(path)` answers a file's bytes and `fs.file_info(path)` an
object — `{ size, is_file, is_dir, readonly, modified_ms }`: `size`, `is_file`, `is_dir`, `readonly`, and `modified_ms`
(when the file last changed, in epoch milliseconds, the clock
`time.now_ms()` reads) — both from the file's metadata, without reading
it: what a cache or a change check asks.

`fs.join(["logs", name + ".txt"])` joins segments with the platform
separator (an absolute segment restarts the path, standard join
semantics); `fs.dirname`/`fs.basename`/`fs.ext` decompose without
touching the disk. `fs.walk(dir)` lists every file below a directory
(recursive, sorted);
`fs.glob(pattern)` filters by a pattern where `*` matches within a path
segment, `?` one character, and `**` any number of segments. `fs.scan(root, opts)` lists a
checkout's files as a person sees them — every `.gitignore` respected
at every level, hidden entries and the folders named in `skip` left
out, `exts` the extensions kept — relative to `root` and sorted,
walked on worker threads (an editor lists a workspace of thousands of
files with it). `fs.search(root, query, opts)` searches those files (or
`opts.files`) for a literal or, with `regex: true`, a regular
expression — `case` and `word` as a find bar has them — in parallel,
answering `Ok` of `#{ path, line, column, end_column, text }` by path
then line (1-based, in characters, `end_column` just past the match).
With `replace` (a template) each match also
says `with`, what it becomes — a regular expression's groups expanded
(`$1`, `${name}`, `$$` a dollar) — for a replace across files to
preview and apply. An option either function does not know raises.

| `fs.scan` option | Meaning (default) |
|---|---|
| `skip` | folder names left out wherever they are (`[]`) |
| `exts` | the extensions kept, without the dot (every file) |
| `hidden` | `true` keeps entries whose name starts with `.` (`false`) |
| `gitignore` | `false` ignores the `.gitignore`s (`true`) |
| `limit` | at most this many files (100,000) |
| `max_depth` | folders deeper than this are not walked (no limit) |
| `within` | only these folders (relative to `root`) and all below them, by the whole walk's rules: a parent's `.gitignore`, the skipped and hidden folders on the way (what a watch's changed folders are read again with) |
| `own` | only these folders' own files, as `within` (`""` the root's) |
| `empty_dirs` | `true` answers the empty folders instead of the files: those the walk enters holding nothing it visits (a hidden or ignored entry does not count; with `max_depth`, none at the limit); `"also"` answers both from one walk, `Ok(#{ files, empty_dirs })` |

| `fs.search` option | Meaning (default) |
|---|---|
| `regex` | the query is a regular expression (`false`: a literal) |
| `case` | match case (`false`) |
| `word` | whole words only (`false`) |
| `limit` | at most this many matches (2,000) |
| `per_file` | at most this many matches a file (200) |
| `files` | search these paths (relative to `root`) instead of walking |
| `replace` | a template each match also answers as `with` |
| `skip` `exts` `hidden` `gitignore` `max_depth` `within` `own` | as `fs.scan`'s, for the walk |

`fs.watch(root, opts)` follows a folder's whole tree as it changes — a
file made in the Finder, a `git checkout`, a build's output — with the
operating system's own notices (FSEvents on macOS: one stream for the
tree, on a thread of its own, nothing polled; elsewhere it answers an
`Err`, and a caller keeps polling). It answers `Ok(Watch)`, a handle a
reader task holds. `fs.watch_next(w, opts)` waits up to `timeout_ms` for
a change, then keeps taking more while they come within `settle_ms`
(never past a second), so a burst of thousands of writes is one batch:
`Ok(#{ root, events, rescan })`, each path once as `#{ path, kind, dir }`
— `kind` is `created`, `removed` or `modified` as the disk stands when
the batch is made (a rename is its old path removed and its new one
created), `dir` whether it is a folder, `path` absolute and spelled as
`root` was. When the system dropped notices, or a batch would hold more
than `max` paths, `events` is empty and `rescan` names the folders to
read again. `Ok(())` when nothing changed in time; `Err` once closed.
`fs.watch_close(w)` stops it (a `watch_next` waiting answers `Err`). A
path under a folder named in `ignore` never leaves the watching thread,
so a build writing into `target/` costs nothing downstream; `keep` lets
chosen paths under an ignored folder through (a repository's
`.git/HEAD`, `.git/index`, `.git/refs`, at any depth).

| `fs.watch` option | Meaning (default) |
|---|---|
| `ignore` | folder names whose contents are never said, wherever they are (`[]`) |
| `keep` | paths from an ignored folder's name down said all the same: `.git/HEAD` (`[]`) |
| `latency_ms` | how long the system gathers notices before handing them over (50) |

| `fs.watch_next` option | Meaning (default) |
|---|---|
| `timeout_ms` | how long to wait for the first change (1,000) |
| `settle_ms` | a burst ends after this long with nothing new (50) |
| `max` | at most this many paths a batch; past it, `rescan` the root (5,000) |

```olang no-run
let w = unwrap(fs.watch("project", #{ "ignore": [".git", "target"], "keep": [".git/HEAD"] }))
match fs.watch_next(w, #{ "timeout_ms": 2000 }) {
    Ok(b) => if b != () => { for e in b.events { println(e.kind + " " + e.path) } },
    Err(e) => println("closed")
}
fs.watch_close(w)
```

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
| `meta.rename(source, names)` | `Ok(text)`: `source` with each name that is a key of the map `names` renamed to its value wherever it binds or refers to a value — declarations, parameters, pattern binders, uses, template interpolations — and kept where it names something else: a field after `.`, an object's, struct literal's or named argument's key, a struct pattern's field, a type, a variant, a trait's or impl's method, an import. Comments and strings are untouched; `Err` when the source does not parse |
| `meta.eval(source, options?)` | evaluate source; `options` bounds untrusted code — `#{ "max_steps": n }` (loop iterations and calls, deterministic) and `#{ "timeout_ms": ms }` (wall clock) — returning `Err("budget exceeded …")` instead of a hung thread |
| `meta.expand(source)` | `Ok(text)` \| `Err(message)` — the program after macro expansion, as source: every `meta fn` removed and every `@` site replaced by what it generated (what `olang expand FILE` prints) |
| `meta.encode(source, options?)` | `Ok(bytes)` \| `Err(message)` — the parsed program (macros expanded) as a program image: bytes the runtime loads without parsing, behind a header naming the olang version that wrote it, which is the only version that loads it. `#{ "hot": [names] }` names the functions the loading runtime compiles at declaration rather than at their first call. What the web SDK's `serve` hands the browser in place of the source bundle |
| `meta.unresolved(source)` | `Ok(list)` of every name the program uses that nothing in it defines or imports and the language does not provide, in order of first use; `Err` when the source does not parse. How the web SDK's bundler warns of a module that uses a name it does not import, undefined in the browser as natively |
| `meta.exports(path)` | at expansion time, `Ok(map)` of the literal values the module the expanding file imports as `path` binds at its top level (`let`/`share let` of a number, string, list, map, or tuple literal), by name; `Err` for a path the file does not import, and at runtime |

Nodes are discriminated-union maps. Top-level statements carry `line` and
`column`; expressions nest (a `call`'s `callee` and `args` are themselves
node maps; `target` is the dotted callee, like `fs.read_file`). The table
lists every `kind` and the fields a node of it has; a pattern node is
`pattern/<form>` and a type node `type/<form>` (their `kind` is `pattern`
or `type`, and the rest is in `form`). A field shown here may be absent
when the source has no such part: an `if` with no `else`, a `break` or
`return` with no value, an arm with no `guard`, an `index_assign` of a
plain list (`field` is there for `m.k[i] = v`). A parameter, a struct's
field, and a pattern's field are plain maps without a `kind`. Kinds and
fields are only ever added; `tests/meta_parse_nodes_test.rs` holds this
table to what `meta.parse` produces.

<!-- meta.parse nodes -->
| Kind | Fields |
|---|---|
| `arm` | `body`, `guard`, `pattern` |
| `assert` | `value` |
| `assert_eq` | `left`, `right` |
| `assert_false` | `value` |
| `assert_ne` | `left`, `right` |
| `assert_true` | `value` |
| `assign` | `target`, `value` |
| `binop` | `left`, `op`, `right` |
| `block` | `body` |
| `bool` | `value` |
| `break` | `value` |
| `call` | `args`, `callee`, `target` |
| `continue` | — |
| `decorated_type` | `column`, `decl`, `decorators`, `line` |
| `entry` | `key`, `value` |
| `err` | `value` |
| `error` | `column`, `line`, `name` |
| `expr` | `column`, `line`, `value` |
| `field` | `field`, `object` |
| `field_value` | `name`, `value` |
| `float` | `value` |
| `fn` | `body`, `column`, `line`, `name`, `parameters`, `params`, `return_type`, `shared`, `type_params` |
| `for` | `body`, `iterable`, `var` |
| `ident` | `name` |
| `if` | `condition`, `else`, `then` |
| `impl` | `column`, `line`, `trait`, `type` |
| `index` | `index`, `object` |
| `index_assign` | `field`, `index`, `target`, `value` |
| `int` | `value` |
| `lambda` | `body`, `parameters`, `params` |
| `let` | `column`, `line`, `mutable`, `name`, `pattern`, `shared`, `type`, `value` |
| `list` | `items` |
| `loop` | `body` |
| `macro_call` | `args`, `name` |
| `map` | `entries`, `entry_count` |
| `match` | `arm_count`, `arms`, `value` |
| `meta_fn` | `body`, `column`, `line`, `name`, `parameters`, `params`, `return_type`, `shared`, `type_params` |
| `named_arg` | `name`, `value` |
| `object` | `fields` |
| `ok` | `value` |
| `par_for` | `body`, `iterable`, `var` |
| `pattern/enum` | `patterns`, `text`, `variant` |
| `pattern/err` | `inner`, `text` |
| `pattern/ident` | `name`, `text` |
| `pattern/list` | `items`, `rest`, `text` |
| `pattern/literal` | `text`, `value` |
| `pattern/ok` | `inner`, `text` |
| `pattern/or` | `alternatives`, `text` |
| `pattern/range` | `end`, `inclusive`, `start`, `text` |
| `pattern/record` | `fields`, `text` |
| `pattern/struct` | `fields`, `text`, `type` |
| `pattern/tuple` | `items`, `text` |
| `pattern/wildcard` | `text` |
| `pipeline` | `left`, `right` |
| `range` | `end`, `inclusive`, `start` |
| `return` | `value` |
| `spawn` | `value` |
| `spread` | `value` |
| `spread_literal` | `of`, `parts` |
| `str` | `value` |
| `struct` | `fields`, `type` |
| `template` | `parts` |
| `test` | `body`, `column`, `line`, `name` |
| `trait` | `column`, `line`, `name` |
| `try` | `value` |
| `tuple` | `items` |
| `type` | `column`, `definition`, `fields`, `line`, `name`, `type`, `type_params`, `variants` |
| `type/basic` | `name`, `text` |
| `type/function` | `params`, `returns`, `text` |
| `type/generic` | `args`, `base`, `text` |
| `type/list` | `element`, `text` |
| `type/literal` | `text`, `value` |
| `type/named` | `name`, `text` |
| `type/record` | `fields`, `text` |
| `type/tuple` | `text`, `types` |
| `type/union` | `text`, `types` |
| `unary` | `op`, `operand` |
| `use` | `column`, `items`, `line`, `path`, `shared` |
| `while` | `body`, `condition` |
<!-- /meta.parse nodes -->

`spread_literal` is a map or object literal with spreads (`#{ ...m, "k":
v }`): `of` is `map` or `object`, and `parts` the written runs (literal
nodes of that kind) and the spreads (`spread` nodes), in source order.
`index_assign` is `xs[i] = v` or `m.k[i] = v`.

```olang
// A dependency lister — a file's imports — in four lines over meta.parse.
let program = unwrap(meta.parse("use geometry { area }\nuse fmt\nfn f() = 1"))
for node in program |> filter((n) => n.kind == "use") {
    println(node.path + " " + show(node.items))
}
// geometry ["area"]
// fmt ["*"]
```

Every sub-expression is emitted — a `match`'s `arms`, a map's `entries`, a
struct's `fields`, a template's `parts`, and the `spawn`/`assert` interiors
are all walkable node maps — so a tool that filters the node tree cannot
silently miss a call hidden in a subtree. Patterns and type annotations
are nodes too. An arm's `pattern` (and a `let`'s) is `kind: "pattern"`
with a `form` — `ident`, `wildcard`, `literal` (`value`), `enum`
(`variant`, `patterns`), `struct` (`type`, `fields`), `record`
(`fields`), `list` (`items`, `rest`), `tuple` (`items`), `ok`/`err`
(`inner`), `range`, `or` (`alternatives`), `guarded` (`pattern`,
`guard`), `rest` — and its source as `text`. A `fn` or `lambda` carries
`parameters` (each `name`, `type`, `has_default`) beside the bare
`params` names, and a `fn` its `return_type`; a `let` its `type`. A type
node is `kind: "type"` with `text` and a `form` — `basic`/`named`
(`name`), `record` (`fields` of `name` and `type`), `list` (`element`),
`map` (`key`, `value`), `tuple`/`union` (`types`), `result` (`ok`,
`err`), `function` (`params`, `returns`), `generic` (`base`, `args`),
`literal` (`value`) — so an exhaustiveness or shape rule is an olang
program over `meta.parse`. A `type` declaration's alias `type` is such
a node (a record alias's `fields` are the shape), and a struct's
`fields` carry their `type` as text beside a `type_node`. Enough to
*analyze* a program, not to perfectly reconstruct one. See
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
map keys in the map's order, so the literal makes the same map; and `meta.fresh(prefix)`
yields a name no program writes by hand, for generated temporaries: the
form is `prefix__m<N>`, numbered per expansion, and the expander refuses
a macro-using program that spells a name of that form, so a generated
temporary cannot collide with a hand-written binding. Inside a meta fn,
`meta.exports("lib.decl")` reads the literal top-level bindings of a
module the expanding file imports, so a macro in one file can act on a
declaration written in another.

```olang
println(to_string(unwrap(meta.eval("2 + 3"))))     // 5
println(meta.lit([1, "a"]))                        // [1, "a"]
```

## `runtime` — what this binary carries

The olang binary carries its browser runtime: the playground wasm
(olang compiled for `wasm32-unknown-unknown`, the browser profile the
web SDK ships), embedded at build time from the artifact `cargo xtask
wasm` builds. The web SDK's `serve` reads it from here, so an app has no
wasm file to build, copy, or keep in step with the binary; a program
that serves its own routes reads the same bytes.

| Function | Result |
|---|---|
| `runtime.wasm()` | `Ok(#{ "bytes", "hash", "gzip", "br", "version" })` — the runtime's bytes, the first sixteen hex digits of their SHA-256 (the name in `/olang.<hash>.wasm` and the ETag), the gzip and brotli forms, and the olang version it is. Every form and the hash are made when the binary is built and are slices of its own image: the call computes nothing, copies nothing to the heap, and may be made per request; `Err(message)` naming how to build a binary that carries one |
| `runtime.version()` | the olang version string |
| `runtime.memory()` | `#{ "heap", "program", "values", "embedded", "tasks" }`, in bytes — where the process's memory is (below) |
| `runtime.profile_start()` / `runtime.profile_start(interval_us)` | begin sampling this process with the profiler behind `olang profile` (every 1,000 µs by default): `Ok(())`, or `Err` when a profile is already running |
| `runtime.profile_stop()` | end it: `Ok(#{ "interval_us", "ticks", "idle", "blocked", "samples", "rows" })`, where `rows` is `[#{ "function", "tier", "samples", "share" }]`, most first — the functions that were running when the sampler looked, the tier each ran on (`interp`, `vm`, `native`, `builtin`), and its share of the samples that landed in code. `blocked` counts thread-ticks spent parked — a receive, a sleep, a server waiting for a connection, an idle http worker — which are charged to no function. `Err` when no profile is running |
| `runtime.profile_live_start(dir)` `runtime.profile_live_start(dir, #{ "every_ms", "interval_us", "label" })` | a live profile of this process from now — what `olang profile --format json --live DIR` makes of a program it runs: stacks sampled by tier every `interval_us` (1,000), and a snapshot of the profile and the vital signs written to `DIR/<pid>.json` every `every_ms` (500, at least 50) in that format (docs/tooling.md), cumulative and replaced atomically. `Ok(())`, or `Err` when a profile already runs (`--profile-live`, `olang profile`, `runtime.profile_start`, an earlier call). Nothing is paid until it is called: an app that draws its own profile (olang Studio) turns it on and off |
| `runtime.profile_live_stop()` | end it: sampling stops, the final snapshot (`"done": true`) is written. `Ok(())`, or `Err` when none runs |
| `runtime.build()` | what this binary is: `#{ "version", "commit", "branch", "dirty", "date", "profile", "target", "rustc", "features", "crates", "exe" }` — the git commit it was built from (empty outside a checkout) and whether tracked files had changes then (`dirty`, `()` when git could not say), when its build script last ran (UTC: the last change to the sources, the lock or the commit), `release` or `debug`, the target triple, `rustc -V`, the Cargo features, the gui engine's and the stdlib's key crates from Cargo.lock as `[#{ "name", "version", "vendored" }]` (`vendored`: a patched copy under vendor/, accesskit_macos), and the binary's path |
| `runtime.load_module(path, grant)` | the olang file at `path` loaded into this program as a module, read afresh (its folder's cached modules forgotten), everything under its folder held to `grant` — `#{ name, fs: false \| "read" \| true, fs_roots: [folders], proc: false \| true \| [programs], net, db, env }`, a key left out denied, never the host's builtins (`gui`, `tty`, loading code, the profiler). `Ok(module)`, or `Err(message)` with the grant before it kept (packages.md, "Modules loaded at run time") |
| `runtime.unload_module(path)` · `runtime.module_grant(path)` · `runtime.module_permits(path, builtin, args)` | forget a loaded module's grant (true when there was one); the grant as it holds (`()` when none); whether its code would be allowed `builtin(args…)` — `Ok(())` or `Err(why)`, the same gate |
| `runtime.call_budget(f, args, ms)` | `f(args…)` stopped past `ms` (an interrupt on this thread only) and its failure caught: `#{ ok, value, error, frames, over, us }` |
| `runtime.last_error()` | the error a boundary on this thread caught last (`attempt`, `call_budget`, `load_module`) with its frames, `#{ message, frames: [#{ path, line, function }] }`, innermost first (line 0: an expression body); `()` when none |
| `runtime.shape(value)` | a value's shape as text — kind, a map's sorted keys with theirs, a list's first item's — what a host compares to keep a reloaded module's state |
| `runtime.bind(node, value)` | a control a host binds (what `ui.slider` and its kin answer): `value` with no host; in a notebook's kernel the value the editor set for the evaluating cell's control while it still fits. Also in the browser build, where `runtime` has nothing else |
| `runtime.preset(value, name)` | the next control the program binds with no host answers `value`, held to the control's kind and range or options (an error naming `name` when it does not fit): what `olang run x.olmd --param region=west` sets for a notebook's control cell |

```olang no-run
let rt = unwrap(runtime.wasm())
fn wasm(req, params) = {
    status: 200, body: rt.br,
    headers: #{ "Content-Type": "application/wasm", "Content-Encoding": "br",
                "ETag": "\"" + rt.hash + "\"" }
}
```

`runtime.memory()` answers from a counting layer over the system
allocator, so the figures are exact and the call is cheap enough for a
`/profile` route:

| Key | Meaning |
|---|---|
| `heap` | bytes allocated and not yet freed, across every thread |
| `program` | the share the loaded program holds: the heap's growth measured across parsing the entry file and across each outermost `use` with everything it loads |
| `values` | `heap - program`: the running program's values, tables, and buffers |
| `embedded` | the browser runtime and its compressed forms in the binary's image — file-backed pages, not heap |
| `tasks` | `[#{ "name", "bytes" }]`, largest first: each live spawned task (`olang-spawn-N`) and http worker (`olang-http-N`) with the bytes that thread allocated and has not itself freed |

A task's figure is attributed by the thread that made the allocation: a
value a task builds and hands to a joiner counts on the task until the
task ends, when its balance folds into the rest of the heap. Resident
memory as the operating system reports it is larger than `heap` — it
includes the binary's image, thread stacks, and pages the allocator has
freed but not yet returned.

```olang no-run
let m = runtime.memory()
let mb = (key) => to_string(map_get(m, key) / 1048576) + " MB"
println("program " + mb("program") + ", values " + mb("values"))
```

A binary built without the artifact (a fresh checkout's `cargo build`
before `cargo xtask wasm`, or a `cargo install` from crates.io) answers
`Err`; `serve` then serves the shell and answers the runtime's URLs
with a 503 that says what to build.

## `caps` — what this code was granted

A program run under a capability grant (an `olang.toml`'s
`[capabilities]`, a dependency's attenuation, or `--deny`) can ask what
it holds before making a call the gate would stop. The answer is the
**caller's** grant: code in an attenuated dependency hears what that
dependency was given. None of the three functions is itself gated
([Packages and dependencies](packages.md#asking-what-you-were-granted)).

| Function | Description |
|---|---|
| `caps.allowed(name)` | whether the capability is held: `"fs"` (at read level or above), `"net"`, `"proc"`, `"db"`, `"env"`; another name raises |
| `caps.level(name)` | the grant's own word for it, as a manifest writes it: `false`, `"read"` or `true` for `"fs"`; `false` or `true` for the others |
| `caps.granted()` | the whole grant as a map, in the same words: `fs` `false`, `"read"` or `true`, the others a Bool |

```olang
let plan = if caps.allowed("fs") => "cache to disk" else => "in memory"
println(caps.level("net"))
println(typeof(caps.granted().fs))
```

## `os` — operating system

The process's view of its world: arguments, environment, directories,
stdin, and the ability to run other programs. Together with `fs` it is
what makes olang a scripting language in the practical sense — the
[task CLI](../examples/) and the examples harness are built on little
else.

| Group | Functions |
|---|---|
| Process | `args` `exit(code)` `pid` `exe_path` `exec(argv)` |
| Terminal | `is_tty()` — is stdout a terminal? (`Bool`); `stdin_is_tty()` — is stdin? (`Bool`: `false` reading a pipe or a file); `color_enabled()` — will ANSI styling render? (`CLICOLOR_FORCE` set, or a terminal with `NO_COLOR` unset; decided once, re-decided after the program's own `set_env`/`remove_env`); `flush()` — flush buffered stdout, for progress bars |
| Input | `read_line()` — one line from stdin as `Ok(line)`, `Err("eof")` at end; `stdin()` — everything to end-of-file as one string; `stdin_lines()` — everything as a list of lines, endings stripped. The stdin pair is what makes olang pipe-friendly: `cat access.log \| olang analyze.ol` |
| Environment | `get_env` (`Err` when unset) `env` (the value or `()`) `set_env` `remove_env` `has_env`; `list_env()` — every variable as a map, `#{ "PATH": "…", … }` |
| Directories | `cwd` `chdir` `home_dir` `temp_dir` |
| System | `hostname` `username` `os_type` `arch` `family` `path_separator` |
| Signals | `on_interrupt()` — trap Ctrl-C (SIGINT) instead of terminating (`Result`: installing the handler can fail); `interrupted()` — has it been pressed? (`Bool`); `reset_interrupt()` — clear the flag; `on_shutdown(handler)` — run `handler("shutdown")` on SIGINT or SIGTERM (below) |

`os.exec(argv)` runs an external program to completion — `argv` is
the program and its arguments as one list, `["git", "status"]`, as
`proc.spawn` and `pty.spawn` take it — and returns
`Ok({ code, signal, stdout, stderr, timed_out })` — or `Err` if it could not be
launched at all (the program was not found, or `cwd` names no
directory: the error says which). An optional second argument configures
the child: `#{ "cwd": dir, "stdin": text, "env": #{ name: value },
"clear_env": true, "timeout_ms": n, "bytes": true }` (any subset; in
`env` a name given `()` is removed from the child's environment, and
`clear_env` starts it with none but `env`'s — as `pty.spawn`,
`proc.spawn` and `proc.pipeline` read them). With `timeout_ms` the program runs in a
process group of its own; if it has not exited and closed its output
within `n` ms, the whole group is killed — a shell and everything it
started — and the answer is `Ok` with `timed_out: true`, `code: ()`, and
the output read before the kill. `code` is the exit code when the
program exited by itself, `()` when it did not (a timeout, or a signal:
`signal` names it, `()` otherwise). (Being in its own group, it does not
receive the terminal's Ctrl-C.) Without `timeout_ms` the run is
unbounded and `timed_out` is `false`. `stdout` and `stderr` are text,
any bytes in them that are not UTF-8 replaced by U+FFFD; with `"bytes":
true` among the options they are `Bytes`, exactly as the program wrote
them.
`os.args()` is the program's argv (`[script, arg1, ...]`).

Most of `os` cannot fail, and as of 0.64 says so: `args`, `arch`,
`os_type`, `family`, `pid`, `path_separator`, `temp_dir`, `username`,
`is_tty`, `stdin_is_tty`, `color_enabled`, `flush`, `has_env`, `list_env`, `set_env`, `remove_env`,
`interrupted`, and `reset_interrupt` all return their value directly.
The ones that keep `Result` are the ones that genuinely can fail:
`get_env` (the variable may be absent), `cwd`, `chdir`, `home_dir`,
`hostname`, `exe_path`, `exec`, `read_line`, `stdin`, `stdin_lines`, and
`on_interrupt`.

```olang no-run
let args = os.args()
let target = if len(args) > 1 => args[1] else => "."

let r = unwrap(os.exec(["git", "status", "--short"], #{ "cwd": target }))
if r.code == 0 => print(r.stdout)
else => println("git failed: " + r.stderr)
```

A server that should finish its work when told to stop registers a
shutdown handler: `os.on_shutdown(handler)` runs `handler("shutdown")`
on SIGINT or SIGTERM, on its own thread against a thread-safe copy of
the program, and `http.shutdown()` from inside it asks every running
`http.serve` to drain — no new connections, in-flight requests finish,
workers exit, `serve` returns `Ok`. A `kill` is then a request, not an
interruption.

A handler that must wait — a long poll, a job whose answer arrives on a
channel — need not hold its worker while it does. `http.defer()` inside
a handler answers a ticket; the handler returns that ticket, the worker
moves to the next connection, and the parked connection costs a socket.
`http.respond(ticket, response)` completes it from wherever the answer
turns up — a spawned task, a channel service, a later request on
another worker — with any value a handler could have returned; it is
`Ok(())`, or `Err` when nothing holds the ticket (answered already, or
the client left). The connection closes after the deferred answer.

```olang no-run
fn handle(req) = if req.path == "/wait" => {
    let ticket = http.defer()
    spawn { let answer = chan.recv(updates); http.respond(ticket, http.response(200, show(answer))) }
    ticket
} else => "quick"
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
| Spawn | `spawn(argv)` / `spawn(argv, #{ cwd, env, clear_env, group, framing, bytes })` → `Ok(Process)` (`argv` the program and its arguments as one list, as `pty.spawn` and a pipeline stage take them; an `env` name given `()` is removed); a `cwd` that names no directory is an `Err` saying so. Output is text, bytes that are not UTF-8 read as U+FFFD; `bytes: true` makes `read_all` and `stderr` answer `Bytes` as written (and `read_line`, which reads text, raise) |
| Input | `write(p, s)` · `write_line(p, s)` · `write_frame(p, s)` (one `Content-Length`-framed message) · `close_stdin(p)` (signals EOF) |
| Output | `read_line(p)` → `Ok(line)` \| `Err("eof")`; `read_all(p)` → the rest of stdout; `stderr(p)` → all stderr (complete after exit) |
| Lifecycle | `wait(p)` → `Ok({ code, signal })` (the exit code and `()`, or `()` and the signal that ended it) · `kill(p)` (SIGKILL) / `kill(p, signal)` (`"TERM"`, `"INT"`, `"HUP"`, …, or a number: a process asked to end) / `kill(p, #{ tree: true, signal })` · `pid(p)` |
| Pipeline | `pipeline(stages)` / `pipeline(stages, #{ cwd, env, clear_env, stdin, bytes })` — `bytes: true` answers `stdout` and `stderr` as `Bytes` |

```olang no-run
// Stream a filter: feed lines in, read matches back out.
let p = unwrap(proc.spawn(["grep", "olang"]))
proc.write_line(p, "olang rules")
proc.write_line(p, "nope")
proc.close_stdin(p)
println(unwrap(proc.read_line(p)))     // "olang rules"
println(show(unwrap(proc.wait(p)).code))
```

A child that speaks the language server protocol frames its messages by
`Content-Length` headers, not lines (a message's body ends with no line
break). Spawn it with `#{ "framing": "content-length" }` and each
`read_line` is one whole message body; `write_frame` sends one framed.

```olang no-run
let p = unwrap(proc.spawn(["olang", "lsp"], #{ "framing": "content-length" }))
unwrap(proc.write_frame(p, unwrap(json.stringify(#{ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": #{} }))))
println(unwrap(proc.read_line(p)))     // the initialize result, whole
```

`proc.kill(p)` ends the child alone (SIGKILL); a shell's children
outlive it. To end everything a child started, spawn it with
`#{ "group": true }` — it leads a process group of its own — and kill
it with `proc.kill(p, #{ "tree": true })`, which kills the whole group
(`taskkill /T` on Windows). A tree kill of a child spawned without
`group` is an `Err`. A grouped child does not receive the terminal's
Ctrl-C with the program; that is the point, and the reason it is opt-in.

```olang no-run
let p = unwrap(proc.spawn(["sh", "-c", "make test"], #{ "group": true }))
// … the deadline passes …
unwrap(proc.kill(p, #{ "tree": true }))   // make, and everything it ran
```

`proc.pipeline` chains commands the way the shell's `a | b | c` does —
each stage is a list `[program, ...args]`, and each one's stdout is wired
to the next one's stdin. It returns `Ok({ code, stdout, stderr, codes })`:
the last stage's stdout, all stderr concatenated, the final exit code, and
the per-stage codes list (a stage ended by a signal has `()`). `opts.stdin` feeds the first stage.

```olang no-run
let r = unwrap(proc.pipeline([
    ["printf", "one\ntwo\nfour\n"],
    ["grep", "o"],
    ["wc", "-l"]
]))
println(str.trim(r.stdout))    // "3"
println(show(r.codes))         // [0, 0, 0]
```

The [`watch` example](../examples/tools/watch/) puts the module together:
it streams a command's output, runs pipelines, and shuts down cleanly on
Ctrl-C.

## `pty` — a child on a pseudo-terminal

Where `proc` pipes a child's stdin and stdout, `pty` gives it a terminal
of its own: the child leads a new session with the pty's slave as its
controlling terminal (job control, `^C`, window sizes and full-screen
programs work), and the program holds the master side. What a terminal
emulator (olang Studio's) runs a shell in. macOS and Linux (an `Err`
elsewhere: `pty.available()`); capability-gated as `proc`.

| Function | Description |
|---|---|
| `pty.spawn(argv, opts?)` | start `argv[0]` → `Ok(Pty)`. `opts`: `cwd`, `env` (a map: a String sets a variable, `()` removes one), `clear_env`, `rows`, `cols` (24 × 80). The child's signals are at their defaults (SIGPIPE too) |
| `pty.read(p, opts?)` | what it wrote → `Ok(Bytes)`, `Ok(())` when nothing came within `timeout_ms` (1000), `Err("eof")` once its side closed. Having read something it reads on while more arrives within `settle_ms` (2), up to `max` bytes (1 MiB): a burst is one value. For a reader task of its own |
| `pty.write(p, data)` | a String or Bytes (the keys typed), queued for a writer thread (a large paste never blocks) |
| `pty.resize(p, rows, cols)` | TIOCSWINSZ: the foreground job hears SIGWINCH |
| `pty.kill(p, sig?)` | `"HUP"` (the default), `"INT"`, `"TERM"`, `"KILL"`, `"QUIT"`, `"CONT"`, `"STOP"` or a number, to the shell's process group and the foreground job's |
| `pty.wait(p, ms?)` / `pty.wait(p, #{ timeout_ms })` | `Ok({ code, signal })` once it ended (waiting up to `ms`, 0), `Ok(())` while it runs |
| `pty.foreground(p)` | the terminal's foreground job `{ pid, name, shell }` (`shell` while the shell is at its prompt), or `()` once it ended — ask before closing a terminal a program runs in |
| `pty.cwd(p)` | the foreground job's working directory (the shell's at its prompt), or `()` |
| `pty.pid(p)` · `pty.shell()` · `pty.available()` | the child's pid; the user's login shell (`$SHELL`, the user database's, `/bin/zsh`); whether ptys exist here |
| `pty.close(p)` | hang up: SIGHUP to its groups, what it writes as it ends read and dropped, the child reaped (SIGKILL after 500 ms), the master closed. At once; the handle is gone |

A terminal that closes waits for what it wrote to be read (macOS): keep
reading until `eof`, then `wait`.

```olang no-run
let p = unwrap(pty.spawn([pty.shell(), "-l"], #{ "cwd": "/tmp", "rows": 30, "cols": 100, "env": #{ "TERM": "xterm-256color" } }))
unwrap(pty.write(p, "ls\r"))
match pty.read(p, #{ "timeout_ms": 500 }) { Ok(b) => println(b), Err(e) => println(e) }
pty.close(p)
```

## `vt` — a terminal's screen

What a program on a terminal writes — xterm's escape sequences — read
into a grid of cells with a scrollback, and drawn as a canvas's
operations: a terminal emulator's core, in Rust so a megabyte of output
costs milliseconds. Screens are `Screen` handles into a registry: one may
be fed on a reader's thread and drawn on the window's.

It reads UTF-8 (wide characters take two cells, combining marks join the
cell before; a sequence split across feeds is kept), C0 controls, ESC
(save/restore the cursor, index, reverse index, next line, reset, the
keypad's modes, character sets with DEC line drawing), CSI (cursor
movement, erase in display and line, insert/delete characters and lines,
scroll up/down, scroll regions, tab stops, repeat; SGR with 16, 256 and
true colours, bold, dim, italic, underline, inverse, hidden,
strikethrough; modes: insert, newline, application cursor keys, origin,
autowrap, the cursor shown, the alternate screen 47/1047/1049, bracketed
paste 2004, mouse reporting 1000/1002/1003 and the SGR encoding 1006,
focus events 1004; the cursor's style; device status, attributes and a
mode's state answered), OSC (0 and 2 a title, 7 the working directory,
8 hyperlinks, 10 and 11 the colours asked) and DCS/SOS/PM/APC read and
ignored. Lines are numbered from the first ever kept, so a selection
stays on its text while the scrollback (10,000 lines) moves; a resize
rewraps the main screen.

| Function | Description |
|---|---|
| `vt.new(rows, cols, opts?)` | a screen. `opts`: `scrollback`, `fg`, `bg` (what OSC 10/11 answer) |
| `vt.feed(t, data)` | read a String or Bytes → what the terminal answers the program (a cursor report), often `""` |
| `vt.resize(t, rows, cols)` · `vt.clear(t)` · `vt.reset(t)` | the size (rewrapped); ⌘K's clear (the cursor's line kept as the first); RIS |
| `vt.info(t)` | `{ rows, cols, title, cwd, alt, cursor (line, col), cursor_row, cursor_visible, cursor_style, cursor_blink, app_cursor, app_keypad, bracketed_paste, mouse, mouse_sgr, focus_events, bell, rev, first, screen, last }` |
| `vt.claim(t)` | true once after each `vt.info`: a reader says "changed" once until the window looks (a burst is a frame) |
| `vt.render(t, opts)` | canvas operations: backgrounds, text runs (bold, italic), underlines, strikes, the cursor (block, bar, underline; hollow; the input method's text). `opts`: `cell_w`, `cell_h`, `size`, `font`, `pad_x`, `pad_y`, `text_dy`, `rows`, `top`, `palette` (16), `fg`, `bg`, `cursor_color`, `cursor_text`, `selection_color`, `find_color`, `find_current_color`, `link_color`, `selection` `(l0, c0, l1, c1)`, `matches` and `match`, `hover`, `cursor`, `hollow`, `cursor_style`, `preedit` |
| `vt.text(t, l0, c0, l1, c1)` · `vt.line(t, l)` · `vt.word_at(t, l, c)` · `vt.link_at(t, l, c)` · `vt.find(t, q, opts?)` | a selection's text; a line's text and each character's column; a double click's word; an OSC 8 link; every match `(line, col, cells)` — a line that wrapped read as one (a match's `cells` may run onto the next row), the newest 2,000 |
| `vt.set_colors(t, #{ fg, bg })` · `vt.free(t)` | replace the colours OSC 10/11 answer, as `vt.new`'s `fg` and `bg` set them; release the screen (a later call on it does nothing) |

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
    Ok(a) => if a.help => println(cli.help(spec))
        else => {
            let line = a.who
            let shout = if a.loud => str.to_upper(line) else => line
            for i in range(0, a.count) { println("Hello, " + shout + "!") }
        }
}
```

[`examples/tools/taskcli`](../examples/tools/taskcli/) is the worked example — a
small task tracker whose entire command surface (`list`, `open`,
`stats`, `add --priority`) is one `cli` spec; `--help` and the exit
codes come from the package.

## `term` — the terminal toolkit

`use term`, an embedded olang package, does for the terminal what
`plot`/`viz` do for the browser: color and text styling, aligned tables
and rules, progress bars, and interactive prompts. Styling is emitted
**only when it will render** (standard output is a TTY and `NO_COLOR`
is unset) or when `CLICOLOR_FORCE` is set, the convention that also
makes styled output testable through a pipe. The decision is made once
and made again after the program's own `os.set_env`/`os.remove_env`, so
a program's output is colored on a terminal and plain in a pipe or file
with no extra logic.

| Group | Functions |
|---|---|
| Color | `red` `green` `yellow` `blue` `magenta` `cyan` `white` `black` `gray` — each wraps a string |
| Attributes | `bold` `dim` `italic` `underline` |
| General | `style(s, opts)` — `opts` is `#{ "fg": ..., "bg": ..., "bold": ..., "dim": ..., "italic": ..., "underline": ... }`; `color()` — is styling active (a TTY without `NO_COLOR`, or `CLICOLOR_FORCE`; decided once, re-decided after the program's own `os.set_env`/`os.remove_env`) |
| Structure | `rule(width)` — a horizontal line; `table(headers, rows)` — columns aligned to their widest plain cell, header bold |
| Measure | `visible_len(s)` — the width of `s` on screen, its styling escapes not counted (`visible_len(term.red("hi"))` is 2) |
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

## `ui`, `viz` and `dash` — views, charts and dashboards

Three embedded packages, each imported with `use`, that build HTML and
SVG as strings: pure functions, testable natively, mounted in a page
with `dom` ([olang in the browser](wasm.md)).

| Function | Description |
|---|---|
| `ui.h(tag, attrs, children)` | a node: a map of `tag`, `attrs` and `children` (a String child is a text node) |
| `ui.hk(key, tag, attrs, children)` | a node with a reconciliation key |
| `ui.esc(s)` | `s` (a non-String is first written with `to_string`) with `&`, `<`, `>` and `"` escaped for HTML |
| `ui.html(node)` | a tree rendered to an HTML string, its text escaped |
| `ui.render(el, children)` | a keyed list mounted in `el`, diffed against what it rendered last ([wasm.md](wasm.md)) |
| `ui.control(kind, value, spec)` | a control as a node: `html` renders a real HTML control (`input` range, date, text or checkbox; a `select` and its options; a brush's `div`), and the node says its `control` kind and its `spec` (`min`, `max`, `step`, `options`, `chart`) |
| `ui.slider(value, min, max, step = ())` | a number in [min, max] a host binds (an Int slider when the value and the bounds are Ints) |
| `ui.select(value, options)` | one of `options` a host binds |
| `ui.date(value)` · `ui.text(value)` · `ui.toggle(value)` | a `"YYYY-MM-DD"` String, a String, a Bool a host binds |
| `ui.brush(chart, value = ())` | a range of a chart's x a host binds: `()` for none, or `[x0, x1]` (Floats); `chart` a `plot` chart's SVG or a [`gg`](#gg--a-grammar-of-graphics) plot (the range along the dragged panel's horizontal axis, in data units: a log10, sqrt or reversed scale undone) |
| `viz.check(spec)` | `Ok(spec)` when a chart spec will render, or `Err` naming an unknown key or the absence of points to draw |
| `viz.chart(spec)` | a spec compiled to SVG text ([the `viz` grammar](ods.md#the-viz-grammar)) |
| `viz.draw(canvas, spec)` | the same spec drawn on a canvas, in the browser |
| `viz.tooltip(el)` / `viz.on_mark(el, event, handler)` / `viz.brush(el, handler)` | a chart's hover tip, mark events, and a dragged range |
| `dash.kpi(label, value, note)` / `dash.stat(label, value, note, accent)` | a number tile (`note` `""` for none; `accent` a color for the value) |
| `dash.card(title, inner)` / `dash.half(title, inner)` / `dash.wide(title, inner)` | a titled panel one, two, or every column wide; `inner` is HTML and is not escaped |
| `dash.grid(cards, columns)` | the tiles and cards in a grid of `columns` columns |
| `dash.styles()` | the kit's stylesheet, a `<style>` element to prepend once |

**Controls.** `ui.slider` and its kin check their value (a slider's
within its range, a select's one of its options, each its type) and
raise naming what is wrong, then answer `runtime.bind(ui.control(…),
value)`. With no host — `olang run`, `test`, `check`, a plain REPL — a
control is its value, so a notebook runs as a program. A notebook's
kernel is a host: it says each cell's control to the editor, and the
value the editor sets is the control's when the cell runs again, while
it still fits (the same kind and default, within the range or options
now) — see [notebooks](notebooks.md#controls). Binding reaches no
capability: a control cell is pure.

```olang
use ui
use dash
println(ui.html(ui.h("p", #{ "class": "note" }, ["a < b"])))
println(str.contains(dash.kpi("open", "12", ""), "open"))
```

## `tty` — the terminal as an interactive device

`term` styles lines; `tty` takes the terminal over. It is the layer a
full-screen program stands on — the Heddle terminal UI framework's
runtime is built on it — and it gives four things: raw mode and the
terminal's reporting modes, decoded input as events on a channel,
unbuffered output and capability queries, and a **restore guarantee**:
whatever `tty.enter` changes is changed back however the program ends.
Native-only.

| Function | Description |
|---|---|
| `tty.enter(opts)` | take the terminal → `Ok(handle)`; `Err` when stdin or stdout is not a terminal, or a handle is already entered (one at a time). `opts` below |
| `tty.leave(h)` | give it back: cooked mode, the main screen, the cursor shown, reporting off, the events channel closed. Idempotent; returns `()` and never fails |
| `tty.events(h)` | the channel of decoded events (below) |
| `tty.size()` | `Ok((cols, rows))`, or `Err` when stdout is not a terminal. Needs no `enter` |
| `tty.write(h, s)` | write a String or Bytes now — after anything `print` left buffered, without waiting for a newline → `Ok(())` / `Err` |
| `tty.query(h, seq, ms)` / `tty.query(h, seq, #{ timeout_ms })` | write a query (`"\u{1b}]11;?\u{1b}\\"` for the background colour, `"\u{1b}[c"` for DA1, `"\u{1b}[?u"` for the kitty flags) → `Ok(reply)` — the reply's raw bytes — or `Err("timeout")` |
| `tty.is_tty()` | stdin and stdout are both terminals — what `enter` needs |
| `tty.suspend(h)` | ctrl+z done right: restore, stop, and on `fg` re-enter and deliver `signal` `cont` and a `resize` |

**Options.** `tty.enter` takes a map of Bools, each optional; an
unknown key raises (a typo'd option is a bug, not a preference).

| Option | Default | What it turns on | Undone by |
|---|---|---|---|
| `raw` | `true` | raw mode (`cfmakeraw`): no echo, no line editing; ctrl+c, ctrl+z, ctrl+\\ arrive as keys, not signals; output is not translated, so a line ends `"\r\n"` | the saved settings, restored exactly |
| `alt_screen` | `false` | the alternate screen (`CSI ?1049h`) | `CSI ?1049l` |
| `hide_cursor` | `false` | `CSI ?25l` | `CSI ?25h` (written on every leave) |
| `mouse` | `false` | clicks, drags, and motion, SGR-encoded (`?1000 ?1002 ?1003 ?1006`) | each reset |
| `paste` | `false` | bracketed paste (`?2004`) | `?2004l` |
| `focus` | `false` | focus in/out reports (`?1004`) | `?1004l` |
| `kitty_keys` | `false` | the [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/), flag 1 (disambiguate: ctrl+i is not tab, esc is unambiguous) — pushed with `CSI >1u`, ignored by a terminal without it | `CSI <u` (pop) |

Every leave also writes `CSI ?2026l` (ends a synchronized update a
crash left open) and `CSI 0m` (resets attributes), so a program that
dies mid-frame leaves neither a frozen nor a coloured terminal.

**Events** are maps with a `"kind"` (read their fields as `ev.kind`, `ev.chord`):

| `kind` | Fields |
|---|---|
| `"key"` | `key` — the character (`"a"`, `"A"`, `"中"`, `"?"`) or a name: `space enter tab backspace esc up down left right home end pgup pgdown insert delete begin f1`…`f35` (and, under the kitty protocol, keypad and modifier keys by name); `text` — what the key types, `""` for a named key or with ctrl/alt/super; `ctrl` `alt` `shift` `super` — Bools; `chord` — the key in one spelling: modifiers in the order ctrl, alt, shift, super, then the key (`"ctrl+d"`, `"alt+enter"`, `"shift+tab"`, `"ctrl+shift+a"`; a capital typed plainly is itself, `"G"`) |
| `"paste"` | `text` — one bracketed paste, whole, CR and CRLF turned into LF |
| `"mouse"` | `action` — `"down"` `"up"` `"move"` (no button) `"drag"` (a button held) `"wheel"`; `button` — `"left"` `"middle"` `"right"` `"none"`, or `"up"` `"down"` `"left"` `"right"` for the wheel; `x`, `y` — zero-based cells; `mods` — `#{ ctrl, alt, shift }` |
| `"resize"` | `cols`, `rows` — after SIGWINCH (Unix) or a console resize (Windows) |
| `"focus"` | `on` — the window gained (`true`) or lost focus |
| `"signal"` | `name` — `"tstp"` and `"cont"` around a stop; `"term"`, `"hup"`, `"int"` when the program trapped the signal (below); `"eof"` when the terminal hangs up (the channel closes after it) |

A window size change arrives as `resize`, not as a `signal`. After
`cont`, repaint everything: the screen may have been used by the shell.

**Esc and alt.** Alt+x arrives as ESC then x, which is also what
pressing Escape then x quickly would send. The decoder waits 25 ms
after a lone ESC (250 ms after a longer partial sequence) for more bytes
before deciding: ESC alone is `esc`; ESC and a key in the same moment is
alt+key. The kitty protocol removes the ambiguity entirely where the
terminal has it.

**Queries.** A reply is taken out of the input stream wherever it
arrives: OSC, DCS and APC strings, CSI replies with a private prefix
(`?` `>` `=`) or an intermediate byte (DA1, DA2, DECRPM, the kitty flags
reply), and cursor position reports while a query waits. A reply that
arrives after its query timed out is dropped — never delivered as keys.
`tty.query` needs `raw` (a cooked terminal echoes the reply and holds it
for a newline). The answer is the first reply, with any that arrived in
the same read; to fence a query the terminal may not answer, send it
followed by DA1 (`"\u{1b}[?u\u{1b}[c"`), which every terminal answers.

```olang no-run
if !tty.is_tty() => { println("needs a terminal"); os.exit(0) }
let h = unwrap(tty.enter(#{ "alt_screen": true, "hide_cursor": true, "mouse": true, "paste": true }))
let events = tty.events(h)
let (cols, rows) = unwrap(tty.size())
tty.write(h, "\u{1b}[2J\u{1b}[H" + show(cols) + "x" + show(rows) + " — q quits\r\n")
let mut going = true
while going {
    match chan.recv(events) {
        Ok(ev) => {
            if ev.kind == "key" && ev.chord == "q" => { going = false }
            else if ev.kind == "key" && ev.chord == "ctrl+z" => { tty.suspend(h) }
            else => { tty.write(h, show(ev) + "\r\n") }
        },
        Err(e) => { going = false }
    }
}
tty.leave(h)
```

### The restore guarantee

`tty.enter` records everything it is about to change — the terminal's
settings and the bytes that undo each mode — in a process-wide guard
*before* changing it. Restoring is one idempotent operation that
exactly one caller performs, and every way a process ends reaches it
without the program's help:

| The program… | Restored by |
|---|---|
| calls `tty.leave(h)` | `leave` |
| finishes without leaving | the runtime, as the program's evaluation returns |
| raises and does not catch it | the runtime, *before* the error is printed — the report lands on the main screen, in a cooked terminal |
| calls `os.exit(n)` | `os.exit`, before exiting |
| ends any other way through `exit` (a deadlock abort, a broken pipe) | an `atexit` handler (Unix) |
| hits a runtime panic, in any thread | the panic hook, before the panic message (also in release builds, where a panic aborts) |
| receives SIGTERM, SIGHUP, or SIGINT | the signal action: restore, then die of the signal exactly as the default action would (the exit status says so) |
| is stopped (SIGTSTP) | the signal action restores and stops; SIGCONT re-enters |

The signal actions restore with `write(2)` and `tcsetattr(3)` on data
recorded in advance — both async-signal-safe — so they work whatever
the program's threads were doing. A program that **traps** the signals
itself (`os.on_interrupt()` or `os.on_shutdown(f)`, called before
`tty.enter`) owns them: the guard does not end the process, the signal
arrives as a `signal` event (`"term"`, `"hup"`, `"int"`), and the
program leaves when it chooses (or `os.exit`s, which restores).

Nothing can restore after SIGKILL or a stack overflow (the runtime
aborts without a hook); `reset` in the shell is the remedy there.

`tests/tty_test.rs` holds the guarantee to account: a real olang
program in a pseudo-terminal enters raw mode and the alternate screen,
then raises, exits, ends, receives SIGTERM, SIGHUP, or SIGINT, panics,
or suspends — and the test asserts the leave sequences reached the
terminal after the enter, and that the terminal's settings are cooked
again, exactly as before.

**Platforms.** On Unix (macOS, Linux, the BSDs) the module reads and
decodes stdin itself — the decoder is `src/stdlib/tty/decode.rs`, tested
byte by byte — because crossterm's parser is private, consumes query
replies, and has no Esc timeout. On Windows it uses crossterm: console
raw mode and mouse capture, and the console's input records for keys,
mouse, resize, and focus, output as VT sequences (Windows Terminal and
ConPTY consoles). There, `tty.query` and `tty.suspend` answer `Err`,
bracketed paste arrives as keys, `kitty_keys` has no effect, and the
guard restores on leave, program end, `os.exit`, raise, and panic (a
console process has no SIGTERM). The restore tests have run on macOS;
Linux is compiled and clippy-clean, Windows is compiled against a
stand-in of the crate paths and clippy-clean — neither has been run.

## `gui` — windows drawn by olang

The engine under Loom, olang's desktop framework (`loom/SPEC.md`): a
program lays its view out itself and sends `gui` **positioned, keyed
nodes**; `gui` draws them on the GPU (or with the software reference),
edits text fields (input methods included), hit-tests, keeps the
platform's accessibility tree, and sends back events on one channel.
Loom's components, layout, commands, and tests are olang above it.
Native-only; on in the released binaries (the `gui` cargo feature).

| Function | Description |
|---|---|
| `gui.available()` | a window can be opened here: olang's own main thread, and a display (on Linux, a Wayland display) |
| `gui.open(opts)` | open a window → `Ok(window)`, or `Err` when there is no display. `opts`: `title`, `size` `(w, h)`, `min` `(w, h)`, `resizable`, `renderer` (`"gpu"`, the default, or `"software"`; also `LOOM_RENDERER`), `background`, `tabbing` (the windows it may share a tab bar with: macOS's tabbing identifier), `tab_of` (an open window's id: this one opens as that window's tab, macOS). A `resize` event with the real size and scale follows at once |
| `gui.headless(opts)` | a window with no display: drawn in software, fed by `gui.input`, read by `gui.read`. The same options, plus `scale`. The headless windows a thread made have one key window, as the platform's do: the one opened last is key, a press in another (or `gui.input` `"window_focus"` with `on`) makes that one key, `on: false` leaves none key, and the key window closed hands the key to the one key before it; each change is a `window_focus` event to the window it changes. A field's caret and focus ring are drawn only in the key window (`gui.read(w, "key")`) |
| `gui.close(w)` | close a window. Closing is the program's: the close button only sends `close_requested` |
| `gui.events()` | the channel every window's events arrive on (below) |
| `gui.apply(w, patch)` | apply a list of operations to the window's tree → `Ok(())`, or `Err` naming the operation and what is wrong with it |
| `gui.measure(text, style, opts?)` | the size of a text as it would be drawn → `#{ width, height, lines, baseline }`, logical pixels. `style` as a node's; `opts`: `width` (wraps when the style says `wrap`), `scale` |
| `gui.read(w, what, arg?)` | read the window back: `"focus"`, `"key"` (whether it is the key window: a field's caret and focus show only there), `"hover"`, `"size"` → `(w, h, scale)`, `"hit"` `(x, y)` → a key, `"node"` key (its role, text, name, children, scroll `offset`, `bounds`, and `visible`: what of it shows once scrolling and clipping ancestors and the window cut it, or `()`), `"value"` key and `"selection"` key of a field, `"keys"` (focus order), `"a11y"` (the accessibility tree as maps: each node's `key`, `role`, `name`, `value`, `depth`, states, `actions`, `custom` actions, `description`, `bounds`, `placeholder`, `level`, `expanded`, `live`, `in_set` `(index, size)`, `row`, `column`, a set's `size`, a table's `rows` and `columns`, `range`, a field's `text_runs` and `text_selection`), `"pixels"` (a PNG, from the software renderer), `"rgba"`, `"caret"` → `(x, y, w, h)` of the caret as the last frame showed it, clipped, or `()`, `"settings"` (the system's settings as the window last heard them), `"animation"` key (an animated picture: `#{ frames, frame, paused, button }`), `"next_frame"` (when an animation in view next changes, on the window's clock), `"motion"` (what transitions are moving: see Transitions), `"render"` (the renderers' frames and memory: see Rendering) |
| `gui.input(w, event)` | send input as the platform would: `#{ kind: "key", key, text?, mods? }` (`"mod": true` is the platform's command key), `"text"`, `"pointer"` (`action` down/up/move/leave, `x`, `y`, `button`, `clicks`), `"wheel"` (`dx`, `dy`, and the modifiers held: `mods` or `mod`, `shift`, `alt`, `ctrl`), `"pinch"` (`delta`, `phase`), `"modifiers"` (the modifier keys now held: `mod`, `shift`, `alt`, `ctrl`; a press with ⌘ held), `"compose"` and `"commit"` (an input method), `"resize"`, `"window_focus"`, `"files"` (`action` hover/drop/cancel, `paths`, `x`, `y`: files from another program); and what the platform does around a window: `"close"` (sends `close_requested`), `"menu"` with an `id` (sends `menu`), `"open"` with `paths` (sends `open`), `"new_tab"` (sends `new_tab`), `"quit"` with `why` (sends `quit`, as an external quit `gui.hold_quit` holds), `"clipboard"` with a `text` (the clipboard input through `gui.input` uses, the window's own), `"clock"` with `ms` (a headless window's clock: its animations move only with it), `"appearance"` with `dark`, `contrast`, `reduce_motion`, `reduce_transparency`, `accent` (`"#rrggbb"`) — the window takes them (its pictures stop animating under reduced motion) and sends `appearance`, as a settings change would, `"a11y"` with `key` and `action` (`"click"`, `"focus"`, `"set_value"` with `value`, `"increment"`, `"decrement"`, `"expand"`, `"collapse"`, `"scroll_up"`/`"_down"`/`"_left"`/`"_right"` (a page), `"scroll_into_view"`, `"custom"` with `index`) — an assistive request, as a screen reader makes it. What tests drive |
| `gui.prepare()` | start the platform's event loop now, without waiting for it: the application's start on the main thread (tens of milliseconds on macOS) then overlaps the program's work before its first window opens, which waits for the loop as before. Nothing when it is started already or cannot be (Loom's `run` asks, for real windows) |
| `gui.wake()` | a `wake` event on the channel, so a task blocked on `gui.events()` can stop without polling |
| `gui.context()` | the system's settings now: `#{ dark, contrast, reduce_motion, reduce_transparency, accent }` (`accent` the colour chosen in System Settings, `"#rrggbb"`, or `()`). macOS reads them from AppKit — the application's effective appearance (Auto included) and NSWorkspace's accessibility display options — and keeps them live: a window hears `appearance` the moment one changes (AppKit's notices for the display options and the system's colours, and a window's theme changing), with no restart. Elsewhere they are false and `dark` follows a window's theme |
| `gui.platform()` | what this platform's windows can do, to check rather than find out: `#{ os, native_menu, clipboard_image, file_drop, drop_position, window_position, system_settings, ime, accessibility, window_tabs, open_documents, context_menu, terminal, group_hover, scroll_bars, wheel_x, transitions, corner, shadow, vector, material, a11y_depth, flash, quit_hold }` (`transitions`: a style's `transition`; `corner`, `shadow`, `material`: a style's own (see Shapes); `vector`: a canvas path's `d`, `cap`, `join`, `dash`, `rule` and a radial gradient; `wheel_x`: a node's `wheel_x`; `terminal`: the `pty` and `vt` modules, a node's `input` and `ime_area` and the `text` and `preedit` events, a canvas text's `italic`; `context_menu`: `gui.context_menu` and the `context` event; `window_tabs`: `tabbing`, `tab_of`, `tab` and the `tabs` event; `open_documents`: the `open` and `new_tab` events; `quit_hold`: `gui.hold_quit` and the `quit` event — a program asks before it uses them, as an older engine refuses an option it does not know). On macOS all are `true` (`window_tabs` and `open_documents` only there); on Windows `native_menu`, `clipboard_image`, `drop_position`, and `system_settings` are `false`; on Linux (Wayland) `file_drop` and `window_position` are `false` too (winit has no Wayland drag and drop, and a Wayland window is never told where it is). `ime` and `accessibility` are `true` everywhere |
| `gui.compare(a, b, opts?)` | two PNGs compared as a pixel snapshot is: `#{ same, differing, total, worst, diff }`; a pixel differs past `threshold` (32 of 255), the images are the same while at most `ratio` (0.001) of the pixels differ; `diff` is a PNG with the differing pixels in red |
| `gui.hold_quit(on)` | `true`: a quit from outside the program — the Dock's Quit, AppleScript's `quit app`, a logout, a restart or a shutdown (AppKit's `applicationShouldTerminate:`) — is the program's `quit` event, and the application waits for the program to end, then exits with its status; `false` (the default): such a quit ends the process at once. The wait lasts until the program ends; meanwhile the platform runs no window handler, so `gui.open` and `gui.dialog` answer an error and what is sent to the windows waits. Loom's run loop holds quits and sends the message ⌘Q sends. macOS; `gui.platform()` says `quit_hold`. → `()` |
| `gui.turn(on)` | a program says a turn begins (`true`) and ends (`false`) — Loom's loop does, around each turn. One that runs past 2 s (`GUI_NOT_RESPONDING_MS`) makes every window show it is waiting (the busy cursor, a veil and a spinner over the frame) until it ends; the engine draws on meanwhile, so scrolls, looks and fields keep the refresh. → `()`. `gui.platform()` says `turn_watch` |
| `gui.set(w, props)` | `title`, `size`, `min`, `visible`, `cursor`, `focus`, `background`, `zoom` (the text size, 0.5 to 3: the window keeps its pixels and its boxes, reads, and events are in units of `zoom` logical pixels, so a program lays out in less room at a larger size; a `resize` event says the new size), `appearance` (`"light"`, `"dark"`, or `"system"`: the title bar and the platform's own controls in the appearance the program draws in), `scripted` (`true`: a script drives the window through `gui.input`, and a person's keys, pointer, wheel and drops on it are not taken — a test's window is not moved by the mouse passing over it), `tab` (macOS window tabs: `"next"`, `"previous"` tab chosen, `"merge"` every window into this one's tab bar, `"detach"` this tab into a window of its own, `"bar"` the bar shown or hidden; a `tabs` event follows), `corner` (`"continuous"` or `"round"`: the corners of every rounded box whose style says none), `flash` (`true`: the repaint rainbow — every region a frame draws tinted with that frame's colour, unseen by damage, so a region keeps the tint of the frame that last drew it; `false` draws the window whole again; `gui.platform()` says `flash`) |
| `gui.fonts(sources, opts?)` | register fonts: paths or Bytes, TTF/OTF/TTC, optionally brotli-compressed → the number of faces added. Registered fonts come before the system's. `#{ "background": true }`: the sources are read now, then decompressed and registered on a thread of their own while the program goes on — whatever shapes or measures text meanwhile waits until they are in — and it answers the number of sources (Loom's `run` registers its bundled fonts so: they were tens of milliseconds of a window's first frame) |
| `gui.clipboard_read()` / `gui.clipboard_write(text)` | the platform clipboard (headless windows' editing uses a private one) |
| `gui.clipboard_image()` | the clipboard's image (a screenshot copied, an image copied in another program) → `Ok(#{ png, width, height })`, `png` its PNG Bytes (a TIFF is turned into one), or `Err` when it holds none. macOS only so far: elsewhere an `Err` that says so (`gui.platform().clipboard_image`) |
| `gui.image_info(source)` | a picture's size and format from its header, a path or Bytes → `Ok(#{ width, height, format })` (`"png"`, `"jpeg"`, `"webp"`, `"gif"`, `"svg"`; a JPEG's turned as its EXIF orientation says, an SVG's in its own units), or `Err`. It reads the file: call it from a task |
| `gui.dialog(kind, opts)` | the platform's dialog: `"open"`, `"open_many"`, `"save"`, `"folder"` (`title`, `directory`, `name`, `filters: [#{ name, extensions }]`) → a path, a list, or `()` when cancelled; `"message"` (`title`, `text`, `level`, `buttons`: `ok`, `ok_cancel`, `yes_no`, `yes_no_cancel`) → the button |
| `gui.context_menu(window, items, at)` | a native context menu (macOS: NSMenu `popUpMenuPositioningItem:atLocation:inView:`) over `window` at `at` `#{ x, y }` (the content's logical pixels): `items` `[#{ id, label, keys?, enabled?, checked?, submenu? } or "separator"]`, keys shown as the menu bar shows them. `true` when shown — the window then hears `context` with the chosen item's `id` (`()` when dismissed); `false` on a headless window or a platform without one (`gui.platform().context_menu`): the program draws its own. A script arms a choice with `gui.input(w, #{ kind: "context_pick", label })`: the next menu shows, then closes as though that item was chosen (`picked: true`). ⌃-click arrives as the right button |
| `gui.menu(spec)` | the macOS menu bar: `[#{ title, items: [#{ id, label, keys?, enabled?, checked? } or "separator"] }]`; an item chosen sends `menu` with its `id`. Elsewhere answers `false` — Loom draws the window's menu |

`gui.flatten`, the `gui.flat_` family (`flat_emit`, `flat_join`,
`flat_diff`, `flat_index`, `flat_arrays`, `flat_keep`, `flat_geo`), and `gui.roles`
are Loom's layout and diff internals; they are not part of the surface
a program uses.

**The patch.** Each operation is a map; `op` defaults to `"node"`:

| `op` | Fields |
|---|---|
| `"node"` | insert or replace: `key`, `parent` (none for the window's one root; a parent comes before its children), `index` among the siblings, `role`, `box` `(x, y, w, h)` in logical pixels **relative to the parent's content origin**, `style`, `text`, `name`, `description`, `focusable`, `disabled`, `checked`, `selected`, `expanded`, `value`, `range` `(value, min, max)`, `level` (1 the top), `live` (`"polite"`/`"assertive"`; `"off"` takes a `status`'s or a `log`'s politeness back), `pos` `(index, count)` (its place in its set: a virtualized list's, table's or tree's row, counted from 0; without it a set's items are counted from the tree), `scroll` (the node scrolls its children; the wheel moves it without the program, and says so with `scrolled` — each axis goes to the nearest scroller that can still move along it, so two fingers sideways over a list that scrolls down move a region around it across, and shift turns a mouse wheel sideways; bringing the focus into view says `scrolled` too; a focused `region` that scrolls is moved by the arrows, page keys, space, home and end too, and the key goes on to the program only at its edge), `content` `(w, h)` (what a scrolling node scrolls through when its laid-out children reach less: a virtualized list), `modal` (a dialog, popover, or menu: while one is in the tree the pointer and Tab stay inside the topmost, the focus moves into it, and a press outside sends `outside`), `grab` (the pointer finds the node before its siblings, wherever it sits among them: a divider whose target reaches over the panes beside it; reading and focus keep its place), `draw` (a canvas: a list of operations, below), `wheel` (it hears the wheel and the pinch itself: below), `wheel_x` (it hears the wheel's sideways part only: below), `active` (the key of the node said as its focus: below), `inert` (drawn and said, never under the pointer), `image` (a picture's path or Bytes: below), `fit` (`"contain"`, the default, `"cover"`, `"fill"`, or `"none"`: its own size, below), `edit` (below) |
| `"remove"` | `key`: the node and its subtree |
| `"focus"` | `key`, or none to clear |
| `"scroll"` | `key`, `to` `(x, y)` |
| `"clear"` | everything |

Roles: `window group text heading image button checkbox radio switch
slider input textarea search list listitem table row cell columnheader
tree treeitem tablist tab tabpanel dialog menu menuitem tooltip status
log progressbar region link separator figure`, the landmarks
`navigation main banner complementary contentinfo form` (a `region` is
one only when it has a `name`), and `grid rowheader rowgroup spinbutton
meter toolbar radiogroup` (`gui.platform()` says `a11y_depth`). A role
decides what a pointer or Enter/Space activates and what takes focus by
default, and what the screen reader is told: a table says its rows and
columns (a row whose cells are all column headers is its header, not
one of its rows), a tree's item its level and its place among its
siblings, a slider or spinbutton steps (`a11y` `increment`/`decrement`),
what has `expanded` opens and closes (`expand`/`collapse`), a scroller
scrolls a page the ways it can still go (the engine scrolls, and says
`scrolled`). On macOS VoiceOver's rotor finds headings, links,
landmarks, form controls and tables (`AXUIElementsForSearchPredicate`),
a live node's changed value is announced, and a plain field's text is
text runs over its box, so Zoom follows its caret.

**Canvases.** A node's `draw` is a list of operations in the node's own
logical pixels, drawn in order, each an `op` with its fields: `"rect"`
(`x`, `y`, `w`, `h`, `radius`, one or four; `gradient`, a list of
colours, across or with `dir: "y"` down), `"line"` (`x1`, `y1`, `x2`,
`y2`), `"circle"` (`cx`, `cy`, `r`), `"path"` (`points`, a list of `(x,
y)`; `close`), `"text"` (`x`, `y` the top of the line, `text`, `size`,
`weight`, `align` `"start"`/`"center"`/`"end"` about `x`, `max_w`: cut
with an ellipsis to that width), `"image"` (`src`, a picture's path or
Bytes, `x`, `y`, `w`, `h`, `fit`), and `"push"` (`clip` `(x, y, w, h)`,
`translate` `(x, y)`, `scale` `(kx, ky)` or one number) and `"pop"`, a
stack of clips and transforms. A transform moves and sizes geometry,
never a stroke's width, a corner's radius, or a text's size (a timeline
zooms its days, its labels stay readable). A shape takes `fill`,
`stroke` and `width`; a line's colour is `color` (or `stroke`).
Rectangles, lines along an axis, and text become the window's own
primitives, so a canvas is drawn like as many boxes, and a program draws
what is visible each frame. Paths, circles, and slanted lines are
rasterized on their own bounding box at the window's scale, and kept by
their shape there (the same arrowhead anywhere, or moved by whole
pixels, is one raster); a gradient is a strip of its stops stretched
over its rectangle. An operation with a `hit` value is a hit region: a
`pointer` event over the canvas carries the topmost one under it as
`hit`. A canvas with `wheel: true` hears the wheel and a trackpad's pinch
itself (`wheel`, `pinch` events) before any scrolling node around it; one
with `wheel_x: true` hears only the sideways part (a `wheel` event with
`dy` 0; shift turns a mouse's wheel sideways first) and the rest scrolls
what is around it — lines in a list that scroll across together, when
`gui.platform()` says `wheel_x`. Give
a canvas a `name` (role `figure`) so a screen reader can say what it
shows, and `active: <key>` to have the element `key` (one placed over its
focused shape) said as its focus: its active descendant. A canvas text
may say `italic: true`. A focusable node with `input: true` takes the
keyboard as it is (a terminal): Tab is its own rather than the focus's,
and the input method is allowed while it has the focus — its composed
text arrives as `preedit` events and its committed text as `text`
events, the candidates placed at `ime_area` `(x, y, w, h)` (its own
pixels: the cursor's cell). Ask `gui.platform().terminal` before
using either key.

**Pictures.** An `image` node's source is a file's path or its Bytes:
PNG, JPEG (turned as its EXIF orientation says), WebP, GIF, or SVG
(shapes; text in an SVG is not drawn). A real window
decodes on a decoding thread of the engine's, draws without the picture
until it is ready, then draws again: the window's thread never waits on
a decode. A headless window decodes at once. A picture is kept at the
size it shows — a photo as a thumbnail is held scaled down to the power
of two above the thumbnail's size in display pixels — in a cache of
384 MB of pixels by source and size (a path with its length and
modification time, so a file written again is read again). `fit:
"none"` draws it at its own size from the box's top left, one image
pixel to one display pixel (sharp on every display: on a 2× display
it is half as many logical pixels), an SVG's units as logical pixels
drawn at the display's scale. A picture of more than 100 million pixels
is refused, and one is kept at most 8192 pixels a side.

**Animations.** An animated GIF or WebP shows its first frame, then
plays once its frames are decoded (on the decoding thread; at once in a
headless window): each composited as the format says (a GIF's disposal,
WebP's blending), kept at the size it shows, with its delay (under 20 ms
is 100 ms, as browsers do) and its loop count (a GIF with no loop block
plays once; it rests on its last frame). An animation of more than 48 MB
of frames at that size, or of more than 1,000 frames, stays on its first
frame. The window draws a frame only when one is due and only for a
picture in view — not scrolled away or clipped, the window neither
covered, minimized, nor hidden. Under Reduce Motion it stands on its
first frame with a play button over it (on a picture at least 50 pixels
a side): a press, or an assistive press, plays it, and pauses it again;
neither the press nor its release reaches the program. A headless
window's clock stands still until `gui.input(w, #{ kind: "clock", ms })`
moves it; `gui.read(w, "animation", key)` answers `#{ frames, frame,
paused, button }` and `gui.read(w, "next_frame")` when the next frame
is due.

**Styles.** `bg`, `border`, `border_width`, `radius` (one or four,
top-left first), `color`, `font` (`"body"`, `"mono"`, or a family list),
`size`, `weight`, `italic`, `line_height`, `align` (`start` and `end` follow the text's direction; `left`, `center`, `right` do not), `valign`, `wrap`,
`pad` (one, two, or four), `clip` (children are clipped to the inside of
the border, following its corners), `opacity` (the node and everything
in it — text, boxes, a canvas's drawing, a picture — faded together),
`caret`, `selection`,
`placeholder_color`, `focus_ring` (a colour, or `false`), and the
variants `hover`, `pressed`, `focus` — style maps the engine applies as
the pointer and focus move, with no turn of the program. `group_hover:
true` applies a node's `hover` look while the pointer is over an
ancestor too (a tab's close button shown while the pointer is on the
tab).

**Shapes.** A style's `corner: "continuous"` draws Apple's continuous
corners (a superellipse reaching 1.528 × the radius along each edge, as
deep at its diagonal as a quarter circle; a capsule's ends stay round)
on its fill, border, focus ring and the clip of its children; `"round"`
quarter circles; unsaid, the window's (`gui.set`'s `corner`). A style's
`shadow: #{ color, blur, x, y, spread }` draws the exact Gaussian blur
of the box's shape under it (`blur` CSS's: twice the sigma). A style's
`material: "sidebar" | "popover" | "sheet" | "hud"` draws what is under
the box blurred, its own (translucent) `bg` over it — on the GPU; the
software renderer and a window under Reduce Transparency draw the `bg`
opaque. Boxes, borders and canvases sit on whole device pixels (a moving
one, mid-transition, glides). A canvas's `path` takes `points` or `d`
(`("M", x, y)`, `("L", x, y)`, `("Q", cx, cy, x, y)`, `("C", …)`,
`("Z")`) and `rule: "evenodd"`; a stroke `cap`, `join`, `dash`,
`dash_offset`; a gradient (mixed in linear light) `dir: "radial"`.

**Transitions.** A style's `transition` — `#{ duration (ms, 200), delay,
curve, bounce, animate, enter, exit }` — has the engine move the node
between frames at the display's refresh, with no turn of the program:
`animate` names what eases when a frame changes it (`"opacity"`,
`"box"` — its place and size, its children with it —, `"color"` — its
background, border and text —, or `"hover"`: its colours only as the
pointer comes and goes; the first three by default); `curve` is
`"ease"`, `"ease_in"`, `"ease_out"`, `"ease_in_out"`, `"linear"` or
`"spring"` (`duration` its response, `bounce` 0 … 0.9; retargeted
mid-way it keeps its velocity). `enter` and `exit` are poses —
`#{ opacity, x, y, scale }` — a node a patch adds comes in from (not in
a window's first frame), and one a patch removes is drawn leaving to,
from what it last showed, over everything, found by neither the pointer
nor assistive output. Only the drawing moves: hit tests, `bounds` and
the accessibility tree are the new frame's at once. Reduce Motion makes
every transition instant, as does a headless window until
`gui.set(w, #{ motion: true })` (then `"clock"` moves them;
`gui.read(w, "motion")` → `#{ moving, enabled, nodes, leaving }`).
`gui.platform()` says `transitions`.

**Scroll bars.** Whatever scrolls further than it shows (a scrolling
node, a styled field) has an overlay bar at its right edge, as the Mac's:
a thin thumb shown while the view moves and for a moment after, faded
(at once under Reduce Motion); the pointer over it while it shows
widens it on a quiet track; the thumb drags the view, a press on the
track pages it (neither reaches the program, which hears `scrolled` or
`viewport` as from the wheel). It stays a second after the view stops
and fades over 250 ms, as an NSScroller's knob. Scrolling itself is
AppKit's: a trackpad's gesture pulls a scrolling node or a styled field
past its ends against a rubber band, springing back as the fingers
lift; its momentum runs to an end and bounces there; a mouse wheel's
notch glides over 160 ms. The program hears offsets inside the view
only. `gui.input`'s `"wheel"` takes `phase` (`began`, `changed`,
`ended`, `momentum_began`, `momentum`, `momentum_ended`) and `precise`
(`false`: a mouse wheel's notch) to play them. A styled field's bar lies in its right
padding and the pointer there finds it whenever it scrolls; a scrolling
node's lies over its content and is found only while it shows. Its
colour is black or white over the background under it.
`gui.platform()` says `scroll_bars` and `group_hover`. Colours are
`"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, or `(r, g, b[, a])`. A focusable
node with no `focus` variant gets a focus ring drawn by the engine.

**Text fields.** A node with `edit: #{ value, placeholder?, multiline?,
secure?, rev? }` is edited by the engine: movement by grapheme, word
(option on macOS, ctrl elsewhere), and line; selection by keyboard and
pointer (double-click a word, triple-click a line); copy, cut, paste,
select all, undo, redo; and the platform's input method, its
composition drawn in place. Each committed edit sends `changed` with the
value, the selection (character offsets), and a revision. The program's
value is the authority: a patch whose value differs is adopted — unless
it is the field's own earlier value at an earlier `rev`, which is the
program catching up, not a reset. Send back the `rev` of the last
`changed` you applied.

**A styled field** (`rich: true`, a `textarea`) is laid out a paragraph
(a hard line) at a time: `spans` a list a line of `(start, end, style)`,
`styles` the looks (`weight`, `italic`, `font`, `size`, `color`,
`underline` (`wavy` a problem's wave), `strike`, `bg`, `line_bg`,
`ligatures` (`false`: the font's ligatures off in the span, a string's
`->` said as typed)), `select` `(anchor, focus, seq)`
the program's selection, `undo: false` (⌘Z is the program's), `report`
(it says `select` as the selection moves). Only the paragraphs shown,
and the caret's, are laid out (the rest are estimated until they show),
so a 50,000-line text opens and edits at the cost of a page. For a code
editor it also takes `gutter` `#{ numbers, color, current, line_bg,
marks, changes }` — line numbers (the caret's in `current`) and marks `(line,
glyph, colour)` drawn in the field's left padding, the caret's line
behind in `line_bg`, and `changes` `(line, kind, colour)` (`kind`
`"added"`, `"modified"`, `"removed"`: a line's change against a version
kept elsewhere, a repository's index) in a 3-pixel lane between the
numbers and the text — a bar down a changed line, a tick at the edge a
removal came out of; a press in the lane is a `lens` event with `id`
`"change"` on that line, the caret left where it was — `decorations` `[(line, start, end, style)]`
(characters within a line: a style's `bg` behind, its `underline` under,
without restyling the text: a matching bracket, the find's matches, a
problem), `scroll_to` `(line, seq)` (that line first in view, when
`seq` is new), `track` `[(line, colour)]` (marks on its scroll bar's
track, shown whether or not the thumb is, with the caret's line), and
`past_end` (the share of a view it scrolls past its last line). A
selection over lines is drawn as one shape: each row it covers whole,
out to the right edge. Its `changed` adds `delta` `#{ first, removed, lines, at,
old_len, inserted }` (what changed by line and by character), `caret`,
`top` (the first line in view) and `place` `(anchor line, column, focus
line, column)` (the selection by line, characters within it); `select`
adds `caret`, `top`, `place`, and `click_mod` (the press was made holding the command key: ⌘-click). The
value it reports is shared with the program, not copied, and a value
handed back is known as the field's own earlier one by its identity.

**Events** are maps with a `kind` and the `window` they came from:

| `kind` | Fields |
|---|---|
| `"resize"` | `width`, `height` (logical pixels), `scale` |
| `"close_requested"` | — the window stays until `gui.close` |
| `"key"` | `key` (`"a"`, `"enter"`, `"left"`, `"f5"`), `text`, `chord` (`"ctrl+shift+p"`, `"super+s"`), `mod_chord` (the platform's command key as `mod`: `"mod+s"` on every platform), `mods`, `repeat`, `target` (the focused key). A key a text field or the focus used is not sent |
| `"activate"` | `key`, `source` (`"pointer"`, `"key"`, `"a11y"`) — a click, Enter or Space on a focused control, or an assistive press |
| `"focus"` | `key` (or `()`) — focus moved (Tab, a click, an assistive action, or a `"focus"` op) |
| `"changed"` | `key`, `value`, `selection`, `rev` |
| `"submit"` | `key` — Enter in a single-line field |
| `"text"`, `"preedit"` | `key` (a node with `input: true`), `text` (committed; being composed, with `cursor`) — the input method's |
| `"pointer"` | `action`, `x`, `y`, `button`, `clicks`, `mods` (the modifier keys held: a ⌘-click), `target`, `hit` (over a canvas: its topmost hit region there), `sx`, `sy` (the place on the screen, what follows a drag into another window; `()` where a window cannot learn where it is — Wayland, `gui.platform().window_position`) |
| `"moved"` | `x`, `y` — the window's content moved on the screen (never sent on Wayland) |
| `"wheel"` | `key` (a node with `wheel: true` under the pointer), `dx`, `dy` (logical pixels; positive moves the view up and left), `x`, `y`, `mod`, `shift`, `alt`, `ctrl` — the wheel or a trackpad's two fingers |
| `"pinch"` | `key` (as `wheel`), `delta` (the change of magnification; positive zooms in), `phase` (`"start"`, `"move"`, `"end"`, `"cancel"`), `x`, `y` — a trackpad's pinch (macOS) |
| `"scrolled"` | `key`, `x`, `y` |
| `"viewport"` | `key`, `top` — a styled field's first line in view moved into another band of 16 lines (the wheel) |
| `"select"` | `key`, `selection`, `place`, `rev`, `caret`, `top`, `click_mod` — a styled field's selection moved (`report`) |
| `"hover"` | `key` (a styled field with `hover: true`), `line` and `column` (1-based, the character the pointer rests on; both `()` when it left the text, `column` `()` and `gutter: true` over the gutter), `rect` |
| `"lens"` | `key`, `line` (1-based: the line the lens sits under), `id`, `hit`, `x`, `y`, `clicks` — a press in a code field's lens (a lens is `#{ line, height, ops, id }`, `line` 1-based); `id` `"sticky"` a pinned scope's line, `"change"` the gutter's changes' lane |
| `"a11y"` | `key`, `action` (`"set_value"`, `"increment"`, `"decrement"`, `"expand"`, `"collapse"`, `"custom"` with `index` and `label`), `value` |
| `"outside"` | `key` (the modal layer), `x`, `y` — a press outside the topmost modal layer |
| `"window_focus"` | `on` |
| `"appearance"` | `dark`, `contrast`, `reduce_motion`, `reduce_transparency`, `accent` (`"#rrggbb"` or `()`), `why` (`"open"`, `"focus"` — a change missed while the window was behind —, `"theme"`, `"settings"`, `"test"`) |
| `"menu"` | `id` |
| `"open"` | `paths` — no `window`: the application was asked to open files and folders (macOS: a document opened with it in the Finder, dropped on its Dock icon, `open -a`; AppKit's `application:openURLs:`). Sent as soon as the platform's loop starts, so a program that starts the loop (`gui.prepare`) hears the documents it was launched with |
| `"new_tab"` | — no `window`: a tab bar's + button (macOS `newWindowForTab:`) |
| `"quit"` | `why` (`"quit"`, `"logout"`, `"restart"`, `"shutdown"`) — no `window`: a quit from outside the program, held for it by `gui.hold_quit(true)`; the application ends when the program does |
| `"tabs"` | `tabs` — the window ids shown as tabs with this window, in the bar's order (itself alone without a bar); sent when a window comes forward and after a `tab` set (macOS) |
| `"context"` | a context menu closed (`gui.context_menu`): `id` (the item chosen, or `()`), `items` (the titles as the native menu held them, `"-"` a separator), `picked` (a script's `context_pick` chose it) |
| `"files"` | `action` (`"hover"`: files from another program are over the window; `"drop"`: they were dropped; `"cancel"`: the hover went away), `paths`, `x`, `y` (where: on macOS the pointer's place as the files arrive; elsewhere the pointer's last place in the window — `gui.platform().drop_position`). Not sent on Linux: winit has no Wayland drag and drop (`gui.platform().file_drop`), `target` (the node there). The platform sends a file at a time; the engine sends the files of one gesture as one event |

```olang no-run
let w = unwrap(gui.open(#{ "title": "Hello", "size": (360, 160) }))
let events = gui.events()
fn view(name) = [
    #{ "key": "root", "box": (0, 0, 360, 160), "style": #{ "bg": "#ffffff" } },
    #{ "key": "name", "parent": "root", "role": "input", "name": "Name", "box": (24, 24, 312, 36),
       "edit": #{ "value": name, "placeholder": "Your name" },
       "style": #{ "border": "#d4d4d8", "border_width": 1, "radius": 6, "pad": (0, 10, 0, 10) } },
    #{ "key": "hi", "parent": "root", "role": "status", "box": (24, 80, 312, 20),
       "text": if name == "" => "" else => "Hello, " + name }]
gui.apply(w, view(""))
gui.apply(w, [#{ "op": "focus", "key": "name" }])
let mut going = true
while going {
    let e = unwrap(chan.recv(events))
    let kind = e.kind
    if kind == "changed" => { gui.apply(w, view(e.value)); () }
    else if kind == "close_requested" => { going = false }
}
gui.close(w)
```

**The main thread.** macOS requires the platform's event loop on a
process's first thread. olang runs every program on a thread of its
own, so the first thread is free: the first `gui.open` starts the loop
there, and a program that never opens a window never starts one. When
the program ends — finishing, raising, `os.exit`, or a signal — its
windows go with it. While a window is open, a program waiting on
`gui.events()` is not reported as a deadlock: input can arrive.

**Rendering.** The GPU renderer (wgpu: Metal, Direct3D 12, Vulkan) is
the default; the software renderer (tiny-skia) is the reference it is
checked against (`tests/gui_test.rs`), the fallback where no GPU adapter
works, and what headless windows draw with. Both blend in sRGB-encoded
space and draw glyphs from the same rasterizer, so they agree to within
antialiasing. Text is shaped by parley — complex scripts, bidi, font
fallback, dictionary word breaks for Chinese, Japanese, and Thai.

A window keeps its last frame and draws only what changed: each frame's
primitives are hashed into 64-pixel tiles, the tiles whose primitives
changed are redrawn over the kept frame, a scroll region scrolled by
whole pixels moves the kept pixels with it (only what it uncovers is
drawn), and a frame that changes no pixel is not presented
(`tests/gui_damage_test.rs` holds damaged frames to whole ones).
`OLANG_GUI_FULL_REDRAW=1` draws every frame whole; `OLANG_GUI_GPU_TIME=1`
says each frame's GPU time and the share it drew. `gui.read(_, "render")`
(any first argument) answers the frames drawn and left unchanged, the
pixels drawn, and the bytes the glyph atlas (up to four 2048² pages,
the least recently drawn emptied), pictures' textures, kept frames and
the glyph and shape caches hold.

**Platforms.** Run on macOS (the GPU renderer, the menu bar, the
accessibility tree as NSAccessibility). Linux is Wayland only: with no
`WAYLAND_DISPLAY`, `gui.open` answers `Err`. Windows is compiled, not
yet run.

## `http` — HTTP

Both sides of HTTP in one module: a client for calling APIs, and
`http.serve`, a blocking HTTP/1.1 server with keep-alive and a bounded
worker pool. The design leans on the rest of the language rather
than inventing its own idioms: requests are fallible so they return
`Result`, a response is an ordinary struct-like value, and a server
handler is just a function from request to response. (`no-run`: network.)

A client response (an `HttpResponse`, the shape `http.response` builds)
carries `status` (Int), `body` (String, or Bytes with `"bytes": true`),
`headers` (a map of lowercased header name to its value, a list of the
values for a header sent more than once, as `set-cookie` is), and
`success` (Bool, true for 2xx):

| Function | Description |
|---|---|
| `http.get(url[, opts])` / `http.post(url, body[, opts])` / `http.put` / `http.delete` | requests |
| `http.request(method, url, body[, opts])` | any method |
| `http.parse_url(url)` | split a URL into parts: `Ok({ scheme, host, port, path, query, fragment })`, `port`, `query` and `fragment` `()` when the URL names none |
| `http.encode_query(map)` / `http.decode_query(s)` | query strings — `encode_query` takes a `#{}` map or a `{ }` record, keys in its order, each value a string, a number, a Bool or a list of them (the name repeated for each); anything nested raises. `decode_query` answers a map: a name given once its string, a name given more than once a list of its strings in order (`"t=a&t=b"` is `#{ "t": ["a", "b"] }`) |
| `http.serve(port, handler[, options])` | serve `handler(request)` on a bounded worker pool; blocks the calling program |
| `http.response(status, body)` / `http.response_with_headers(status, body, headers)` | build responses: an `HttpResponse` of the client's shape, `{ status, body, headers, success }`, `headers` a map |
| `http.shutdown()` | ask every running `http.serve` to drain: no new connections, in-flight requests finish (up to `drain_ms`), and `serve` returns `Ok` — what an [`os.on_shutdown`](#os--operating-system) handler calls |
| `http.defer()` / `http.respond(ticket, response)` | park the connection a handler is answering and complete it later, from any thread |
| `http.hold(topic)` / `http.notify(topic, response)` / `http.notify_all(response)` / `http.held(topic)` | park the current request under a topic; answer every connection held under it (the count answered); count what is parked |

Every client verb takes an optional trailing **options map** — this is
how a request carries headers, a timeout, and authentication:

| Option | Meaning |
|---|---|
| `"headers"` | map of header name → string value |
| `"timeout_ms"` | whole-request timeout in milliseconds |
| `"bearer"` | sets `Authorization: Bearer <token>` |
| `"basic"` | `(user, password)` tuple: HTTP basic auth |
| `"bytes"` | `true`: the response `body` is `Bytes`, not text (an image, an archive) |

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
| `req.query` | map of decoded query parameters (a name given more than once: a list of its values) |
| `req.headers` | map of headers, keys lowercased |
| `req.body` | the request body as text (bytes that are not UTF-8 read as U+FFFD) |
| `req.body_bytes` | the request body as `Bytes`, exactly as sent (an upload, an image) |
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
| `trust_proxy` | `false` — when true, `req.remote_addr` is the first entry of `X-Forwarded-For` (a header anyone can send, so only behind a proxy you control) |
| `drain_ms` | `5000` — how long a drain (`http.shutdown()`) waits for the requests in flight before the rest are cut. While draining, every response says `Connection: close` and no further request is read off a kept-alive connection |

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
    if req.path == "/hello" => "hi, " + (req.query.name ?? "anonymous")
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
in-process database, so the examples below run in the test suite.

| Function | Description |
|---|---|
| `db.open(path)` / `db.open(path, opts)` | open or create (`":memory:"` for in-memory); options below |
| `db.execute(conn, sql)` / `db.execute(conn, sql, params)` | run a statement; `?` placeholders — `Ok(rows changed)` |
| `db.query(conn, sql)` / with `params` | `Ok` of the rows, as a list of maps |
| `db.query_one(conn, sql)` / with `params` | `Ok(row)`, the first row as a map, or `Ok(())` when there is none (unambiguous, since a row is always a map) |
| `db.query_rows(conn, sql)` / with `params` | the columns and the rows as SQL has them: `#{ "columns": [#{ "name", "decltype", "index" }], "rows": [[value, …]] }` |
| `db.query_frame(conn, sql, params?, timeout_ms:)` | `Ok` of the rows as a Frame: a column per result column, typed by its values (Int, Float, String; Int and Float together a Float column), a column declared `DATE` a Date and `DATETIME`/`TIMESTAMP` a Datetime (ISO text, or a Datetime's epoch seconds), `BOOLEAN` a Bool; NULL is null. A repeated name gets a suffix (`a`, `a_2`) |
| `db.insert_frame(conn, table, frame, create = true, replace = false)` | every row of a Frame, in one transaction (or the open one): with `create`, the table is made first if missing, its column types from the Frame's (`INTEGER`, `REAL`, `TEXT`, `BOOLEAN`, `DATE`, `DATETIME`); `replace: true` drops it first. Dates and datetimes are written as ISO text, a Category as its level, a null or NaN as NULL — `Ok(rows inserted)`, and `db.query_frame` reads the same Frame back |
| `db.cursor(conn, sql)` / with `params` | a statement to read a piece at a time — `Ok(cursor)` |
| `db.next(cur, n)` | up to `n` more rows of a cursor, each a list; `[]` at the end |
| `db.columns(cur)` | a cursor's columns, as `db.query_rows` has them |
| `db.close_cursor(cur)` | finish a cursor now |
| `db.interrupt(conn)` | stop the statement the connection is running, from any task |
| `db.transaction(conn, f)` | `f(conn)` inside a transaction: commit unless `f` returns an `Err` or raises, which roll back; `f`'s result is handed through. Transactions on one connection are serialized across threads — a second `db.transaction` waits for the first to end — so the http workers of a served app may share a connection; plain statements from another thread do not wait, and join whatever transaction is open |
| `db.migrate(conn, steps)` | bring the schema to the head of `steps` (a list of versions, each a list of SQL); a `schema_version` table records progress, each version runs in its own transaction, a failing statement is named — `Ok(version)` |
| `db.begin(conn)` / `db.commit(conn)` / `db.rollback(conn)` | transactions by hand |
| `db.close(conn)` | close the handle (its open cursors are finished first); a connection no value holds any more closes by itself once its cursors are dropped |

Values: NULL is `()`, INTEGER an `Int`, REAL a `Float`, TEXT a `String`.
A BLOB is `Bytes` from `db.query_rows` and `db.next`; `db.query` and
`db.query_one` answer it as a `String` (invalid UTF-8 replaced), as they
always have. A `Bool` binds as `1`/`0`, `Bytes` as a BLOB.

```olang
let conn = unwrap(db.open(":memory:"))
unwrap(db.execute(conn, "CREATE TABLE scores (name TEXT, points INTEGER)"))
unwrap(db.begin(conn))
unwrap(db.execute(conn, "INSERT INTO scores VALUES (?, ?)", ["ada", 99]))
unwrap(db.execute(conn, "INSERT INTO scores VALUES (?, ?)", ["bob", 82]))
unwrap(db.commit(conn))

let rows = unwrap(db.query(conn, "SELECT name, points FROM scores ORDER BY points DESC"))
for row in rows {
    println(`${row.name}: ${row.points}`)
}
unwrap(db.close(conn))
```

**Frames.** `db.query_frame` reads a result straight into typed
columns and `db.insert_frame` writes one back: no list of maps between
the database and the data stack.

```olang
let c = unwrap(db.open(":memory:"))
let sales = ods.frame({
    day: ods.series(["2026-01-05", "2026-01-06", "2026-01-07"]).to_date(),
    store: ["north", "south", "north"],
    revenue: [1200.5, 980.0, 1410.25],
})
println(unwrap(db.insert_frame(c, "sales", sales)))        // 3
let back = unwrap(db.query_frame(c, "SELECT * FROM sales WHERE revenue > ?", [1000.0]))
println(back["day"].max())                                  // 2026-01-07
println(back["revenue"].sum())                              // 2610.75
unwrap(db.close(c))
```

**Opening.** `db.open(path, #{ "readonly": true, "create": false })` —
both optional, and the defaults are read-write and create:

- `readonly: true` opens with `SQLITE_OPEN_READ_ONLY`: SQLite refuses
  every write (`Err("db.execute: attempt to write a readonly
  database")`), and a missing file is refused rather than made.
- `create: false` opens read-write but refuses a missing file:
  `Err("db.open: no database at <path> (create: false)")`.
- `readonly: true` with `create: true` is an `Err`.

A SQLite URI filename is read as a URI, so its parameters work too:
`db.open("file:/data/app.db?mode=ro")` (also `mode=rw`, `mode=rwc`,
`mode=memory`, `cache=shared`, `immutable=1`). Escape `%`, `?`, and `#`
in the path as `%25`, `%3F`, and `%23`. Under a capability manifest, a
read-only open (either way) needs `fs = "read"`; any other file open
needs full `fs`.

**Rows as SQL has them.** A `db.query` row is a map: its keys come in
the SELECT's order (a map keeps its keys' order), but two columns of one
name collapse into one, and a result with no rows says nothing about
its columns. `db.query_rows` keeps all
of it: the columns in order, each with its declared type (`decltype`,
`()` for an expression), repeated names kept, known with zero rows; the
rows as lists in the same order.

```olang
let c = unwrap(db.open(":memory:"))
unwrap(db.execute(c, "CREATE TABLE p (name TEXT, photo BLOB)"))
unwrap(db.execute(c, "INSERT INTO p VALUES (?, ?)", ["ann", bytes.from_list([0, 255])]))
let r = unwrap(db.query_rows(c, "SELECT name, photo, 1 AS name FROM p"))
assert_eq(map(r.columns, (k) => k.name), ["name", "photo", "name"])
assert_eq(map(r.columns, (k) => k.decltype), ["TEXT", "BLOB", ()])
assert_eq(r.rows, [["ann", bytes.from_list([0, 255]), 1]])
unwrap(db.close(c))
```

**Cursors.** `db.query` and `db.query_rows` read a whole result before
they answer. `db.cursor(conn, sql, params?, opts?)` prepares the
statement and reads nothing; each `db.next(cur, n)` steps it for up to
`n` more rows (lists, as `db.query_rows`'s), and answers `[]` once the
rows run out. Reading the first screen of a ten-million-row SELECT
costs that screen, not the rest.

```olang
let c = unwrap(db.open(":memory:"))
let cur = unwrap(db.cursor(c, "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n LIMIT 10000000) SELECT i FROM n"))
assert_eq(map(unwrap(db.columns(cur)), (k) => k.name), ["i"])
assert_eq(unwrap(db.next(cur, 3)), [[1], [2], [3]])
assert_eq(unwrap(db.next(cur, 2)), [[4], [5]])
unwrap(db.close_cursor(cur))
unwrap(db.close(c))
```

A cursor's lifetime:

- It holds its statement until the rows run out (finished then, at
  once), `db.close_cursor(cur)`, the last copy of the handle is dropped
  (finished when the connection is next free — a drop never waits on a
  running statement), or `db.close` of its connection. `db.next` on a
  closed cursor answers `Err("db.next: the cursor is closed")`; after
  `db.close`, `Err("db.next: connection is closed")`. `db.columns` works
  throughout.
- An open cursor is an active read. In SQLite's default
  (rollback-journal) mode it holds a shared lock on the file, so another
  connection's write waits for it (up to the 5 s busy timeout, then
  `Err("… database is locked")`); in WAL mode (`PRAGMA
  journal_mode=WAL`) readers and a writer do not block each other.
  Close a cursor you have finished with.
- The connection stays usable while a cursor is open: other statements
  and other cursors run on it between reads. Writing to the table a
  cursor reads leaves what the cursor reads next undefined (SQLite's
  rule); read to the end, or use a second connection.
- A `db.next` that fails — an SQL error, an interrupt, a timeout —
  finishes the cursor: that call answers the `Err` (the rows it had
  read are dropped), and every later one answers `Err("db.next: the
  cursor failed: <why>")`.
- A cursor may be read from any task; reads of one connection take turns.

**Stopping a statement.** Every statement a connection runs can be
stopped from outside it:

- `db.interrupt(conn)` (or a cursor) stops the statement the connection
  is running at that moment — a `db.query`, `db.execute`,
  `db.query_rows`, or `db.next` on any task. It is meant to be called
  from another task than the one running the statement: it takes no
  lock, so it answers at once, and the stopped call answers
  `Err("db.<fn>: interrupted")` within a few thousand SQLite
  instructions. The connection stays usable, and an interrupt affects
  nothing that starts after the statement it stopped (with nothing
  running, it does nothing). An open cursor between reads is not
  running: close it instead.
- `timeout_ms` bounds one call: `db.query(conn, sql, params, #{
  "timeout_ms": 500 })` answers `Err("db.query: timed out after 500
  ms")` when the statement runs longer. It is taken by `db.execute`,
  `db.query`, `db.query_one`, `db.query_rows`, and `db.cursor` (where
  it bounds each `db.next`). The options map may stand where the params
  would: `db.query(conn, sql, #{ "timeout_ms": 500 })`.

Both work through a progress handler SQLite calls every thousand
virtual-machine instructions, not `sqlite3_interrupt` (whose flag stays
set while any statement on the connection is active — an open cursor
would make one interrupt fail every later statement). A statement
waiting for another connection's lock (the busy timeout) is not running
instructions, so it is not stopped until it gets the lock or gives up.

```olang
let c = unwrap(db.open(":memory:"))
let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n"
let t = spawn { time.sleep(50); db.interrupt(c) }
assert_eq(db.query(c, endless), Err("db.query: interrupted"))
task.join(t)
assert_eq(db.query(c, endless, #{ "timeout_ms": 50 }), Err("db.query: timed out after 50 ms"))
assert_eq(len(unwrap(db.query(c, "SELECT 1"))), 1)
unwrap(db.close(c))
```

**Threads.** Each connection has its own lock, held for one statement
(one `db.next`): tasks with their own connections run side by side, and
tasks sharing a connection take turns. `db.interrupt` is the one call
that never waits for that lock.

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
| `dom.available()` | `true` in the browser, `false` in a native build: the one `dom` function that works everywhere, so code shared by both can skip event wiring rather than raise |
| `dom.query(sel)` | first element matching a CSS selector — an error if none matches |
| `dom.get_text(el)` / `dom.set_text(el, s)` | read / write an element's text content |
| `dom.set_html(el, html)` | replace an element's inner HTML — the render primitive |
| `dom.value(el)` / `dom.set_value(el, s)` | read / write a form control's value |
| `dom.focus(el)` | focus an element; answers whether it holds focus afterwards (a hidden or disabled control does not take it) |
| `dom.active_id()` | the id of the focused element, or `""` — the read side of `focus`, so a repaint can leave an in-progress edit alone |
| `dom.prefers_dark()` | does the page prefer a dark color scheme — what a themed canvas or SVG needs to pick its palette |
| `dom.confirm(message)` | the browser's yes/no dialog, `true` on accept |
| `dom.read_file(el, callback)` | the first file a file input holds, delivered to the callback as `#{ "name", "size", "type", "base64" }` (or `#{ "error": … }`) |
| `dom.on(el, event, handler)` | attach an event handler (see below); additive — binding the same element and event again adds a listener |
| `dom.off(el, event)` | detach every handler `dom.on` attached to `el` for `event` (`"*"`: every event) and release them; answers how many |
| `dom.window()` / `dom.document()` | handles `dom.on` accepts for the events that fire there and do not bubble to the body: `online`, `offline`, `focus`, `blur` on the window (the first two carry `online` in their event map), `visibilitychange` on the document (carries `hidden`) |
| `dom.on_error(handler)` | hear every failed dispatch — a handler that raised, or a trap under one — as `#{ "error", "output", "trap" }`, after the failed dispatch has ended; the session goes on. `window.olangOnError` is the page-side twin |
| `dom.fetch(method, path, body, callback)` | asynchronous HTTP from the page — the callback receives the response text |
| `dom.fetch_json(method, path, body, callback)` | `dom.fetch`, but the callback receives the parsed value directly |
| `dom.request(method, path, body, callback)` | the whole response — `#{ "status", "headers", "body" }`, status `0` with an `"error"` when no server answered — so a handler tells a 404 from a 500 from a network failure; the response also carries `network_ms` (the send to the last byte) and `queue_ms` (the wait for the runtime to be free) |
| `dom.request_with(method, path, body, headers, callback)` | `dom.request` with request headers — a bearer token, another content type |
| `dom.find(selector)` | the first match, or `()` when nothing matches — the lookup for an element that may be absent (`dom.query` raises on a miss, and a raise inside a handler takes the page down) |
| `dom.patch(el, node)` | reconcile the element's children with a `web.html` node tree: the host diffs the data against the live DOM by `data-key`, no markup rendered or parsed; a `memo` subtree whose inputs stand is kept as it is; answers `#{ "missing", "nodes", "bytes", "serialize_ms", "patch_ms" }` — the keys of `keep` markers the page had no element for, and what the repaint cost |
| `dom.morph(el, html)` | the same reconciliation from markup, for code that renders its own HTML |
| `dom.checked(el)` | a checkbox's or radio's state; `()` for an element that has no checked state (a select, a div) |
| `dom.selection(el)` / `dom.set_selection(el, from, to)` | a text control's selection as `(from, to)`, and setting it (focusing the control) — inserting at the cursor is set the value, then place the caret |
| `dom.values(el)` | a `<select multiple>`'s chosen option values |
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
| `dom.storage_get(k)` / `storage_set(k, v)` / `storage_remove(k)` | localStorage (missing keys read as `""`); `storage_set` answers whether the value was stored — `false` for a full quota or disabled storage, never a raise |
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

`dom` is the one browser-only module: in a native build every call but
`dom.available` reports that it needs the wasm build, mirroring how
`fs`, `os`, `proc`, `pty`, `tty`, `gui`, `http`, and `db` are absent
from the browser. The example is therefore `no-run`; it runs on a page,
not in the test harness:

```olang no-run
dom.set_html(dom.query("#list"),
    ["a", "b"] |> map((s) => "<li>" + s + "</li>") |> join(""))
dom.on(dom.query("#list"), "click", (e) => select(e.id))
dom.on(dom.query("#new-title"), "enter", (e) => add_item(e.value))
dom.fetch("GET", "/api/items", "", (resp) => render(unwrap(json.parse(resp))))
dom.on_frame((f) => {
    dom.draw(canvas, [
        #{ "op": "clear", "color": "rgba(11,14,20,0.35)" },
        #{ "op": "circle", "x": 360.0, "y": 240.0, "r": 16.0, "fill": "#f5c542" }
    ])
})
```

The module has its own chapter, **[olang in the browser](wasm.md)**:
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
inspect, collect, and report failures itself, the raw material for a
custom harness. A returned `Err` does nothing on its own; `unwrap` it
(or match it) to act on it. The exception is `testing.snapshot`, which
answers `()` and raises on a mismatch, as a language-level assertion
does.

| Function | Description |
|---|---|
| `testing.assert_eq(a, b)` / `assert_ne(a, b)` | equality checks |
| `testing.assert_true(x)` / `assert_false(x)` | boolean checks (non-boolean input is an `Err`) |
| `testing.assert_ok(r)` / `assert_err(r)` | Result checks |
| `testing.fail(msg)` | unconditional `Err` |
| `testing.snapshot(name, value[, #{ update }])` | the snapshot test: `value` (a String as its text, else its display form) against `__snapshots__/<name>.snap` beside the file under test. The first run writes the file; a later run answers `()` when it matches and raises with both texts when it does not. `OLANG_UPDATE_SNAPSHOTS=1`, `UPDATE_SNAPSHOTS=1`, or the caller's `update: true` rewrites the file instead |
| `testing.snapshot_dir()` / `testing.snapshot_failed(info)` | the `__snapshots__` directory beside the file under test, where a library keeps snapshots of its own kind (Loom's pixels); a library's report that one of its snapshots differs (`info`: `name`, `baseline`, `actual`, `diff`, …), carried by `olang test --format json` and otherwise ignored |
| `testing.test_summary()` / `testing.reset_tests()` | the session's assertion tally: `#{ "passed", "failed", "total" }` over every `assert_*` outcome on this thread; `reset_tests()` zeroes it |

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

The core of the data stack: a **Series** is a typed, null-aware 1-D
column of `Int`, `Float`, `Bool`, `String`, `Date`, `Datetime` or
`Category` over a contiguous native buffer; a **Frame** is a table of
named, equal-length Series. Operators
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
    println(`${rec.region}: ${rec.total}`)
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
| Create | `series(list\|range)` · `try_series(list)` (the Result form: a mixed-type list is `Err`, not a raise) · `zeros(n)` (n float zeros) · `ones(n)` · `full(n, v)` · `full_like(s, v)` · `linspace(a, b, n)` (n evenly spaced floats, inclusive) · `arange(stop)` / `arange(start, stop, step = 1)` (Int from Ints, else Float; `stop` excluded) · `repeat(s, k)` (each element `k` times, or a list of counts) · `tile(s, k)` (the column `k` times) |
| Math | `abs` `exp` `exp2` `expm1` `ln` `ln1p` `log2` `log10` `sqrt` `cbrt` `sin` `cos` `tan` `asin` `acos` `atan` `sinh` `cosh` `tanh` `sigmoid` `logit` `sign` `floor` `ceil` `trunc` — each `ods.exp(s)`, the scalar `math.*` function over the column (Int widens to Float, except where the answer is whole) · `pow(s, p)` · `round(s, digits = 0)` · `map(s, "sin")` — the older spelling, by name |
| Conditionals | `where(mask, a, b)` (null mask → null) · `select(masks, values, default = ())` (first mask that holds) · `clip(s, lo, hi = ())` (`()` an open bound) · `between(s, lo, hi, closed = "both")` or `between(s, [lo, hi] \| ())` — `"left"`, `"right"`, `"neither"` |
| Comparisons | `eq(a, b)` · `ne(a, b)` — elementwise masks between two Series (against a *scalar*, use the `s == v` / `s != v` operators) |
| Nulls | `is_null(s)` · `not_null(s)` · `is_nan(s)` (masks) · `fill_null(s, v)` — `v` a value, a Series, or a method `"ffill"`, `"bfill"`, `"linear"`, `"mean"`, `"median"` (the first three take a `limit`) · `coalesce(a, b, …)` (first non-null) · `interpolate(s, limit = ())` · `null_count(s)` |
| Reductions | `sum` `mean` `var` `std` `min` `max` `median` (skip nulls; `var`/`std` are sample; a Bool mask sums to its count of true) · `quantile(s, q)` · `count(s)` (non-null) · `first(s)` · `last(s)` · `mode(s)` · `any(mask)` · `all(mask)` · `cumsum(s)` · `cumprod(s)` · `dot(a, b)` — a reduction of an expression reads its columns once |
| Positions | `argmax(s)` · `argmin(s)` (first row of the extreme, `()` if none) · `searchsorted(s, v, side = "left")` |
| Windows | `shift(s, by)` (nulls at the edge) · `diff(s, lag = 1)` · `pct_change(s, lag = 1)` · `cum_max` · `cum_min` · `rank(s, method = "min")` — min/max/average/ordinal/dense · `rolling(s, window, agg, center = false, min_periods = (), exact = false)` — `agg` one of count, sum, mean, min, max, median, std, var — null until the window fills (`min_periods:` any window with that many values); O(n) running sums, `exact: true` a fresh sum per window · `runs(s)` — a Frame of each run's `start`, `end`, `length` (a Bool mask: its runs of true) · `run_id(s \| [s, …])` — a run number per row, one more at each change |
| Recode | `recode(s, mapping, default = keep)` — through a map, an object or `[from, to]` pairs; `()` as the default nulls the unmapped |
| Text | `a + b` on String Series, or a String Series and a String on either side — elementwise concatenation, a null cell staying null · `lower` `upper` `trim` `trim_start` `trim_end` · `replace(s, from, to)` · `replace_re(s, pattern, to)` · `contains(s, text, case = true)` · `starts_with(s, text)` · `ends_with(s, text)` · `len_chars(s)` · `substring(s, start, end = ())` · `split_n(s, sep, i)` (piece `i`, negative from the end; null where there is none) · `pad_start(s, width, fill = " ")` · `pad_end(s, width, fill = " ")` · `to_int(s)` · `to_float(s)` (null where a cell does not read) · `to_date(s, fmt = ())` · `to_datetime(s, fmt = ())` (ISO 8601 without a format, `strptime`'s `%` directives with one; null where a cell does not read) — each over the column's text with no `String` a row, a Category's once per level |
| Head / tail | `head(s, n)` · `tail(s, n)` — the first or last `n` values (the Frame verbs, on a Series) |
| Distinct | `unique(s)` (first-seen order) · `n_unique(s)` · `value_counts(s)` → a value/count Frame, most frequent first |
| Dates | on a `Date` or `Datetime` column: `year` `quarter` `month` `day` `weekday` (ISO: 1 Monday … 7 Sunday — `dates.weekday` of one Date is 0 Sunday) `day_of_year` `week` (ISO) `hour` `minute` `second` `is_weekend` · `date(s)` (a Datetime's day) · `day_name(s)` · `month_name(s)` (Categories in calendar order) · `floor(s, "1h"\|"1d"\|"1w"\|"1mo"\|"1q"\|"1y"…)` · `add_days(s, n)` · `add_months(s, n)` (the day clamped to the month's end) · `months_between(a, b)` (calendar months from `b` to `a`) · `format(s, fmt)` — and `d + n`, `d - n` (days), `d - d` (an Int of days), `t - t` (Float seconds), comparisons with a Date, a Datetime or ISO text · `date_part(s, "year"\|"month"\|"day")` — the prefix of an ISO date or timestamp String Series, as a String Series |
| Categories | `to_category(s, levels = ())` — codes into its distinct values in their order, or into the levels given (a value not among them null) · `levels(s)` · `codes(s)` · `dummies(s \| f, columns = (), drop_first = true, prefix = (), dtype = "Float")` — indicator columns as a Frame (a Frame's String and Category columns, or those named, each replaced where it stands) · `interact(a, b, sep = ":")` — the pairs of two columns as a Category of the pairs that occur |
| Convert type | `cast(s, "Float"\|"Int"\|"Bool"\|"String"\|"Date"\|"Datetime"\|"Category")` — what the target cannot hold becomes null, so `null_count` reports what was lost; text reads as ISO 8601 dates, a Date's Int is its days since 1970-01-01, a Datetime's its microseconds |
| Order / select | `sort(s, descending = false)` (nulls last) · `argsort(s, descending = false)` (sorting indices, stable) · `take(s, idx)` (gather) · `get(s, i)` (a negative position raises, as a list's does) |
| Convert | `to_list(s)` (nulls → `()`) · `len(s)` |

### Frame verbs

| Group | Verbs |
|---|---|
| Build | `frame(columns)` (`[name, column]` pairs, or an object `{ a: s, b: [1, 2] }`) · `frame_from_records(records)` · `try_frame_from_records(records)` (the Result form, for records from a file, a request, or a user) · `read_csv(text, sep:, dates:, dtypes:, nulls:)` — `sep` one character, `dates` column names or `"auto"` (ISO dates and datetimes), `dtypes` an object of column: type, `nulls` the cells that are null |
| Files | `read_csv_file(path)` (the same options) · `write_csv(f, path, float_fmt = ())` · `to_csv(f, float_fmt = ())` — `float_fmt` `".2f"` or `".3e"` · `read_jsonl(text)` · `read_jsonl_file(path)` (`dates:` and `dtypes:`) · `write_jsonl(f, path)` · `to_jsonl(f)` |
| Streaming | `open_csv(path)` · `open_jsonl(path)` — then `next_chunk(r, n)` · `rows_read(r)` · `at_end(r)`, the same verbs for either |
| Native format | `write_frame(f, path)` · `read_frame(path, columns)` · `frame_info(path)` — exact types, faster loads, one column at a time |
| Inspect | `columns(f)` · `n_rows(f)` · `n_cols(f)` · `head(f, n)` · `tail(f, n)` · `describe(f)` · `schema(f)` · `to_records(f)` · `to_matrix(f, names)` — Float/Int columns as a list of Float lists, for `vec` |
| Splits | `split(f, frac)` — a deterministic holdout under `random.seed`, `[kept, rest]` · `split_at(f, column, frac)` — the forward-in-time split: sorted by `column`, the first `frac` first |
| Version | `ods.version()` — the engine's version, the same as `runtime.version()`, worth quoting in a bug report |
| Subscript | `f["name"]` a column · `f[["a", "b"]]` those columns · `f[mask]` the rows a Bool Series keeps · `s[i]` an element (a negative position raises) · `column(f, name)` is the same as `f[name]`, for when a call reads better than a subscript |
| Shape | `select(f, names and exprs)` · `drop(f, names)` · `rename(f, mapping)` (a map or an object) · `slice(f, start, n)` · `row(f, i)` (a map) · `lookup(f, key_col, key, value_col)` (a hash kept with the Frame; a list of keys gives a Series) · `with_column(f, name, series)` · `with(f, { name: expr, … })` · `filter(f, mask \| expr)` · `sort_by(f, keys, descending = false)` — several keys (names or exprs), `descending` a Bool or one per key |
| Expressions | `col(name)` · `lit(v)` · `alias(expr, name)` · `over(agg, keys)` — an aggregation within groups, given back to every row. Every column function and operator takes an expression and gives one; [Expressions](ods.md#expressions--odscol) |
| Whole rows | `distinct(f, names = all)` · `drop_null(f, names = all)` — duplicates and missing data, first occurrence kept |
| Reshape | `pivot(f, index, columns, values, agg, fill = ())` long→wide · `crosstab(a, b, values = (), agg = "count", normalize = (), name = "row")` — counts (or an aggregate) of each pair of values, `normalize` `"all"`, `"rows"` or `"columns"` · `unpivot(f, ids, value_columns = the rest)` wide→long, emitting `name`/`value` |
| Sample | `sample(f, n)` (also on a Series) — n random rows, without replacement, in original order; seeded by `random.seed` |
| Aggregate / join | `group_by(f, keys)` → a GroupBy, then `agg(by, { name: expr })` (e.g. `col("x").sum()`, `ods.count()`, `col("x").quantile(0.9)`) · `size(by)` · `transform(by, expr)` · `apply(by, fn)` (once per group) · `groups(by)` (`(key, frame)` pairs) · `sum(by)`, `mean(by)`, … over every column — or `group_by(f, keys, aggs)` with the aggs an object, or `[name, op, column]` triples, op one of count, sum, mean, min, max, median, std, var, first, last, n_unique, any, all, argmax, argmin (`[name, "count"]` counts rows) · `join(a, b, on, on_b = on, how = "inner", suffix = "_right", validate = ())` — `on` a name or a list, `how` inner/left/full/semi/anti, `validate` `"many_to_one"`, `"one_to_many"` or `"one_to_one"` · `join_left` · `join_full` · `join_semi` · `join_anti` (the same, `how` fixed) · `asof_join(a, b, on, by = (), direction = "backward", tolerance = (), suffix = "_right")` — each left row takes the right row nearest at or before it (`"forward"`, `"nearest"`) · `concat(frames)` |
| Time | `resample(f, on, every, aggs, by = ())` — rows grouped by the start of their `every` bucket (`"15m"`, `"1h"`, `"1d"`, `"1w"`, `"1mo"`, `"1q"`, `"1y"`) of a Date or Datetime column (buckets of its type) or an ISO date/time text column, sorted by time · `floor(s, every)` — each value's bucket start, in the value's shape |
| Matrices | `matrix(f, cols = every numeric, nulls = "raise")` — a [Matrix](#mat--matrices) of the columns (or of a list of Series), sharing null-free Float columns; `nulls: "drop"` leaves out rows with a null · `corr(f, cols = every numeric, method = "pearson")` (or `"spearman"`) · `cov(f, cols)` — labelled Frames: a `term` column, then one per variable; a column with nulls pairs over the rows both columns have |
| Masks | `m1 & m2` · `m1 \| m2` · `m1 ^ m2` · `!m` · `eq(s, v)` · `ne(s, v)` · `is_in(s, values)` · `contains(s, text, case = true)` · `all_of(masks)` · `any_of(masks)` · `not(mask)` — three-valued, like SQL; parenthesize a comparison `&` joins |

The stack has its own chapter, **[The data stack](ods.md)**: the
reasons for the columnar model (with the measured 50× rewrite behind
it), every Series and Frame verb with its semantics, null handling,
joins and grouped aggregation, and the performance characteristics,
each explained with examples. Engineering history and benchmark method
live in [that chapter's design record](ods.md#the-design-record).

## `mat` — matrices

A `Matrix` is a dense, column-major table of Floats: `ods.matrix(f,
cols)` makes one from a Frame's columns without copying them, `@`
multiplies, and `mat` holds the small dense routines. Every `mat`
function is also a method of a Matrix (`X.gram()`, `X.t()`), and a
Matrix prints as a grid.

```olang
let f = ods.frame({ a: [1.0, 2.0, 3.0, 4.0], b: [2.0, 1.0, 0.0, 5.0], y: [3.0, 3.1, 2.9, 9.2] })
let X = ods.matrix(f, ["a", "b"])
let beta = mat.solve(X.gram(), X.t() @ f.y)        // least squares
println(ods.round(X @ beta, 1).to_list())          // [3.1, 3.0, 2.9, 9.2]
println(ods.round(mat.eigh(mat.of([[2.0, 1.0], [1.0, 2.0]])).values, 9).to_list())   // [3.0, 1.0]
println(ods.corr(f, ["a", "y"])["a"].to_list())
```

| Function | Description |
|---|---|
| `ods.matrix(f, cols?, nulls = "raise")` | a Matrix of a Frame's numeric columns (all of them, or the ones named) or of a list of Series; Int and Bool become Float |
| `X @ b`, `v @ X`, `X @ Y` | the product: a Series for a vector `b` (a list or a Series), `vᵀX` for a vector on the left, a Matrix for two matrices |
| `mat.of(rows)` / `mat.identity(n)` | a Matrix of a list of rows / the identity |
| `mat.t(x)` / `mat.gram(x)` / `mat.matmul(a, b)` | the transpose (free: the same storage) / `XᵀX` in one parallel pass / `a @ b` |
| `mat.col(x, j)` / `mat.row(x, i)` / `mat.shape(x)` | column `j` as a Series (shared) / row `i` as a list / `[rows, cols]`; `len(x)` is the row count |
| `mat.mean(x, axis = 0)` / `mat.std(x, axis = 0, ddof = 1)` | a Series: one value per column (axis 0) or per row (axis 1) |
| `mat.solve(a, b)` / `mat.inv(a)` | `x` with `a x = b` (LU with partial pivoting; `b` a vector or a Matrix) / the inverse |
| `mat.cholesky(a)` | the lower factor `L` of a symmetric positive-definite `a = L Lᵀ` |
| `mat.eigh(a)` | `#{ values, vectors }` of a symmetric matrix, largest first, each vector's largest component positive |
| `mat.cdist(x, c, metric = "sqeuclidean")` | distances between the rows of `x` and of `c`, an `n × m` Matrix; `"euclidean"`, `"cityblock"` (`"manhattan"`), `"chebyshev"`, `"cosine"` |
| `mat.to_frame(x, names?)` / `mat.to_list(x)` | the columns as a Frame (named `0`, `1`, … by default) / the rows as lists |

[Matrices](ods.md#matrices--odsmatrix-and-mat) in the data stack's
chapter explains the storage and the kernels, and compares them with
NumPy.

## `stats` — statistical inference

Distributions, tests, power, robust statistics, the bootstrap and time
series (`stats.ts`). Every statistic is pinned against SciPy reference
values in the test suite. Tests and fits return maps. Sampling draws
from a generator when given `rng:` (a [`random.rng`](#random--randomness)
or an Int seed), else from the `random` module's seeded stream, so
`random.seed` makes simulations reproducible either way.

```olang
let a = ods.series([5.1, 4.9, 6.2, 5.7, 5.5, 4.8, 5.9, 6.1])
let b = ods.series([4.2, 4.8, 4.5, 5.0, 4.4, 4.1, 4.9])
let t = stats.t_test(a, b)
println(`p = ${t.p_value |> str.fmt(".4f")}, d = ${t.d |> str.fmt(".2f")}`)
println(`difference ${t.diff |> str.fmt(".2f")}, 95% CI ${t.ci[0] |> str.fmt(".2f")} to ${t.ci[1] |> str.fmt(".2f")}`)
println(stats.norm.ppf(0.975) |> str.fmt(".6f"))     // 1.959964
println(stats.norm.cdf(ods.series([-1.96, 0.0, 1.96])).to_list() |> to_string)
let n = stats.n_for_power(0.5, power: 0.8)          // per group, d = 0.5
println(`${math.ceil(n)} per group`)                 // 64 per group
```

**Distributions.** Each family is a namespace of `pdf` (`pmf` for a
count), `cdf`, `ppf` and `sample`. `x` (or `p`, or `k`) is a number or
a numeric Series: a Series gives a Series, one native pass, nulls kept,
in parallel past the elementwise threshold. A `ppf` outside [0, 1]
raises for a number and is NaN in a column. `sample(n, …, rng:)` is a
Series of `n` draws (Int for `poisson` and `binom`).

| Family | Parameters |
|---|---|
| `stats.norm` | `mean = 0.0`, `std = 1.0` |
| `stats.t` | `df` |
| `stats.chi2` | `df` |
| `stats.f` | `d1`, `d2` |
| `stats.lognorm` | `mu = 0.0`, `sigma = 1.0`: the log is normal with this mean and sd (NumPy's `lognormal`) |
| `stats.beta` | `a`, `b` |
| `stats.gamma` | `shape`, `scale = 1.0` (SciPy's scale) |
| `stats.poisson` | `lam` — `pmf`, `cdf`, `ppf`, `sample` |
| `stats.binom` | `trials`, `p` — `pmf`, `cdf`, `ppf`, `sample` |

**Tests, power and description.**

| Function | Description |
|---|---|
| `stats.describe(s)` | a map: `count`, `null_count`, `mean`, `std`, `min`, `q25`, `median`, `q75`, `max` |
| `stats.corr(a, b)` / `stats.cov(a, b)` | Pearson correlation / sample covariance of two Series, over the rows where both are present |
| `stats.t_test(a, b, level = 0.95, alternative = "two-sided")` | Welch's two-sample test when `b` is a Series, the one-sample test against the mean `b` when it is a number: a map of `t`, `df`, `p_value`, the means (`mean_a`, `mean_b`, or `mean`), `diff`, `se`, `ci` (the interval of the difference at `level`, a pair; of the mean for one sample, as SciPy and R give it) and `d` (Cohen's d: the difference over the pooled sd). `alternative` is `"two-sided"`, `"greater"` or `"less"` |
| `stats.chi2_test(observed, expected)` | the χ² goodness-of-fit test of two Series of counts: a map with `chi2`, `df`, `p_value` |
| `stats.lm(y, xs)` | ordinary least squares with an intercept, a map of coefficients and fit |
| `stats.power_t(effect, n, alpha = 0.05, alternative = "two-sided", kind = "two-sample", ratio = 1.0)` | the power of a t-test by the noncentral t (statsmodels' `TTestIndPower`/`TTestPower`): `effect` is Cohen's d, `n` the first group's size (`ratio` the second's over it), `kind` `"two-sample"`, `"one-sample"` or `"paired"` |
| `stats.n_for_power(effect, power = 0.8, alpha = 0.05, alternative = "two-sided", kind = "two-sample", ratio = 1.0)` | the `n` (a Float, per group) at which `power_t` reaches `power` |
| `stats.mann_whitney(a, b, alternative = "two-sided")` | the Mann–Whitney U test by the normal approximation with tie and continuity corrections (SciPy's `method="asymptotic"`): `u` (of `a`), `z`, `p_value`, `effect` (U / n₁n₂, the chance a value of `a` exceeds one of `b`) |
| `stats.spearman(a, b)` | Spearman's rank correlation over the complete pairs: `rho`, `p_value`, `n` |

**Robust statistics and the bootstrap.**

| Function | Description |
|---|---|
| `stats.mad(s, scale = "normal")` | the median absolute deviation, times 1.4826 by default (a normal sample's sd); `scale: 1.0` for the raw MAD |
| `stats.iqr(s)` | the interquartile range, q75 − q25 (linear interpolation, as `ods.quantile`) |
| `stats.zscore(s, robust = false)` | standard scores: `(x − mean) / sd`, or with `robust: true` `(x − median) / mad`; nulls stay null |
| `stats.tukey_fences(s, k = 1.5)` | the pair `(q25 − k·IQR, q75 + k·IQR)` |
| `stats.bootstrap(s, stat = "mean", n = 2000, seed?, rng?, level = 0.95, compare = "diff")` | `n` resamples, each the statistic (`"mean"`, `"median"`, `"std"`, `"var"`, `"sum"`, or a quantile as a number such as `0.9`), and the percentile interval at `level`: a map of `estimate`, `lo`, `hi`, `ci`, `se` and `draws` (a Series). `s` may be a list of two Series, each resampled on its own: then the statistic is compared by `compare`, `"diff"` (`a − b`) or `"ratio"` (`a / b`). Native and parallel; resample `i` draws from stream `i` of its seed, so the result is the same on any number of threads |

```olang
let r = random.rng(4)
let x = r.normal(500, 10.0, 2.0)
let (lo, hi) = stats.tukey_fences(x, k: 3.0)
println(`a robust sd of ${stats.mad(x) |> str.fmt(".2f")}`)
let boot = stats.bootstrap(x, "median", n: 1000, rng: r)
println(boot.lo < boot.estimate && boot.estimate < boot.hi)   // true
let a = r.normal(400, 50.0, 12.0)
let b = r.normal(400, 52.0, 12.0)
let lift = stats.bootstrap([b, a], compare: "ratio", seed: 2)
println(`lift ${lift.estimate - 1.0 |> str.pct()}`)
```

### `stats.ts` — time series

A series is a numeric Series with no nulls (`interpolate` or `fill_null`
first). The functions are methods of a Series too: `s.acf(48)`.

| Function | Description |
|---|---|
| `stats.ts.acf(s, lags?, ci = 0.95)` | the autocorrelation at lags 0..=`lags` (default `10·log₁₀ n`), by FFT in O(n log n): a Frame of `lag`, `acf`, and Bartlett's interval `lo`, `hi` at level `ci` (statsmodels' `acf(fft=True, alpha=…)`) |
| `stats.ts.pacf(s, lags?, ci = 0.95, method = "yw")` | partial autocorrelations by Yule–Walker (`"yw"`, statsmodels' `ywadjusted`; `"ywm"` its `ywmle`): a Frame of `lag`, `pacf`, `lo`, `hi` (± z/√n) |
| `stats.ts.decompose(s, period, model = "additive", time?)` | a centred moving average's `trend`, the phase means of the rest as `seasonal`, and `resid` (statsmodels' `seasonal_decompose`; `"multiplicative"` divides): a Frame of `observed`, `trend`, `seasonal`, `resid` (the trend null where the average does not reach), with a `time` column first when `time:` (a Date or Datetime Series) is given |
| `stats.ts.holt_winters(s, season?, trend = "add", seasonal = "add", fit = true, alpha?, beta?, gamma?, time?)` | Holt-Winters exponential smoothing: an additive trend (`trend: "none"` for none), and with a `season` an additive or multiplicative (`"mul"`) season of that length (no `season`: Holt's linear method). The rates not given are fitted by bounded Nelder–Mead on the sum of squared one-step errors. The first season (one value without a season) sets the initial state, as Winters' method does. A `HoltWinters` value: `forecast(h)`, and the fields `fitted`, `resid`, `params` (`alpha`, `beta`, `gamma`), `sse`, `level`, `slope`, `season`, `n`, `m` |
| `stats.ts.forecast(model, h)` | the `h` forecasts after the series: a Series, or with the model's `time:` a Frame of `time` (continuing the series' step: a day, an hour, a calendar month) and `forecast` |
| `stats.ts.seasonal_naive(s, season, h, time?)` | the last `season` values, repeated over `h` steps (a Series, or a Frame with `time:`) |

```olang
let t = ods.arange(0, 140)
let y = t * 0.5 + 100.0 + (t % 7).cast("Float") * 2.0
let hw = stats.ts.holt_winters(y, season: 7)
println(hw.forecast(3).to_list() |> map((v) => str.fmt(v, ".1f")))   // ["170.0", "172.5", "175.0"]
println(hw.params.alpha >= 0.0 && hw.sse < 1e-6)                      // true
let c = stats.ts.acf(y.diff().fill_null(0.0), 14)
println(c.n_rows())                                                    // 15
```

**[The data stack](ods.md#stats--from-description-to-inference)**
covers the module end to end, including a complete regression workflow
and how to interpret p-values and `r2`.

## `learn` and `ml` — machine learning

`learn` trains, predicts and evaluates models over Frames: splits and
folds, scalers, encoders and imputers, linear and logistic regression,
trees, random forests, gradient boosting, k-nearest neighbours, naive
Bayes, k-means, Gaussian mixtures and PCA, and the metrics to judge
them. Every kernel is Rust (`olang-ods`), parallel past measured sizes,
and pinned against scikit-learn in the test suite. `learn` functions
take their options by name (`learn.forest(f, y: "churn", n_trees: 300,
seed: 1)`) or in one trailing map, and an unknown option is an error
naming the options there are. **`use ml`** adds short names for a few
options and the forms that call an olang function (below).

A fit returns a Map: `fit.params.sqft`, `fit.r2`, `fit.coef` (a Frame of
term, estimate, se, t, p, lo, hi) read like fields. Its `model` entry is
what `learn.predict` uses, and its `spec` entry refits it, so
`learn.cross_validate`, `learn.learning_curve` and `learn.grid_search`
refit natively, in parallel over folds. A fit's methods are `learn`'s
functions: `fit.predict(test)`, `fit.summary()`, `fit.importance()`,
`fit.conf_int(0.9)`. A method wins over the fit's entry of the same name,
so `fit.importance` reads the Frame and `fit.importance()` computes it.
Inputs are column names (`()` means every column but the target). A String column is one-hot encoded
with the training levels, and the first level is dropped for models with an
intercept. A Bool becomes 0/1. A row with a null (or NaN) in a used column
is left out of a fit (`fit.dropped` counts them) and predicts null.
Classification or regression follows the target's type (Float is
regression; String, Bool and Int are classes), and `task` overrides it.
Every random choice takes `seed`. Without one, a seed is drawn from the
`random` stream and recorded in the spec. Results do not depend on the
thread count.

```olang
use ml
let n = 120
let sqft = map(range(0, n), (i) => 600.0 + to_float((i * 37) % 100) * 20.0)
let hood = map(range(0, n), (i) => if i % 3 == 0 => "east" else => "west")
let price = map(range(0, n), (i) => 50.0 + 0.2 * sqft[i] + (if hood[i] == "east" => 30.0 else => 0.0) + to_float((i * 13) % 7))
let homes = ods.frame([["sqft", sqft], ["hood", hood], ["price", price]])
let s = ml.split(homes, test: 0.25, seed: 7)
let fit = ml.linear(s.train, y: "price", x: ["sqft", "hood"])
println(fit.params.sqft > 0.19)                       // true
let pred = ml.predict(fit, s.test)
println(ml.rmse(s.test["price"], pred) < 5.0)          // true
println(ods.n_rows(ml.cross_validate(fit, homes, folds: 4)))   // 4
```

**Formulas.** Every supervised model also takes a formula where `y` and
`x` go: `learn.linear(f, "price ~ sqft + C(hood) + C(kind):C(hour) +
I(sqft^2)")`. The notation is R's and patsy's, parsed natively: `+` adds
a term, `a:b` is an interaction, `a * b` is `a + b + a:b`, `-` removes a
term, and `- 1` (or `+ 0`) drops the intercept. `C(x)` treats a column
as categorical whatever its type, so an Int hour becomes 24 levels. A
String or Category column named bare is categorical too. `I(…)` is
arithmetic (`+ - * / ^`, numbers, columns), evaluated by the expression
engine, and `log(x)`, `sqrt(x)` and the other math functions are terms
of their own. The response may be an expression too, as in `log(price) ~ …`.
Coding follows patsy, so the design has full rank. A categorical factor
uses treatment contrasts where its margin is already in the model and
is coded in full where it is not, so `C(day):C(hour)` alone gives one
column per cell but one. Columns are named `hood_west` and
`kind_loft:hour_3`. `fit.predict(new)` computes the formula's columns on
the new rows with the training levels. An error points into the
formula:

```olang
let n = 48
let f = ods.frame({
    x: ods.arange(n) * 0.25,
    kind: ods.series(map(range(0, n), (i) => ["flat", "house", "loft"][i % 3])),
    hour: ods.arange(n) % 4,
})
let f = f.with({ y: f.x * 2.0 + f.x * f.x * 0.5 + f.hour * 0.1 + 1.0 })
let fit = learn.linear(f, "y ~ x + I(x^2) + C(kind):C(hour)")
println(len(fit.features))                       // 13
println(math.round(map_get(fit.params, "I(x^2)") * 1000.0) / 1000.0)   // 0.5
println(ods.columns(fit.conf_int(0.9)))          // ["term", "lo", "hi"]
println(ods.n_rows(fit.predict(f, interval: "prediction")))   // 48
```

| Function | Description |
|---|---|
| `learn.split(f, opts)` | `#{ train, test }`; opts `test` (fraction or rows, 0.25), `stratify` (a column), `shuffle`, `seed` |
| `learn.kfold(f \| n, k, opts)` / `learn.folds(f, k, opts)` | fold ids per row / the folds as `#{ fold, train, test }`; opts `shuffle`, `stratify`, `seed` |
| `learn.time_splits(f \| n, n_splits, opts)` | rolling-origin `#{ train, test }` windows (sklearn's `TimeSeriesSplit`); opts `test_size`, `gap`, `max_train` |
| `learn.scaler(f, cols, opts)`, `learn.one_hot(f, cols, opts)`, `learn.imputer(f, cols, opts)` | fitted transformers: `method` standard/minmax/robust; `drop_first`; `strategy` mean/median/most_frequent/constant and `fill` |
| `learn.transform(model, f)` | apply a fitted scaler, encoder, imputer or PCA |
| `learn.poly(f, cols, degree, opts)` | polynomial columns (`a^2`, `a*b`) appended; `interaction_only` |
| `learn.linear(f, y, x, opts)` | OLS: `params`, `coef`, `r2`, `adj_r2`, `sigma`, `aic`, `bic`, `fitted`, `resid`; opts `intercept`, `weights`. `y` may be a formula (`"price ~ sqft + C(hood)"`) in this and every supervised model |
| `learn.ridge`, `learn.lasso`, `learn.elastic_net` `(f, y, x, opts)` | penalized regression (sklearn's objectives): `alpha`, `l1_ratio`, `intercept`, `max_iter`, `tol` |
| `learn.logistic(f, y, x, opts)` | binary (IRLS) or multinomial (Newton) logistic regression with se, z, p and odds ratios; `C` or `penalty`, `class_weight` (`"balanced"` or a map), `weights` |
| `learn.tree(f, y, x, opts)` | CART: `max_depth`, `min_samples_split`, `min_samples_leaf`, `criterion` (gini, entropy, squared_error), `max_features`, `task`, `seed` |
| `learn.forest(f, y, x, opts)` | random forest, trees grown in parallel: the tree options plus `n_trees`, `bootstrap`, `max_samples`, `oob` (gives `oob_score`) |
| `learn.boost(f, y, x, opts)` | histogram gradient boosting (squared loss, or log loss for a binary target): `n_trees`, `learning_rate`, `max_depth`, `max_leaf_nodes`, `min_samples_leaf`, `l2`, `subsample`, `max_bins`, `early_stopping`, `validation_fraction`, `validation_rows` (row indices held out instead: an explicit validation set), `validation_tail` (the last n rows held out: a time series' most recent), `n_iter_no_change`; the fit says `train_loss`, `valid_loss` (each round's) and `best_iter` |
| `learn.knn(f, y, x, opts)` / `learn.naive_bayes(f, y, x, opts)` | k-nearest neighbours (`k`, `weights_by`) / Gaussian naive Bayes (`var_smoothing`) |
| `learn.kmeans(f, cols, k, opts)` / `learn.gmm(f, cols, k, opts)` | k-means++ (`n_init`, `max_iter`, `seed`): `labels`, `centers`, `inertia` / Gaussian mixture by EM: `means`, `weights`, `bic` |
| `learn.pca(f, cols, opts)` | principal components (`n_components`, `scale`): `explained_variance`, `explained_variance_ratio`, `components` |
| `learn.silhouette(f, cols, labels, opts)` | mean silhouette coefficient; `sample`, `seed` |
| `learn.conf_int(model, level = 0.95)` | the coefficients' intervals: a Frame of term, lo, hi |
| `learn.predict(model, f, opts)` | values, class labels or clusters; `interval`: `"confidence"` or `"prediction"` (an OLS interval Frame at `level`); `kind`: `"prob"` (positive class, or a Frame for 3+ classes), `"probs"`, `"link"`, `"confidence"` / `"prediction"` (an OLS interval Frame at `level`); `trees: k` a boosted model's prediction from its first k trees (staged) |
| `learn.importance(model)`, `learn.summary(model, opts)`, `learn.params(model)` | importances (impurity, or abs(coef)) / the model as a Frame / estimates by term |
| `learn.score(model, f, metric)` | one metric against the target column of `f` |
| `learn.spec(kind, y, x, opts)` / `learn.fit(spec, f)` | how to fit, without fitting / fit a spec or refit a model |
| `learn.cross_validate(spec, f, opts)` | per-fold scores (`folds`, `metrics`, `seed`); stratified for classifiers |
| `learn.permutation_importance(model, f, opts)` | `metric`, `repeats`, `seed` |
| `learn.partial_dependence(model, f, feature, opts)` | the mean prediction over a grid (`grid`, `points`, `sample`) |
| `learn.learning_curve(spec, f, opts)` | train and test scores by training size (`sizes`, `folds`, `metric`) |
| `learn.grid_search(spec, f, grid, opts)` | `#{ results, best, best_score, metric, model }` |
| `learn.rmse`, `learn.mse`, `learn.mae`, `learn.mape`, `learn.r2` `(y, pred)` | regression metrics |
| `learn.accuracy(y, pred)`, `learn.precision`, `learn.recall`, `learn.f1` `(y, pred, opts)` | label metrics; `average` binary/macro/micro/weighted, `pos` |
| `learn.log_loss`, `learn.brier`, `learn.auc`, `learn.average_precision` `(y, p, opts)` | probability metrics; `pos` names the positive class |
| `learn.roc`, `learn.pr_curve` `(y, p, opts)` | the curves as Frames |
| `learn.confusion(y, pred)`, `learn.classification_report(y, pred)` | Frames |
| `learn.at_threshold(y, p, opts)` / `learn.threshold_sweep(y, p, opts)` | counts and rates at one `threshold` / at every distinct one |
| `learn.calibration(y, p, opts)` | predicted against observed per bin (`bins`, `strategy` uniform/quantile) |

`use ml` mirrors each of these with keywords (`ml.forest(f, y: "churn",
trees: 300, seed: 1)`; `trees`, `bins`, `validation` and `patience` are the
short names of `n_trees`, `max_bins`, `validation_fraction` and
`n_iter_no_change`; `ml.boost`'s `validation` may also be a Frame, its
rows appended to the data's and held out, and `validation_rows` or
`validation_tail` turns early stopping on). It adds three more: `prob(model, f)`, the class
probabilities; `fit_transform(model_fn, data)`; and a
`cross_validate(fit, data)` whose `fit` may also be a
`(train) => model` function, called once per fold:

| `ml` function | Purpose |
|---|---|
| `split`, `kfold`, `folds`, `time_splits` | splitting, with `test:`, `k:`, `stratify:`, `seed:` |
| `scaler`, `one_hot`, `imputer`, `transform`, `fit_transform`, `poly` | preprocessing |
| `linear`, `ridge`, `lasso`, `elastic_net`, `logistic` | linear models, `y:` and `x:` |
| `tree`, `forest`, `boost`, `knn`, `naive_bayes` | trees, ensembles and the rest |
| `kmeans`, `gmm`, `pca`, `silhouette` | unsupervised |
| `predict`, `prob`, `importance`, `summary`, `params`, `score`, `spec`, `fit` | the model interface |
| `cross_validate`, `permutation_importance`, `partial_dependence`, `learning_curve`, `grid_search` | evaluation |
| `rmse`, `mse`, `mae`, `mape`, `r2`, `accuracy`, `precision`, `recall`, `f1`, `log_loss`, `brier`, `auc`, `average_precision`, `roc`, `pr_curve`, `confusion`, `classification_report`, `at_threshold`, `threshold_sweep`, `calibration` | metrics |

Early stopping on a time series holds out its most recent rows, not a
random tenth: `validation_tail` (or `validation_rows`, or a `validation`
Frame) says which, `valid_loss` is their loss each round, and `trees:`
in `predict` gives the error curve on any rows:

```olang
use ml
let n = 400
let t = map(range(0, n), (i) => to_float(i))
let y = map(range(0, n), (i) => math.sin(t[i] / 20.0) + to_float(i % 7) * 0.05)
let f = ods.frame([["t", ods.series(t)], ["y", ods.series(y)]])
let m = ml.boost(f, y: "y", x: ["t"], trees: 200, validation_tail: 80, patience: 10, seed: 1)
println(ods.len(m.valid_loss) > m.best_iter)            // true: a loss each round
let recent = ods.tail(f, 80)
let first = ml.rmse(recent["y"], ml.predict(m, recent, trees: 1))
let kept = ml.rmse(recent["y"], ml.predict(m, recent))
println(kept < first)                                   // true
```

**[The data stack](ods.md#learn--machine-learning)** walks through a
classification workflow end to end, and gives the benchmarks against
scikit-learn.

## `plot` — charts as SVG text

Charts render to complete standalone SVG documents as strings — write
one with `fs.write_file`, serve it over `http`, or land it on a page
with `dom.set_html`. `plot.line`, `plot.scatter`, `plot.area`,
`plot.lines` (multi-series with legend), `plot.xy` (layered marks over
shared scales), `plot.bar`, `plot.bars` (grouped), `plot.stacked`,
`plot.hist`, `plot.heatmap`, and `plot.box` take Series data plus one
options map (`title`, `x_label`, `y_label`, `width`, `height`, `theme`
— `"dark"` re-tunes every color for a dark surface — `paper` and `ink`,
which take the background and the frame's color from the page
(`"transparent"`, `"currentColor"`) instead of the theme, `responsive`,
which sizes the SVG to its container, `font_size` and `font` for the
text — the default 12.5 suits a 720×440 document, a chart drawn into a
narrow column wants less — plus the color system below; unknown keys
are errors). Defaults follow a colorblind-validated
ten-hue palette, so a chart is presentable with `#{}` — and color is
an option, not a fate: `colors` gives a chart its own palette, `vary`
colors bar categories individually, `scale` picks a heatmap ramp
(`ocean`/`ember`/`thermal`/`diverging`), and `plot.ramp(name, t)`
hands the same ramps to your code. Every chart carries its description
as data (a `<metadata id="olang-chart">` of JSON after its opening
tag), and `plot.describe(svg)` answers it as a Map — `Err` for text
that carries none ([the data stack](ods.md#a-charts-description)).
`plot.gg(p)` renders a [`gg`](#gg--a-grammar-of-graphics) plot to SVG
and `plot.gg_scene(p)` to its scene as JSON (the display list a host
draws natively). In `viz`, `"color_by": "column"`
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
[the data stack](ods.md#the-viz-grammar).

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
`/board.html` is the complete example: a KPI row, five charts, a
URL-carried status filter, and a 5-second auto-refresh in one source
file.

```olang
let x = ods.linspace(0.0, 6.28, 50)
let y = ods.series(map(ods.to_list(x), (v) => math.sin(v)))
let svg = plot.line(x, y, #{ "title": "sin(t)", "x_label": "t" })
println(to_string(str.contains(svg, "<svg")))
```

**[The data stack](ods.md#plot--charts-as-svg-text)** covers the
SVG-as-text philosophy, each chart type, and null handling per chart.

---

Next: [Packages](packages.md) for multi-file programs and dependencies, or
[Internals](internals.md) for how the runtime executes all of this.

## `gg` — a grammar of graphics

**`use gg`** says a plot the way ggplot2 does: data, aesthetic mappings,
and layers (a geom drawn from a stat's output after a position), then
scales, a facet, a coordinate system, labels and a theme. A plot is a
value — a map built in steps, each subject-first so it pipes — and
nothing in the module computes: `plot.gg` does it all natively (stats,
scale training, positions, facet layout, legends, text measurement) and
serializes one scene to SVG. A plot that is a notebook cell's or the
REPL's value is drawn natively by an editor (the kernel sends its scene:
[tooling](tooling.md)), with each mark's values on hover and a dragged
range zoomed; `gg.svg(p)` is its SVG text and `gg.save(p, path)` writes
it.

```olang
use gg
let f = ods.frame([["size", ods.series([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])],
                   ["price", ods.series([2.1, 3.9, 6.2, 7.8, 10.1, 12.2])],
                   ["kind", ods.series(["a", "b", "a", "b", "a", "b"])]])
let p = gg.plot(f, gg.aes(x: "size", y: "price", color: "kind"))
    |> gg.point(size: 2)
    |> gg.smooth(method: "lm", se: false)
    |> gg.scale_y_continuous(labels: "dollar")
    |> gg.labs(title: "Price by size")
assert(str.starts_with(gg.svg(p), "<svg"))
```

A mapping names a column (`"price"`), holds a Series of the frame's
length, or names a variable the layer's stat computes:
`gg.aes(y: gg.after_stat("density"))`. A number or a Bool in `aes`, or
`gg.constant(value)`, is a constant *mapping*: one value for every row
that still groups and keys a legend, as ggplot's `aes(group = 1)` and
`aes(colour = "all")` do (`gg.line(gg.aes(group: 1))` joins a line across
a categorical x; a String in `aes` always names a column, so a constant
String says `gg.constant("all")`). A mapped `group` is the grouping
alone, not crossed with the other categories. A constant look goes to
the layer instead (`gg.point(color: "steelblue", alpha: 0.4, size: 2)`): colours
are names or `#hex`, sizes are ggplot's units (a point's `size` 1.5, a
line's `linewidth` 0.5), a text's `size` is pixels. Strings mapped to a
position that all read as ISO dates (`2024-03-01`, `2024-03-01 14:30`)
make a date axis; other strings are categories, sorted naturally unless a
scale's `limits` orders them. Errors say what is wrong and what exists: a
missing column lists the frame's.

| Building | Purpose |
|---|---|
| `plot(data, mapping)` | a plot of a Frame (or records) with a default mapping |
| `aes(x, y, color, fill, size, shape, alpha, linetype, linewidth, group, label, xmin, xmax, ymin, ymax, xend, yend, weight, sample, xintercept, yintercept)` | the mappings (`colour` too) |
| `after_stat(name)` | a stat's variable as a mapping: `count`, `density`, `ncount`, `ndensity`, `scaled`, `prop`, `ecdf`, … |
| `constant(value)` | one value for every row, as a mapping (`color: gg.constant("all")`): a group and a legend key |
| `layer(p, geom, stat, position, mapping, data, params)` | any geom with any stat and position |
| `svg(p)` · `save(p, path)` · `scene(p)` | the SVG text, written to a file; the scene as JSON (what a host draws) |

Every geom takes `mapping` and `data` (a layer's own, else the plot's;
`inherit: false` leaves the plot's mapping out), its constants, and
`legend: false` to keep it out of the legends.

| Geom | Draws |
|---|---|
| `point(p, …)` · `jitter(p, width, height, seed, …)` | points; points moved at random (seeded) |
| `line(p, …)` · `path(p, …)` · `step(p, direction, …)` | lines in x order, in data order, as a staircase (`"hv"`, `"vh"`) |
| `area(p, …)` · `ribbon(p, …)` | an area from zero (stacked by group), a band from `ymin` to `ymax` |
| `bar(p, …)` · `col(p, …)` | bars counting each x, bars of height y (stacked; `position: "dodge"` or `"fill"`) |
| `histogram(p, bins, binwidth, boundary, center, breaks, …)` · `freqpoly(p, …)` | binned counts as bars, as a line |
| `density(p, bw, adjust, kernel, n, trim, …)` | a kernel density estimate: `bw` `"nrd0"` (Silverman's rule, the default), `"nrd"`, `"scott"` or a number; kernels gaussian, epanechnikov, rectangular, triangular, biweight |
| `boxplot(p, width, coef, outliers, linewidth, …)` | median, hinges, whiskers at 1.5 IQR, outliers |
| `violin(p, scale, trim, bw, adjust, draw_quantiles, …)` | each group's density mirrored (`scale` `"area"`, `"count"`, `"width"`) |
| `errorbar(p, width, position, …)` · `pointrange(p, position, …)` · `linerange(p, position, …)` · `crossbar(p, width, position, …)` | intervals from `ymin` to `ymax`; `position: gg.position_dodge(width: 0.5)` sets groups side by side (each with `alpha`, `linewidth`) |
| `summary(p, fun, level, mult, geom, alpha, fill, linewidth, position, …)` | y summarised at each x: `fun` `"mean_se"` (default), `"mean_cl_normal"` (a t interval at `level`), `"mean_sdl"`, `"median_hilow"`, `"mean"`, `"median"`, drawn as a point range (or `geom`: `"errorbar"`, `"crossbar"`, `"point"`, `"line"`, `"col"`, styled as that geom is) |
| `smooth(p, method, se, level, span, degree, k, n, fullrange, …)` | a smoother and its standard-error band: `"lm"` (a polynomial of `degree`), `"loess"` (local quadratic, `span` 0.75), `"gam"` (penalized cubic regression spline, smoothness by REML as ggplot asks of mgcv; `k` 10, or a third of the distinct x when fewer than 30, at least 4), `"auto"` (loess under 1,000 points, gam over) |
| `text(p, label, size, hjust, vjust, nudge_x, nudge_y, check_overlap, fontface, …)` · `label(p, …)` | text at each point; text on a box. Without a colour, text on a filled tile, bar or rect beneath it (or a label's own fill) takes the dark ink or the light, whichever reads, in either appearance |
| `tile(p, width, height, …)` · `raster(p, …)` · `rect(p, …)` | heatmap tiles; tiles that abut; rectangles from `xmin`… |
| `hline(p, yintercept)` · `vline(p, xintercept)` · `abline(p, slope, intercept)` | reference lines (a number or a list); an abline is cut to its panel (a coord's limits, a fixed ratio) |
| `rug(p, sides, length, …)` · `segment(p, …)` | ticks along the axes; segments to (`xend`, `yend`) |
| `hex(p, bins, binwidth)` · `bin2d(p, bins, binwidth)` | 2-D bins filled by count |
| `ecdf(p, …)` · `qq(p, mean, sd, …)` · `qq_line(p, …)` | the empirical cdf; a normal Q-Q plot of `sample` and its quartile line |
| `annotate(p, geom, x, y, xend, yend, xmin, xmax, ymin, ymax, label, …)` | one mark from constants: a note, a band, a line |

Positions: a layer's `position` is a name or one of `position_stack(reverse)`,
`position_fill(reverse)`, `position_dodge(width)`, `position_dodge2(width, padding)`,
`position_jitter(width, height, seed)`, `position_nudge(x, y)`. A dodge's
`width` is the space the groups at an x spread over (by default the mark's
own); each mark keeps its own width shared among them, as ggplot dodges
(`errorbar(width: 0.2, position: gg.position_dodge(width: 0.6))`).

Scales (a later one for the same aesthetic replaces the earlier):

| Scale | Purpose |
|---|---|
| `scale_x_continuous(p, name, breaks, labels, limits, expand, trans, prefix, suffix)` · `scale_y_continuous` | an axis: `labels` `"percent"`, `"comma"`, `"dollar"`, `"short"` (1.5k, 2M), `"scientific"`, `"plain"`, or a list; `breaks` a list or a count; `limits` `[lo, hi]` (either `()`); `trans` `"log10"`, `"sqrt"`, `"reverse"` |
| `scale_x_log10` · `scale_y_log10` · `scale_x_sqrt` · `scale_y_sqrt` · `scale_x_reverse` · `scale_y_reverse` | a transformed axis (log10: breaks at powers of ten) |
| `scale_x_discrete(p, name, limits, labels, breaks)` · `scale_y_discrete` | categories: `limits` their order, `labels` a list or a map |
| `scale_x_date(p, name, date_breaks, date_labels, limits, breaks)` · `scale_y_date` · `scale_x_datetime` · `scale_y_datetime` | dates: `date_breaks` `"1 month"`, `"2 weeks"`, `"6 hours"`; `date_labels` strftime (`%Y %m %d %b %B %H %M %S %e %j %a %A`) |
| `scale_color_manual(p, values, …)` · `scale_fill_manual` | colours by hand: a list, or a map of level to colour |
| `scale_color_discrete(p, palette, …)` · `scale_fill_discrete` | categories from `"quiet"` (the default: ten hues tuned per appearance) or a viridis option |
| `scale_color_viridis_d(p, option, begin, end, direction, …)` · `scale_fill_viridis_d` · `scale_color_viridis_c` · `scale_fill_viridis_c` | the viridis family: `option` viridis, magma, inferno, plasma, cividis, mako, rocket |
| `scale_color_gradient(p, low, high, …)` · `scale_fill_gradient` · `scale_color_gradientn(p, colors)` · `scale_fill_gradientn` | sequential colours |
| `scale_color_gradient2(p, low, mid, high, midpoint, …)` · `scale_fill_gradient2` | diverging colours centred on `midpoint` (0) |
| `scale_color_grey(p, start, end)` · `scale_fill_grey` · `scale_color_identity(p)` · `scale_fill_identity` | greys; the column's values as the colours |
| `scale_size(p, range, …)` · `scale_alpha(p, range, …)` · `scale_shape(p, values, …)` · `scale_linetype(p, values, …)` | sizes by area, opacities, glyphs (circle, triangle, square, diamond, open ones, plus, cross), dashes |
| `scale_linewidth(p, range, name, breaks, labels, limits, trans, guide)` | line widths for a `linewidth` mapping: `range` [thinnest, thickest] in linewidth units (default [0.25, 2]) |

Every colour scale takes `name`, `labels`, `limits`, `breaks` and
`guide: "none"`. Legends merge: aesthetics mapped to one variable under
one title (colour and shape, say) share a legend; a continuous colour is a
colour bar.

| Layout | Purpose |
|---|---|
| `facet_wrap(p, facets, ncol, nrow, scales)` | a panel per value of a column (or a list of them); `scales` `"fixed"`, `"free"`, `"free_x"`, `"free_y"` (a free scale's panels each bin a histogram, and grid a density, over their own range) |
| `facet_grid(p, rows, cols, scales)` | a grid of panels, rows by columns |
| `coord_flip(p, xlim, ylim)` · `coord_fixed(p, ratio)` · `coord_cartesian(p, xlim, ylim)` | the axes swapped; a fixed aspect; a zoom that drops no data |
| `labs(p, title, subtitle, caption, x, y, color, fill, size, shape, alpha, linetype, linewidth)` · `ggtitle(p, title, subtitle)` · `xlab(p, title)` · `ylab(p, title)` | titles (`""` hides an axis's) |
| `theme(p, name, appearance, base_size, legend_position, width, height, grid, font)` | the look: `"minimal"` (the default — hairline grid, no ticks, one type scale), `"classic"`, `"bw"`, `"gray"`, `"linedraw"`, `"void"`; `appearance` `"light"`, `"dark"`, `"contrast"` (else the host's: a notebook's follows Studio's) |

The vertical axis's title reads across, over its labels; a plot is 720
pixels wide and as tall as its panels need unless the theme says.

