# Migrating to 0.88

Part of [the olang book](README.md) · [What's new in 0.88](whats-new.md) ·
[Stability and compatibility](stability.md)

This chapter lists what a program written for 0.87 may need to change
for 0.88, and how to find each case. Most programs need nothing: the
changes below are settled now because 1.0 freezes them. Run `olang
check` over a project after upgrading; it reports the cases it can
find.

## Table of contents

- [A missing field of an object is `()`](#a-missing-field-of-an-object-is-)
- [A module's missing member reads as `()`](#a-modules-missing-member-reads-as-)
- [The precedence of `??`](#the-precedence-of-)
- [A lambda stage ends at the next `|>`](#a-lambda-stage-ends-at-the-next-)
- [Renamed functions](#renamed-functions)
- [Snapshots of strings](#snapshots-of-strings)
- [Moving to `.` and `??`](#moving-to--and-)

## A missing field of an object is `()`

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

This rule is settled for 1.0 ([Stability](stability.md#settled-in-088)).

## A module's missing member reads as `()`

`plugin.init` for a module without `init` answered an error; it now
answers `()`, so a plugin host can ask a module it loaded whether it
has a hook. Calling the missing member still raises and names the
nearest member (`fs.reed_file(p)` says "did you mean 'read_file'"), as
does reading a field of it or calling a method on it.

## The precedence of `??`

`a ?? b` is new in 0.88. Its level is below `+`, `*`, `|>` and the
ranges and above the comparisons, so `m.n ?? 0 > 3` is `(m.n ?? 0) >
3` and `m.n ?? 0 + 1` is `m.n ?? (0 + 1)`. Parenthesize the default when
arithmetic is meant on the result: `(m.n ?? 0) + 1`.

```olang
let m = #{ "n": 4 }
println(m.n ?? 0 + 1)       // 4: the default is 0 + 1
println((m.n ?? 0) + 1)     // 5
```

## A lambda stage ends at the next `|>`

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

## Renamed functions

`map_path` is `map_get_in` under an older name, deprecated in 0.88. It
keeps working; `olang check` warns at each call, and `olang fix
deprecated` renames them. The [stdlib naming
audit](roadmap.md#stdlib-naming-audit) lists further candidates; none of
them is renamed in 0.88.

## Snapshots of strings

`testing.snapshot` writes a String as its text, without the quotes it
had. A snapshot recorded quoted still matches, and is rewritten bare the
next time snapshots are refreshed (`OLANG_UPDATE_SNAPSHOTS=1`), so
nothing needs to change at once.

## Moving to `.` and `??`

`map_get` and `map_get_or` keep working. To move a project to the new
forms, run `olang fix maps` (with `--check` first to see what it would
change): `map_get(x, "k")` becomes `x.k` and `map_get_or(x, "k", d)`
becomes `x.k ?? d`, layout and comments kept, and a default that may
have an effect is reported rather than rewritten
([tooling](tooling.md#olang-fix-maps)).
