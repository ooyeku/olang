# The olang Standard Library Reference

Everything the runtime ships: the global builtins (always in scope) and the
sixteen native modules plus two olang-source modules compiled into the
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

### Lazy evaluation

| Function | Description |
|---|---|
| `lazy(expr)` | defer a computation |
| `force(l)` | evaluate a deferred value |
| `set_parallel(n)` | thread budget for parallel-capable builtins |

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
| `http.serve(port, handler)` | serve `handler(request)` forever (blocking) |
| `http.response(status, body)` / `http.response_with_headers(status, body, headers)` | build responses |

```olang no-run
let resp = unwrap(http.get("https://example.com/api/status"))
println(to_string(resp.status))
let data = unwrap(json.parse(resp.body))
println(data.message)
```

### Serving

`http.serve(port, handler)` binds `127.0.0.1:port` (port `0` picks a free
one, reported on stdout as `listening on http://127.0.0.1:PORT`) and blocks,
handling requests **sequentially** over **persistent connections**:
HTTP/1.1 keep-alive is honored (a client opts out with `Connection: close`;
idle connections close after a few seconds). The handler receives a request
struct:

| Field | Contents |
|---|---|
| `req.method` | `"GET"`, `"POST"`, ... (uppercased) |
| `req.path` | the path, without the query string |
| `req.query` | map of decoded query parameters |
| `req.headers` | map of headers, keys lowercased |
| `req.body` | the request body as a string |

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
http.serve(8080, handle)   // blocks; Ctrl-C to stop
```

See [`examples/webserver/`](../examples/webserver/) for a complete JSON API
with a router (`:id` path parameters) over a SQLite store.

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

---

Next: [Packages](packages.md) for multi-file programs and dependencies, or
[Internals](internals.md) for how the runtime executes all of this.
