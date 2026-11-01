# Error messages

This chapter lists the errors olang reports, what usually causes each
one, and what to write instead. olang has no numbered error codes. An
error is known by its kind (the first words of its message, such as
`Type error:`), and the kinds are few. The findings of `olang check`
are the exception: each carries a class name, listed
[at the end](#olang-check-classes).

The wording of a message is not part of olang's compatibility promise
and may be improved in any release ([Stability](stability.md)); the
kinds and the `olang check` classes are stable. The messages below are
those of olang 1.0.0-rc.6.

Part of [the olang book](README.md) · [Common pitfalls](pitfalls.md) ·
[olang for LLMs](llms.md).

## Table of contents

- [How an error is reported](#how-an-error-is-reported)
- [Parse errors](#parse-errors)
- [Undefined variable](#undefined-variable)
- [Type errors](#type-errors)
- [Runtime errors](#runtime-errors)
- [Pattern match failed](#pattern-match-failed)
- [Errors as values](#errors-as-values)
- [olang check classes](#olang-check-classes)

## How an error is reported

An error that stops a program is printed to standard error, and the
program exits with status 1 ([exit codes](tooling.md#exit-codes)).
There are two shapes. A program that does not parse never starts, and
gets a `Parse Error` report with the line, the column and, often, a
hint:

```text
═══ Parse Error ═══
  File: counter.ol
  Error: expected unary expr
  Location: Line 2, Column 8

  Code Context:
    2   │  total += 1
        │         ^
```

A program that parses and then fails gets an `Execution Error` report:
the message after `×`, the line it happened on, and the functions that
were running, outermost first:

```text
═══ Execution Error ═══
  × Runtime error: Division by zero
   ╭─[demo.ol:2:5]
 2 │     x / 0
   ·     ▲
   ╰────

  Call stack (outermost first):
    → outer
    → inner
```

Inside `attempt(f)`, a spawned task or an `http.serve` handler, the same
error becomes a value instead: `attempt(() => 1 + "a")` is
`Err("cannot add Int and String; use to_string(...) to convert")`, the
message without its kind ([Error
handling](language.md#where-failure-stops-the-boundaries)).

## Parse errors

A parse error means the text is not olang. Most come from writing
another language's syntax.

| Message | Usual cause | Write instead |
|---|---|---|
| `expected an operator or a function call` | `if x > 0 { ... }`, `c ? a : b`, `a and b` | `if x > 0 => ...`, `if c => a else => b`, `a && b` |
| `expected param pattern` | `fn f(x) { ... }` | `fn f(x) = { ... }` |
| `expected unary expr` | `x += 1` | `x = x + 1` |
| `expected argument` | `map(xs, fn(x) => x)` | `map(xs, (x) => x)` |
| `expected fn kw` | trait methods separated by `;` | one method per line |
| `Unclosed template interpolation — ... templates do not nest` | a template inside `${...}` | bind the inner template to a name first |
| `a spread ... is not supported in a declared struct's literal` | `P { ...p, x: 1 }` | `map_set(p, "x", 1)` |

## Undefined variable

`Undefined variable: NAME` means nothing called `NAME` is in scope where
it is read. The usual causes:

- A misspelling, or a `let` value read above the line that binds it.
  (Functions can be called from anywhere in the file; values cannot.)
- `None`, `null` or `nil`, which olang does not have. Absence is `()`.
- A Python or Rust builtin that olang spells differently: `int(x)` is
  `to_int(x)`, `sqrt(x)` is `math.sqrt(x)`, `set(xs)` is `col.unique(xs)`.
- A bare `impl Type { ... }` block used to fail here as `Undefined
  variable: impl`. It is now refused before the program runs, at the
  `impl`: `olang has no inherent impl blocks: methods come from a trait,
  so declare one (trait Name { fn method(self) }) and write impl Name
  for Type { ... }` ([Methods come only from a
  trait](pitfalls.md#methods-come-only-from-a-trait)).

## Type errors

`Type error:` means a value of the wrong kind reached an operation.

| Message | Usual cause | Write instead |
|---|---|---|
| `cannot add Int and String; use to_string(...) to convert` | `"n = " + n` | `"n = " + show(n)` or `` `n = ${n}` `` |
| `cannot iterate over a Map — iterate its pairs with for (k, v) in entries(m)` | `for (k, v) in m` | `for (k, v) in entries(m)` |
| `Cannot access field 'len' on non-struct value` | `xs.len()`, `s.split(",")`, `xs.push(v)` | `len(xs)`, `str.split(s, ",")`, `xs = xs + [v]` |
| `P has no field or method 'y'` | a field the struct does not declare, or a method no trait gave it | check the declaration and the `impl` |
| `struct 'P' is missing field 'x' (declared fields: x)` | a struct literal without every field | give every field |
| `a condition must be a Bool, got Int: say what is meant` | `if xs => ...` on a number, list or string | `if len(xs) > 0 => ...` |
| `parameter 'x' of f expects Int, got String` | an argument that contradicts an annotation | convert it, or fix the annotation |
| `unwrap_or: first argument must be a Result type (Ok or Err)` | `unwrap_or(m.key, 0)` on a map read | `m.key ?? 0` |
| `Index must be an integer` | slicing, `xs[1..3]` | `take(skip(xs, 1), 2)` |
| `Only lists, tuples, and strings can be indexed` | `[i]` on a number or `()` | check the value first |
| `module 'str' has no member 'upper'` | a function the module does not have | look it up in [the reference](stdlib.md) (`str.to_upper`) |
| `max: cannot take max of an empty list` | `min`, `max` or `average` of `[]` (each names itself) | check `len(xs) > 0` first |
| `Invalid binary operation: cannot apply '*' to String and Int` | an operator between kinds it does not take | `str.repeat(s, n)` for strings |

## Runtime errors

`Runtime error:` means the program did something that has no answer,
or asked to stop.

| Message | Usual cause | Write instead |
|---|---|---|
| `Division by zero`, `Modulo by zero` | `/` or `%` by zero, for floats too | check the divisor |
| `Index 5 out of bounds for list of length 2` | an index past the end, or a negative one | `last(xs)`, or check `len(xs)` |
| `Integer overflow in addition (bigint.of gives arbitrary precision)` | an Int past 64 bits | `bigint`, or Floats |
| `cannot assign to 'x': it is not declared mutable` | `x = ...` after a plain `let x` | `let mut x = ...`, or a fresh `let x = ...` |
| `cannot assign to 'n': it is captured from an enclosing scope, and functions capture by value` | a lambda assigning to an outer name | return the new value, or use a `cell` |
| `Missing required argument: y` | a call with too few arguments | pass it, or give the parameter a default |
| `Unwrap failed on error: "bad"` | `unwrap` of an `Err` | `match`, `unwrap_or` or `?` |
| `head: cannot get head of empty list` | `head` or `tail` of `[]` | `first(xs)`, which answers `()` |
| `Assertion failed: Integer(1) != Integer(2)` | a failed `assert_eq` (each assertion has its own wording) | the test caught something |
| your own text | `raise("your own text")` | `raise` is for misuse; expected failures return `Err` |

## Pattern match failed

`Pattern match failed` means a `match` had no arm for the value. End a
`match` with `_ => ...`, or with a name, which matches anything. When
the value is an enum or a union of literals, `olang check` names the
missing cases (class `exhaustiveness`).

## Errors as values

Many failures are not errors in this sense at all. Functions that read
the outside world or parse text answer `Err(message)` and let the
program carry on, and the program decides what to do:

| Call | Its `Err` |
|---|---|
| `str.parse_int("x")` | `Err("cannot parse 'x' as an integer")` |
| `str.parse_float("x")` | `Err(...)`, also for a value out of range |
| `fs.read_file("/no/such")` | `Err("Failed to read file '/no/such': No such file or directory (os error 2)")` |
| `json.parse("{bad")` | `Err("JSON parse error: key must be a string at line 1 column 2")` |

A missing map key is not an error either: `m.key` is `()`, and
`m.key ?? default` supplies a value.

## olang check classes

`olang check` reports what it can prove before a program runs. Each
finding has a class, the `code` field of `olang check --format json`,
and the class names are stable. The errors are `syntax`, `macro`,
`scope`, `runtime`, `type` and `manifest` (an `olang.toml` problem).
The advisory classes, which warn but let the program run, are
`exhaustiveness`, `shape`, `keys`, `result`, `shadow`, `copy`,
`no_effect` and `tier`; `unused` marks a name nothing reads, and
`allow` an allow comment that names no class. [Promoting
advisories](tooling.md#promoting-advisories) says what each advisory
class finds and how a project turns one into an error, and [Allowing
one line](tooling.md#allowing-one-line) how to silence one finding.
