# Migrating

Part of [the olang book](README.md) · [What's new in 0.88](whats-new.md) ·
[Stability and compatibility](stability.md)

This chapter lists what a program written for an earlier release may
need to change, and how to find each case: first the changes made since
0.88 in the pass that settles what 1.0 freezes, then those of 0.88. Most
programs need nothing. Run `olang check` over a project after
upgrading; it reports the cases it can find.

## Table of contents

- [Since 0.88](#since-088)
  - [`find` answers the element or `()`](#find-answers-the-element-or-)
  - [`reduce` starts from the first element](#reduce-starts-from-the-first-element)
  - [`re.replace` replaces every match](#rereplace-replaces-every-match)
  - [A line starting with `-` or `+` starts a statement](#a-line-starting-with---or--starts-a-statement)
  - [A function declared twice in a file](#a-function-declared-twice-in-a-file)
  - [The bitwise operators' order](#the-bitwise-operators-order)
  - [A block ending in `let` or `;` is `()`](#a-block-ending-in-let-or--is-)
  - [Equality](#equality)
  - [Smaller rulings](#smaller-rulings)
  - [A field of `()` reads `()`](#a-field-of--reads-)
  - [A negative index raises](#a-negative-index-raises)
  - [A condition is a Bool](#a-condition-is-a-bool)
  - [Values print as source](#values-print-as-source)
  - [Six reserved words](#six-reserved-words)
  - [`task.join` answers a Result](#taskjoin-answers-a-result)
  - [Rounding answers an Int](#rounding-answers-an-int)
  - [Stdlib records](#stdlib-records)
  - [Absence and "nothing yet"](#absence-and-nothing-yet)
  - [Dates](#dates)
  - [An assignment answers `()`](#an-assignment-answers-)
  - [`[package] olang` is a minimum](#package-olang-is-a-minimum)
  - [Maps, objects and JSON objects](#maps-objects-and-json-objects)
  - [Top-level names are read when a function runs](#top-level-names-are-read-when-a-function-runs)
  - [Bytes and repeated names at the boundaries](#bytes-and-repeated-names-at-the-boundaries)
- [In 0.88](#in-088)
  - [A missing field of an object is `()`](#a-missing-field-of-an-object-is-)
  - [A module's missing member reads as `()`](#a-modules-missing-member-reads-as-)
  - [The precedence of `??`](#the-precedence-of-)
  - [A lambda stage ends at the next `|>`](#a-lambda-stage-ends-at-the-next-)
  - [Renamed functions](#renamed-functions)
  - [Snapshots of strings](#snapshots-of-strings)
  - [A capability grant olang cannot honour](#a-capability-grant-olang-cannot-honour)
  - [Moving to `.` and `??`](#moving-to--and-)

## Since 0.88

These are listed in the CHANGELOG's Unreleased section, each with what
to change.

### `find` answers the element or `()`

`find(xs, pred)` answered `Ok(element)`, or `Err("NotFound")` when no
element matched; it now answers the element, or `()`, as every lookup's
absence is `()`. A program that unwrapped the result drops the
`unwrap`; one that matched `Ok`/`Err` matches the element against `()`,
or defaults with `??`:

```olang
let first_big = find([3, 8, 2], (x) => x > 5)        // 8, was Ok(8)
let none = find([3, 8, 2], (x) => x > 9) ?? 0         // 0, was Err("NotFound")
println(`${first_big} ${none}`)
```

`unwrap` of the new result raises (it is not a `Result`), so a missed
call site stops at its first run rather than reading on.

### `reduce` starts from the first element

`reduce(xs, init, f)` was `fold` under another name. It now takes no
initial value: `reduce(xs, f)` combines the elements from the first,
and answers `()` for an empty list. A three-argument call is an arity
error that `olang check` reports; rename it to `fold`, which is
unchanged.

```olang
println(reduce([3, 9, 4], (a, b) => if a > b => a else => b))   // 9
println(fold([3, 9, 4], 0, (a, b) => a + b))                    // 16
```

### `re.replace` replaces every match

`re.replace(pattern, text, rep)` replaced the first match while
`str.replace` replaced every occurrence. It now replaces every match;
`re.replace_first` replaces one. `re.replace_all`, the old name for
every match, is deprecated and keeps working. A program that relied on
`re.replace` changing only the first match calls `re.replace_first`.

### A line starting with `-` or `+` starts a statement

A line that begins with `-` or `+` used to continue the expression on
the line above; it now starts a new statement (`-1` on its own line is
the number −1, and a line starting with `+` is a parse error). Move the
operator to the end of the line above:

```olang
let base = 10
let total = base +
    5
println(total)   // 15
```

`olang check` reports each line that no longer parses. A line starting
with `|>`, `&&`, `||`, `??`, `*`, `/`, a comparison or a bitwise operator
still continues the one above.

### A function declared twice in a file

A second top-level `fn` of the same name in one file is an error, from
`olang check` and when the file loads; the second declaration used to
replace the first. Rename one. The REPL still lets a later input
redefine a name.

### The bitwise operators' order

`<<` and `>>` bind below `+` and `-`, and `&`, `^`, `|` below the shifts
and above the comparisons, as in Rust: `1 + 2 << 3` is 24 (it was 17).
Code that parenthesized mixed bitwise and arithmetic operators is
unaffected.

### A block ending in `let` or `;` is `()`

`{ let a = 1 }` and `{ 1; }` were 1; their value is now `()`, and a
function whose body is `{}` answers `()` rather than an empty object. A
function that returned its last `let` names the value on a line of its
own.

### Equality

`Ok(1) == Ok(1)` is `true` (it raised). Comparing a list, map or record
that holds a function, a builtin or a range raises, as comparing the
function itself does (it compared by identity); compare the data. An
Int and a Float are equal only when they are the same number
(`9007199254740993 == 9007199254740992.0` is false), and `contains` and
`col.index_of` find an element by `==` (`contains([1, 2], 1.0)` is
true).

### Smaller rulings

- `a == b == c` and `a < b < c` are parse errors; write `a < b && b <
  c`.
- A named argument to a builtin is an error; pass it by position.
- A pattern that binds a name twice, or an or-pattern whose alternatives
  bind different names, is an error before the program runs.
- `'a'..='z'` matches a one-character String only.
- An enum variant with named fields (`C { n: Int }`) is a parse error;
  declare `C(Int)`.
- `()` annotations, a lambda's return annotation and a tuple
  annotation's length are enforced.
- `to_int` of a Float outside the Int range, and a `<<` that loses a
  bit, raise.

### A field of `()` reads `()`

`s.pp.cur` with `s.pp` absent, `().k`, and `map_get((), k)` answer `()`
where they raised. Guards written only to avoid that raise
(`if x == () => () else => x.f`) still work and may be dropped. `r?.f` is
a parse error; write `(r?).f`.

### A negative index raises

`xs[-1]` read the last element; it now raises, as `xs[len(xs)]` does,
and so do `xs[-1] = v`, a string's, a tuple's, Bytes' and a Series'
negative index, and `col.set`/`col.swap` with one. Read the ends with
`last(xs)` and `first(xs)` (both `()` when empty), or write
`xs[len(xs) - 1]`. `str.substring`, `col.slice`, `bytes.slice` and
`str.char_at` raise on a negative position where they clamped or
counted from the end.

### A condition is a Bool

`if`, `while` and a `match` guard raise on a condition that is not a
Bool, where `0`, `""`, an empty list and `()` were false. Write the
test: `len(xs) > 0`, `x != ()`, `s != ""`, `n != 0`.

### Values print as source

A value printed (by `print`, `show`, `to_string` or `${…}`) is written
as olang source: a string inside a list, map or record is quoted and
escaped, an object is `{ a: 1 }`, a struct `P { x: 1 }` with its fields
in declaration order, an enum value `Circle(2.0)` without its type's
name. A program that compared printed text, or a snapshot of it,
changes with it; a top-level String prints as its text, as before. Take
a string out of a value before comparing it (`match r { Err(e) => e }`
rather than `show(r)`).

### Six reserved words

`await`, `yield`, `async`, `defer`, `const` and `pub` cannot name a
binding, a parameter or a function; the parser says which word. Rename
it. A member after `.` and a map key are not names and are unaffected.

### `task.join` answers a Result

`task.join(t)` is `Ok(value)` when the task returned and `Err(message)`
when it raised; it answered the value bare, and a raise and a returned
`Err` both as `Err`. `task.join_timeout` answers `Ok` of the same, or
`Err("timed out")`. Wrap a join whose value is used in `unwrap(…)`
(`map(task.join)` becomes `map((t) => unwrap(task.join(t)))`), and give
a match's catch-all arm `Ok(v) =>`. Search for `task.join`.

### Rounding answers an Int

`math.floor`, `math.ceil`, `math.round` and `math.trunc` answer an Int,
`math.pow` of two Ints an Int, `math.min`/`math.max` the argument they
pick (an Int stays an Int), and `vec.sum` of Ints an Int. Arithmetic
with a Float still answers a Float, so `math.round(x * 100.0) / 100.0`
is as it was; a rounded value printed or compared by its text alone
changes (`3`, not `3.0`). A Float too large for an Int (`1e300`) raises
where it answered itself, and `math.pow(2, -1)` raises: write
`math.pow(2.0, -1)`. A `to_int(…)` around a rounding is now redundant
and harmless.

### Stdlib records

A record keyed by data is a Map (`http.decode_query`, `os.list_env()`,
`csv.parse_with_headers`'s rows, a response's `headers`), and a result
is a plain object (`os.exec`, `proc.wait`, `pty.wait`, `proc.pipeline`,
`fs.file_info`, `http.parse_url`): a `typeof` test against the old
names (`"ExecResult"`, `"QueryParams"`, …) fails; test the fields
instead. Reading fields with `.` is unchanged. A process that did not
exit by itself has `code: ()` and its `signal` (it was -1): `code == 0`
still tells success. `http.parse_url`'s missing `port`, `query` and
`fragment` are `()` (were 0 and `""`).

### Absence and "nothing yet"

`chan.try_recv` on an empty channel answers `Ok(())` where it answered
`Err("channel is empty")`. A loop that drains a channel until `Err`
would now spin: add an `Ok(()) =>` arm that stops, before `Ok(v) =>`.
`re.find` with no match is `Ok(())` and `re.captures` with no match
`Ok(())` (test `== ()`, or `?? ""`), a group that took no part `()`,
and `str.char_at` past the end `()`. Search for `try_recv`,
`"channel is empty"`, `re.find`, `re.captures` and `char_at`.

### Dates

`dates.timestamp` answers milliseconds and `dates.from_timestamp` takes
milliseconds; a program that used seconds calls `dates.timestamp_s` and
`dates.from_timestamp_s`. `dates.datetime`, `dates.from_timestamp` and
`dates.parse_datetime` answer RFC 3339 with the offset
(`2026-01-02T03:04:05+00:00`, where they answered
`2026-01-02 03:04:05`): text compared or sliced by position (a
`str.substring(…, 11, 19)` for the time) still finds the time where it
was, and the day in the first ten characters. Search for
`dates.timestamp`, `from_timestamp`, `dates.datetime` and
`parse_datetime`.

### An assignment answers `()`

`x = v` answers `()`, where it answered `v`. A block, branch or
function whose last line is an assignment and whose value something
reads now answers `()`: add the name after it (`{ count = count + 1;
count }`). `let y = (x = 5)` and `f(x = 5)` pass `()`.

### `[package] olang` is a minimum

`olang = "0.88"` in olang.toml means 0.88.0 or newer, where it meant
any 0.88.x. A pin written as a requirement (`"^0.88"`, `"=0.88.1"`,
`">=0.87, <0.90"`) is now an `olang check` error: write the oldest
version the project runs on.

### Maps, objects and JSON objects

A record pattern now matches a Map too, so a `match` whose `{ … }` arm
came after an arm meant for maps may take maps sooner: order the arms
from the most specific. A map literal with a whole Float key (`#{ 2.0:
x }`) has the key `"2"` where it had `"2.0"`.

### Top-level names are read when a function runs

A function that reads a top-level `let mut` sees the value it has at the
call, where it saw the value it had when the function was declared; a
top-level name bound again by a later `let` is seen rebound. A program
that relied on the old snapshot binds the value it wants under a name of
its own (`let base0 = base`, then read `base0`) or passes it in.

### Bytes and repeated names at the boundaries

`crypto.random_bytes` answers Bytes: index it or `bytes.to_list` it where
a list of Ints was read. A query name given more than once
(`http.decode_query`, `req.query`) and a response header sent more than
once are a list of their values: a reader expecting a string there
takes `xs[0]` or the last. `http.encode_query` raises on a nested value
where it dropped it silently.

## In 0.88

What a program written for 0.87 may need to change for 0.88.

### A missing field of an object is `()`

Reading a field an anonymous object (`{ … }`) or a parsed JSON object
does not have answered an error; it now answers `()`, as `map_get` and a
map's `m.key` do. A program that relied on the error to stop has to test
for the field (`o.k == ()`, or `o.k ?? default`). A declared struct's
missing field still raises. `olang check` warns on a read of a key no
construction of the value has (advisory class `keys`) and of a field a
declared struct never has (class `shape`).

```olang
let o = { name: "ada" }
println(o.age == ())        // true: absent, not an error
println(o.age ?? 36)        // 36
```

This rule is settled for 1.0 ([Stability](stability.md#settled-before-10)).

### A module's missing member reads as `()`

`plugin.init` for a module without `init` answered an error; it now
answers `()`, so a plugin host can ask a module it loaded whether it
has a hook. Calling the missing member still raises and names the
nearest member (`fs.reed_file(p)` says "did you mean 'read_file'"), as
does reading a field of it or calling a method on it.

### The precedence of `??`

`a ?? b` is new in 0.88. Its level is below `+`, `*`, `|>` and the
ranges and above the comparisons, so `m.n ?? 0 > 3` is `(m.n ?? 0) >
3` and `m.n ?? 0 + 1` is `m.n ?? (0 + 1)`. Parenthesize the default when
arithmetic is meant on the result: `(m.n ?? 0) + 1`.

```olang
let m = #{ "n": 4 }
println(m.n ?? 0 + 1)       // 4: the default is 0 + 1
println((m.n ?? 0) + 1)     // 5
```

### A lambda stage ends at the next `|>`

An unparenthesized lambda written as a pipeline stage took the stages
after it into its body; the `|>` after it now begins the next stage.
The two readings give the same value whenever the lambda's body is
arithmetic feeding the pipe; they differ when the body ends with an
`if` (the later stages ran only in its last branch), or when a stage
after it used the lambda's parameter. To keep a pipeline inside the
lambda, write it in the body's parentheses or braces:

```olang
fn wrap(x) = [x]
println(5 |> (v) => if v > 3 => v else => 0 |> wrap)       // [5]: wrap is the next stage
println(5 |> (v) => (if v > 3 => v else => 0 |> wrap))     // 5: wrap inside the else branch
```

Over the workspace's programs 33 stages read differently; 29 give the
same value, and the other four were a bug the new reading fixes.

### Renamed functions

`map_path` is `map_get_in` under an older name, deprecated in 0.88. It
keeps working; `olang check` warns at each call, and `olang fix
deprecated` renames them. The [stdlib naming
audit](roadmap.md#stdlib-naming-audit) lists further candidates; none of
them is renamed in 0.88.

### Snapshots of strings

`testing.snapshot` writes a String as its text, without the quotes it
had. A snapshot recorded quoted still matches, and is rewritten bare the
next time snapshots are refreshed (`OLANG_UPDATE_SNAPSHOTS=1`), so
nothing needs to change at once.

### A capability grant olang cannot honour

A `[capabilities]` block olang cannot honour stops the program before
it runs, where it used to run with no restriction at all: a key the gate
does not know (`filesystem = false` for `fs`), a value it does not take
(`net = "no"`), or an `olang.toml` that does not parse while it declares
capabilities. `olang check` fails on each and names the problem; fix the
manifest. A tool's own table kept in `olang.toml` belongs under its own
name — olang Studio's plugins write `[plugin.capabilities]`.

### Moving to `.` and `??`

`map_get` and `map_get_or` keep working. To move a project to the new
forms, run `olang fix maps` (with `--check` first to see what it would
change): `map_get(x, "k")` becomes `x.k` and `map_get_or(x, "k", d)`
becomes `x.k ?? d`, layout and comments kept, and a default that may
have an effect is reported rather than rewritten
([tooling](tooling.md#olang-fix-maps)).
