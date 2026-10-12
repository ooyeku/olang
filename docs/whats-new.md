# What's new in 0.88

Part of [the olang book](README.md) · [Migrating](migrating.md#in-088) ·
[Stability and compatibility](stability.md)

This chapter summarises what olang 0.88 adds and changes, for a reader
who knows 0.87. It is the last release before 1.0: what it settles is
what 1.0 freezes. The complete record, with the evidence behind each
change, is in [CHANGELOG.md](../CHANGELOG.md); what a program may need
to change is in [Migrating](migrating.md#in-088).

## Table of contents

- [Records and maps](#records-and-maps)
- [The language](#the-language)
- [The standard library](#the-standard-library)
- [The checker and the tools](#the-checker-and-the-tools)
- [The execution tiers](#the-execution-tiers)

## Records and maps

A map, an object and a parsed JSON object are read and written the same
way, with less ceremony:

- `m.key` reads a map's key: exactly `map_get(m, "key")`, `()` when the
  key is absent. A missing field of an object or a parsed JSON object is
  `()` as well; a declared struct's missing field still raises.
- `a ?? b` is `b` when `a` is `()`, else `a`, and `b` is evaluated only
  when needed. Its level is below the arithmetic and the pipeline and
  above the comparisons: `m.n ?? 0 > 3` is `(m.n ?? 0) > 3`.
- A spread builds a copy with keys laid over it: `#{ ...m, "k": v }` and
  `{ ...o, k: v }`, a later key winning.
- `map_set_in`, `map_get_in` and `map_update` write and read nested
  maps, in place when nothing else holds the map.
- `olang fix maps` rewrites `map_get(x, "k")` as `x.k` and
  `map_get_or(x, "k", d)` as `x.k ?? d` across a project.

```olang
let config = #{ "server": #{ "port": 8080 } }
let port = config.server.port ?? 80
let timeout = config.server.timeout ?? 30
let updated = #{ ...config, "debug": true }
println(`${port} ${timeout} ${updated.debug}`)   // 8080 30 true
```

## The language

- `xs[i] = v` replaces one element of a `let mut` list, in place when
  nothing else holds it; `m.k[i] = v` does the same one level into a map
  or object.
- An unparenthesized lambda written as a pipeline stage ends at the next
  `|>`, however its body ends ([Pipelines](language.md#pipelines)).
- `if a == (x, y) => …` compares with the tuple: on the right of a
  comparison, a parenthesized group followed by `=>` is an operand.
- Tuples order lexicographically under `<`, `<=`, `>`, `>=` and `sort`,
  which is how a list is sorted by several keys.
- `raise(message)` stops with a runtime error whose message is
  `message`, word for word; `attempt` answers it as `Err(message)`.
- A module's missing member reads as `()`, as a map's missing key does;
  calling one still raises and names the nearest member.

```olang
let mut counts = [0, 0, 0]
for word in ["a", "bb", "a", "ccc"] {
    counts[len(word) - 1] = counts[len(word) - 1] + 1
}
println(counts)                                     // [2, 1, 1]
println(sort([(2, "b"), (1, "z"), (2, "a")]))       // [(1, "z"), (2, "a"), (2, "b")]
fn check(n) = if n >= 0 => n else => raise("check: n must not be negative, got " + show(n))
println(attempt(() => check(-1)))                   // Err("check: n must not be negative, got -1")
```

## The standard library

- `abs`, `all` and `any` are global, beside `min`, `max` and `clamp`.
- `eprint` and `eprintln` write to standard error.
- `str.is_ascii`, `str.from_char_code`, `fs.real_path`,
  `os.stdin_is_tty`, `bytes.to_hex` and `bytes.from_hex`.
- `json.stringify_pairs([(k, v), …])` writes one JSON object with its
  keys in the order given.
- `testing.snapshot` keeps a String as its text, and takes the caller's
  update flag: `testing.snapshot(name, value, #{ "update": true })`.
- A `db` connection no value holds any more is closed.
- `map_path` is deprecated in favour of `map_get_in`
  ([Deprecated names](stdlib.md#deprecated-names)).

## The checker and the tools

`olang check` (and the language server, which publishes the same
findings) reports, before a program runs:

- a wrong argument count to a builtin or a stdlib function;
- a `.key` read that no construction of the value has;
- a field read that a declared struct never has;
- a parameter or a pattern binding that shadows a stdlib module and is
  then read as that module;
- a call of a deprecated name.

`olang check --format json` prints every problem as data. `olang fix
deprecated` renames deprecated calls. A path dependency's own path
dependencies are followed by every command that installs. The REPL's
`:env`, `:search` and `:tutorial` print the same on every run.

## The execution tiers

More programs stay on the compiled tiers: `return` and `?`, the bitwise
operators, `loop` and `break value` now compile to bytecode (the
bitwise operators to native code too), and a refusal names the
construct it could not compile. Compiled bytecode is kept between runs,
and a callee compiles at its first call rather than with its caller.
