# The olang Standard Library Reference

Everything the runtime ships: the global builtins (always in scope) and the
nineteen native modules plus two olang-source modules compiled into the
binary. As in the [language reference](language.md), every `olang` code
block here is executed by the test suite — the examples are guaranteed
current.

Part of [the olang book](README.md) ·
[Tour](tour.md) · [Language](language.md) · [Packages](packages.md) ·
[Internals](internals.md) · [Stability](stability.md)

---

## Table of Contents

- [Conventions](#conventions)
- [Global builtins](#global-builtins)
- [`str` — strings](#str--strings)
- [`col` / `colx` — collections](#col--colx--collections)
- [`math` / `mathx` — mathematics](#math--mathx--mathematics)
- [`json` — JSON](#json--json)
- [`csv` — CSV](#csv--csv)
- [`re` — regular expressions](#re--regular-expressions)
- [`dates` — dates and times](#dates--dates-and-times)
- [`time` — clocks and sleeping](#time--clocks-and-sleeping)
- [`random` — randomness](#random--randomness)
- [`crypto` — hashing and encryption](#crypto--hashing-and-encryption)
- [`base64` — base64](#base64--base64)
- [`fs` — file system](#fs--file-system)
- [`os` — operating system](#os--operating-system)
- [`http` — HTTP](#http--http)
- [`db` — SQLite](#db--sqlite)
- [`testing` — assertions](#testing--assertions)
- [`ods` — Series and Frames](#ods--series-and-frames)
- [`stats` — statistical inference](#stats--statistical-inference)
- [`plot` — charts as SVG text](#plot--charts-as-svg-text)

## Conventions

**Modules are always in scope.** `str.trim(...)`, `json.parse(...)`, and the
rest work without any `use`. (The olang-source modules `colx` and `mathx`
are the exception: import them with `use colx`.)

**Fallible functions return `Result`.** Anything that can fail — parsing,
I/O, lookups that may miss — returns `Ok(v)` or `Err(e)`. Unwrap it, pattern
match it, or propagate with `?`:

```olang
let n = unwrap(str.parse_int("42"))          // trust it
let m = unwrap_or(str.parse_int("nope"), 0)  // with a default
println(to_string(n + m))
```

Pure computations (`str.trim`, `math.sqrt`, `crypto.sha256`, ...) return
their value directly.

## Global builtins

### Output

| Function | Description |
|---|---|
| `print(v)` | write without newline |
| `println(v)` | write with newline |

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

`show` is for building output for people; `to_string` shows a value's shape
(so `to_string("hi")` is `"hi"` with quotes, `show("hi")` is bare `hi`):

```olang
println(show("hi") + " vs " + to_string("hi"))
println(typeof(3.5) + " " + show(to_int(3.9)) + " " + show(to_float("2.5")))
println(show(len([1, 2, 3])) + " " + show(len("abcd")))
```

### Lists

| Function | Description |
|---|---|
| `range(a, b)` | list of integers `a` up to (excluding) `b` |
| `head(xs)` / `tail(xs)` | first element / all but the first |
| `take(xs, n)` / `skip(xs, n)` | first `n` / all but the first `n` |
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
println(to_string(cons(0, [1, 2])) + " " + to_string(flatten([[1], [2, 3]])))
println(to_string(chunk([1, 2, 3, 4, 5], 2)))
println(to_string(zip([1, 2], ["a", "b"])))
println(to_string(group_by([1, 2, 3, 4], (x) => x % 2)))
```

### Aggregates

| Function | Description |
|---|---|
| `sum(xs)` / `min(xs)` / `max(xs)` | numeric aggregates |
| `average(xs)` | mean of a numeric list |
| `clamp(v, lo, hi)` | bound a number to a range |

```olang
let xs = [4, 1, 7]
println(to_string(sum(xs)) + " " + to_string(min(xs)) + " " + to_string(max(xs)))
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
`examples/parmap/` measures the speedup. One deliberate difference: like
`spawn`, the function runs against worker snapshots, so mutating enclosing
state from inside it is not visible to the caller. If several elements
would fail, the error reported is the one `map` would have hit first.
For per-element *effects* rather than values, the language has a
loop-construct twin:
[`par for`](language.md#par-for--parallel-iteration) — the same
fan-out and snapshot semantics, with an implicit barrier.

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
| `set_parallel(n)` | thread budget for parallel-capable builtins |

**olang is eager.** A builtin receives its argument already evaluated, so
`lazy` cannot defer anything — both functions are the identity, kept for
source compatibility. (Earlier releases wrapped values in lazy handles and
immediately forced them; the observable behavior was always eager.)

```olang
let deferred = lazy([1, 2, 3] |> map((x) => x * 10))
println(to_string(force(deferred)))
```

## `str` — strings

The complete string toolkit. Pure functions return strings/ints directly;
the parsers return `Result`.

| Function | Description |
|---|---|
| `str.length(s)` | character count |
| `str.char_at(s, i)` | 1-char string, `""` out of bounds |
| `str.chars(s)` | list of characters |
| `str.substring(s, from, to)` | half-open slice, clamped |
| `str.index_of(s, sub)` / `str.last_index_of` | position or `-1` |
| `str.contains(s, sub)` / `str.count(s, sub)` | search |
| `str.starts_with` / `str.ends_with` | affix tests |
| `str.split(s, sep)` / `str.join(xs, sep)` | list conversion |
| `str.lines(s)` / `str.words(s)` | split on newlines / whitespace |
| `str.trim` / `str.trim_start` / `str.trim_end` | strip whitespace |
| `str.pad_start(s, n, fill)` / `str.pad_end` | pad to width |
| `str.repeat(s, n)` | repetition |
| `str.replace(s, from, to)` / `str.replace_first` | substitution |
| `str.to_upper` / `str.to_lower` / `str.capitalize` | case |
| `str.reverse(s)` | reversed |
| `str.is_empty(s)` | `""` test |
| `str.parse_int(s)` / `str.parse_float(s)` | `Result` parses |
| `str.fmt(template, ...)` | fill `{}` placeholders, display form; `{{`/`}}` escape |

```olang
println(str.pad_start("7", 3, "0"))                 // 007
println(str.substring("olang", 1, 4))               // lan
println(str.replace("a-b-c", "-", "+"))
println(to_string(str.chars("ok")))
println(to_string(unwrap(str.parse_float("2.5")) * 2))
println(str.fmt("{} scored {} ({}%)", "ada", 99, 97.5))
```

## `col` / `colx` — collections

Higher-order operations beyond the global builtins. `colx` is the same API
implemented *in olang* and embedded in the binary (`use colx`) — the two are
differential-tested against each other.

| Function | Description |
|---|---|
| `col.all(xs, p)` / `col.any(xs, p)` | quantifiers (short-circuit) |
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

`mathx` (`use mathx`) is the olang-source twin of the pure subset.

| Group | Functions |
|---|---|
| Basics | `abs` `sign` `min` `max` `pow` `sqrt` `cbrt` |
| Rounding | `floor` `ceil` `round` `trunc` `fract` |
| Integers | `gcd` `lcm` `factorial` |
| Exp/log | `exp` `exp2` `ln` `log` `log2` `log10` |
| Trig | `sin` `cos` `tan` `asin` `acos` `atan` `atan2` |
| Hyperbolic | `sinh` `cosh` `tanh` |
| Angles | `radians` `degrees` |

```olang
println(to_string(math.pow(2, 10)) + " " + to_string(math.sqrt(2.25)))
println(to_string(math.gcd(12, 18)) + " " + to_string(math.factorial(5)))
println(to_string(math.round(2.6)) + " " + to_string(math.fract(2.75)))
```

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
println(doc.name + " has " + to_string(len(doc.tags)) + " tags")
println(to_string(map_get(doc, "age")))          // dynamic key access
let out = unwrap(json.stringify({ ok: true, n: 1 }))
println(out)
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
| `csv.to_json(s)` / `csv.from_json(s)` | format conversion |

```olang
let raw = "name,score\nada,99\nbob,82"
let rows = unwrap(csv.parse_with_headers(raw))
let total = rows |> fold(0, (acc, r) => acc + unwrap(str.parse_int(r.score)))
println(rows[0].name + ", total " + to_string(total))
```

## `re` — regular expressions

Rust regex syntax. Matching functions return `Result`.

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
| Now | `now` `utc_now` `today` `time` |
| Build | `date(y, m, d)` `datetime(y, m, d, h, mi, s)` |
| Parse/format | `parse_date` `parse_datetime` `parse_time` `format_date` `format_datetime` `format_time` |
| Fields | `year` `month` `day` `hour` `minute` `second` `weekday` |
| Arithmetic | `add_days` `add_weeks` `add_months` `add_years` `diff_days` |
| Epoch | `timestamp(dt)` `from_timestamp(n)` |
| Facts | `is_leap_year(y)` `days_in_month(y, m)` |

```olang
let d = unwrap(dates.date(2026, 8, 7))
let later = unwrap(dates.add_days(d, 30))
println(d + " + 30d = " + later)
println("leap 2028: " + to_string(dates.is_leap_year(2028)))
println("days apart: " + to_string(unwrap(dates.diff_days(d, later))))
```

`dates.timestamp(dates.now())` is the idiom for "seconds since the epoch,
now".

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

## `random` — randomness

Seedable (making runs reproducible) and covering the usual distributions.

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

Digests return lowercase hex strings; key operations return `Result`.

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

## `base64` — base64

| Function | Description |
|---|---|
| `base64.encode(s)` / `base64.decode(s)` | standard alphabet (decode returns `Result`) |
| `base64.encode_url_safe` / `decode_url_safe` | URL-safe alphabet |
| `base64.encode_no_pad` / `decode_no_pad` | without `=` padding |
| `base64.is_valid(s)` / `base64.validate(s)` | checks |

```olang
let enc = base64.encode("olang")
println(enc + " -> " + unwrap(base64.decode(enc)))
```

## `fs` — file system

All fallible; all return `Result`. (Examples are `no-run`: they touch the
disk.)

| Group | Functions |
|---|---|
| Files | `read_file` `write_file` `append_file` `copy_file` `move_file` `remove_file` |
| Directories | `create_dir` `create_dir_all` `list_dir` `walk` `glob` `remove_dir` `remove_dir_all` |
| Queries | `exists` `is_file` `is_dir` `file_size` `file_info` |

`fs.walk(dir)` lists every file below a directory (recursive, sorted);
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

## `os` — operating system

| Group | Functions |
|---|---|
| Process | `args` `exit(code)` `pid` `exe_path` `exec(program, args)` |
| Input | `read_line()` — one line from stdin as `Ok(line)`, `Err("eof")` at end; the primitive behind prompts, REPLs, and shells (see `examples/oshell/`) |
| Environment | `get_env` `set_env` `remove_env` `has_env` `list_env` |
| Directories | `cwd` `chdir` `home_dir` `temp_dir` |
| System | `hostname` `username` `os_type` `arch` `family` `path_separator` |

`os.exec` runs an external program to completion and returns
`Ok({ code, stdout, stderr })` — or `Err` if it could not be launched at
all. An optional third argument configures the child:
`#{ "cwd": dir, "stdin": text, "env": #{ name: value } }` (any subset).
`os.args()` is the program's argv (`[script, arg1, ...]`).

```olang no-run
let args = unwrap(os.args())
let target = if len(args) > 1 => args[1] else => "."

let r = unwrap(os.exec("git", ["status", "--short"], #{ "cwd": target }))
if r.code == 0 => print(r.stdout)
else => println("git failed: " + r.stderr)
```

## `http` — HTTP

Requests return `Result` responses with status, headers, and body — and
`http.serve` is a real, blocking HTTP/1.1 server. (`no-run`: network.)

| Function | Description |
|---|---|
| `http.get(url)` / `http.post(url, body)` / `http.put` / `http.delete` | requests |
| `http.request(method, url, body, headers)` | full control |
| `http.parse_url(url)` | split a URL into parts |
| `http.encode_query(map)` / `http.decode_query(s)` | query strings |
| `http.serve(port, handler[, options])` | serve `handler(request)` on a bounded worker pool; blocks the calling program |
| `http.response(status, body)` / `http.response_with_headers(status, body, headers)` | build responses |

```olang no-run
let resp = unwrap(http.get("https://example.com/api/status"))
println(to_string(resp.status))
let data = unwrap(json.parse(resp.body))
println(data.message)
```

### Serving

`http.serve(port, handler)` binds `127.0.0.1:port` (port `0` picks a free
one, reported on stdout as `listening on http://127.0.0.1:PORT`) and blocks
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

The handler returns either a bare string (a `200 text/plain`) or a response
built with `http.response`/`http.response_with_headers` — pass headers as a
map literal so keys like `Content-Type` can contain `-`. A handler error
becomes a `500` and a malformed request a `400`; the server keeps running
through both.

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

See [`examples/webserver/`](../examples/webserver/) for a complete JSON API
with a router (`:id` path parameters) over a SQLite store, and
[`examples/app/`](../examples/app/) for a full-stack issue tracker — the
same server also delivering its own browser frontend from disk.

## `db` — SQLite

A connection handle flows through every call. `:memory:` gives a fresh
in-process database — the example below really runs.

| Function | Description |
|---|---|
| `db.open(path)` | open or create (`":memory:"` for in-memory) |
| `db.execute(conn, sql)` / `db.execute(conn, sql, params)` | run a statement; `?` placeholders |
| `db.query(conn, sql)` / with `params` | rows as a list of maps |
| `db.query_one(conn, sql)` | exactly one row |
| `db.begin(conn)` / `db.commit(conn)` / `db.rollback(conn)` | transactions |
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
    println(map_get(row, "name") + ": " + to_string(map_get(row, "points")))
}
unwrap(db.close(conn))
```

## `testing` — assertions

The same assertions available in [`test` blocks](language.md#testing), as
callable functions — failures raise errors. `reset_tests`/`test_summary`
maintain counters for custom harnesses.

| Function | Description |
|---|---|
| `testing.assert_eq(a, b, msg)` / `assert_ne` | equality assertions |
| `testing.assert_true(x, msg)` / `assert_false` | boolean assertions |
| `testing.assert_ok(r, msg)` / `assert_err` | Result assertions |
| `testing.fail(msg)` | unconditional failure |
| `testing.reset_tests()` / `testing.test_summary()` | counter management |

```olang
testing.assert_eq(2 + 2, 4, "arithmetic works")
testing.assert_ok(str.parse_int("5"), "parses")
println("assertions passed")
```

## `ods` — Series and Frames

The data stack ([design](design/ods.md)): typed, null-aware columns and
tables over contiguous native buffers, measured at NumPy parity for
reductions and within 1.13× of Polars for group-by — see the design doc's
benchmark tables. A **Series** is a 1-D column of `Int`, `Float`, `Bool`,
or `String`; a **Frame** is named, equal-length Series.

Two ideas carry everything. **Operators are vectorized**: `s * 2.0 + 1.0`
runs native kernels over the whole column, `s > 2` yields a Bool-series
mask for `ods.filter`, and a scalar on either side broadcasts. **Nulls are
first-class**: a `()` value in a source list (a missed `map_get`, an empty
CSV cell, a missing JSON key) becomes a null that propagates through
arithmetic and is skipped by reductions.

| Function | Description |
|---|---|
| `ods.series(xs)` | Series from a list or range; dtype inferred, `()` is null |
| `ods.zeros(n)` / `ods.linspace(a, b, n)` | constructors |
| `ods.to_list(s)` / `ods.get(s, i)` / `ods.len(s)` | back to values (null → `()`; negative `i` from the end) |
| `ods.null_count(s)` / `ods.is_null(s)` / `ods.fill_null(s, v)` | null tools |
| `ods.sum` `mean` `var` `std` `min` `max` | reductions, skipping nulls (`var`/`std` are sample, n−1) |
| `ods.quantile(s, q)` | linear interpolation, like NumPy |
| `ods.sort(s)` / `ods.argsort(s)` | ascending, nulls last |
| `ods.take(s, idx)` / `ods.filter(s, mask)` | selection |
| `ods.cumsum(s)` / `ods.dot(a, b)` | running sum; inner product |
| `ods.eq(a, b)` / `ods.ne(a, b)` | *elementwise* equality masks — `a == b` between Series stays structural, like every olang collection |

```olang
let prices = ods.series([12.5, 8.0, 15.25, 4.0])
let taxed = prices * 1.07
println(to_string(ods.mean(taxed)))

let missing = map_get(#{}, "absent")          // Unit → null
let s = ods.series([1.0, missing, 3.0])
println(to_string(ods.null_count(s * 2.0)))   // nulls propagate: 1
println(to_string(ods.mean(s)))               // reductions skip them: 2
println(to_string(ods.to_list(ods.filter(prices, prices > 10.0))))
println(to_string(ods.series(1..4) == ods.series([1, 2, 3])))
```

Frames add the table verbs — all pipeline-friendly:

| Function | Description |
|---|---|
| `ods.frame(pairs)` | from `[[name, series-or-list], ...]` |
| `ods.read_csv(text)` | CSV text → Frame, column types inferred, empty cells null |
| `ods.frame_from_records(xs)` | list of maps (what `json.parse` gives for an array of objects) |
| `ods.to_records(f)` | back to a list of maps |
| `ods.columns` `column` `n_rows` `n_cols` | introspection |
| `ods.select(f, names)` / `ods.with_column(f, name, col)` | shape the columns |
| `ods.filter(f, mask)` / `ods.take(f, idx)` / `ods.head(f, n)` | shape the rows |
| `ods.sort_by(f, col, descending)` | one key, nulls last either way |
| `ods.group_by(f, keys, aggs)` | aggs are `[[out, op, col], ...]` with ops `count` `sum` `mean` `min` `max`; a null key is its own group |
| `ods.join(a, b, on_a, on_b)` / `ods.join_left(...)` | hash joins; null keys never match, collisions suffix `_right` |

```olang
let sales = ods.read_csv("region,amount,qty\neast,25.5,10\nwest,320.0,3\neast,80.0,4\n")
let full = ods.with_column(sales, "revenue",
    ods.column(sales, "amount") * ods.column(sales, "qty"))

let summary = full
    |> ods.group_by("region", [["total", "sum", "revenue"], ["n", "count"]])
    |> ods.sort_by("total", true)
for rec in ods.to_records(summary) {
    println(map_get(rec, "region") + ": " + to_string(map_get(rec, "total")))
}

let tax = ods.frame([["name", ["east", "west"]], ["rate", [0.07, 0.09]]])
let joined = ods.join(summary, tax, "region", "name")
println(to_string(ods.columns(joined)))
```

## `stats` — statistical inference

Distributions, hypothesis tests, and regression, every result pinned
against scipy reference values in the test suite. Tests and fits return
maps — destructure them with `map_get`.

| Function | Description |
|---|---|
| `stats.describe(s)` | count, nulls, mean, std, min, quartiles, max as a map |
| `stats.corr(a, b)` / `stats.cov(a, b)` | Pearson r and sample covariance, pairwise-complete |
| `stats.t_test(a, b)` | Welch's two-sample when `b` is a Series; one-sample vs the null mean when `b` is a number |
| `stats.chi2_test(observed, expected)` | goodness of fit |
| `stats.lm(y, xs)` | OLS with an intercept; `xs` is one Series or a list of them; rows with nulls drop; returns coef/se/t/p_value Series plus `r2`, `adj_r2`, `n` |
| `stats.norm` / `stats.t` / `stats.chi2` / `stats.f` | distribution families |
| `stats.<fam>.pdf` / `cdf` / `ppf` | density, cumulative, quantile (`norm` takes `mu, sigma`; `t`/`chi2` take `df`; `f` takes `d1, d2`) |
| `stats.<fam>.sample(n, ...)` | draw a Series — from the `random` module's stream, so `random.seed` makes it reproducible |

```olang
println(to_string(stats.norm.ppf(0.975, 0.0, 1.0)))   // 1.9599...

let a = ods.series([5.1, 4.9, 6.2, 5.7, 5.5, 4.8, 5.9, 6.1])
let b = ods.series([4.2, 4.8, 4.5, 5.0, 4.4, 4.1, 4.9])
let t = stats.t_test(a, b)
println("p = " + to_string(map_get(t, "p_value")))
println("significant: " + to_string(map_get(t, "p_value") < 0.05))

let x = ods.series([1.0, 2.0, 3.0, 4.0, 5.0])
let y = ods.series([2.1, 3.9, 6.2, 8.1, 9.8])
let fit = stats.lm(y, x)
println("slope = " + to_string(ods.get(map_get(fit, "coef"), 1)))
println("r2 = " + to_string(map_get(fit, "r2")))

random.seed(42)
let draws = stats.norm.sample(1000, 100.0, 15.0)
println(to_string(ods.mean(draws) > 95.0))
```

## `plot` — charts as SVG text

Charts render to complete standalone SVG documents as strings — write one
with `fs.write_file`, serve it over `http`, or return it from the
playground. Defaults follow a colorblind-validated palette with hues
assigned in fixed series order, so a chart is presentable with an empty
options map.

| Function | Description |
|---|---|
| `plot.line(x, y, opts)` / `plot.scatter(x, y, opts)` | one xy series; null pairs drop |
| `plot.lines(x, pairs, opts)` | multiple series as `[[label, y], ...]` (≤ 8), legend included |
| `plot.bar(labels, values, opts)` | labels are a Series or list; null values refuse |
| `plot.hist(s, bins, opts)` | binned counts of a numeric Series |

Options ride in one map — `title`, `x_label`, `y_label`, `width`,
`height` — and an unknown key is an error, because it is always a typo.

```olang
let x = ods.linspace(0.0, 6.28, 50)
let y = ods.series(map(ods.to_list(x), (v) => math.sin(v)))
let svg = plot.line(x, y, #{ "title": "sin(t)", "x_label": "t" })
println(to_string(str.contains(svg, "<svg")))

let by_region = ods.read_csv("region,rev\neast,2415.0\nwest,5167.5\n")
let bars = plot.bar(ods.column(by_region, "region"),
                    ods.column(by_region, "rev"), #{})
println(to_string(str.length(bars) > 500))
```

```olang no-run
// The usual ending: a chart on disk, viewable in any browser.
unwrap(fs.write_file("chart.svg", svg))
```

---

Next: [Packages](packages.md) for multi-file programs and dependencies, or
[Internals](internals.md) for how the runtime executes all of this.
