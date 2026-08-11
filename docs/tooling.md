# Tooling

The developer tools shipped inside the `olang` binary: the test runner and
the formatter. (Package commands live in `otc` — see
[Packages](packages.md).)

Part of [the olang book](README.md) ·
[Language](language.md) · [Standard Library](stdlib.md) ·
[Internals](internals.md)

## `olang test`

Discovers and runs [`test` blocks](language.md#testing) across a directory:

```bash
olang test              # everything under the current directory
olang test examples/    # or a specific directory / single file
```

A *test file* is any `.ol` file containing a top-level `test "name" { ... }`
block. Each test file runs in a fresh interpreter, top to bottom — helper
functions, fixtures, and imports work exactly as they do under
`olang <file>`, and files inside a package get the package's dependencies.
Files without test blocks are not executed at all.

Under the runner, a failing block **records its failure and execution
continues**, so one red test doesn't hide the rest (in a normal
`olang <file>` run, a failing assertion still aborts — the runner is what
changes the policy). Programs see a bare `os.args()`, so a file that
branches on arguments takes its no-argument path.

```text
examples/markdown/main.ol
  ✓ markdown conversion
tests/math_test.ol
  ✓ doubling works
  ✗ deliberately red
      Runtime error: wrong on purpose
────────────────────────────────────────
  2 passed, 1 failed  (2 test files, 35ms)
```

The exit code is non-zero when anything fails — `olang test` slots directly
into CI. An error *outside* any test block (setup code failed) is reported
as a file error and counts as a failure.

Conventions that work well: name dedicated suites `*_test.ol`, or keep a
`test` block next to the code it guards (the markdown example self-checks
its conversion contract this way).

## `olang fmt`

A conservative formatter for whitespace hygiene:

```bash
olang fmt                 # format the current directory, in place
olang fmt src/ lib/       # specific paths
olang fmt --check .       # report what would change; non-zero exit (CI)
```

What it does — only ever *outside* multi-line string literals:

- CRLF line endings become LF
- trailing spaces and tabs are stripped
- leading tabs become 4 spaces (the book's indentation unit)
- runs of blank lines collapse to one
- the file ends with exactly one newline

What it deliberately does **not** do: re-indent, re-wrap, or reflow code.
Comments and layout are the author's; template-string and raw-string
interiors are content and are never touched.

**The safety property is absolute**: before writing, the formatted source
is re-parsed and must produce an AST identical to the original's. A file
whose formatting would change its meaning — or that doesn't parse — is
skipped and reported, never modified. Formatting is idempotent: a formatted
tree passes `--check`.

## `olang check`

The static side of gradual typing — reports provable type-annotation
violations before the program runs:

```bash
olang check               # every .ol file under the current directory
olang check src/ app/     # specific paths or a single file
```

```text
  × parameter 'a' of add expects Int, got String
   ╭─[src/main.ol:2:1]
 1 │ fn add(a: Int, b: Int) -> Int = a + b
 2 │ let total: Int = add("one", 2)
   · ▲
   · ╰── this would fail at runtime
   ╰────
```

Its discipline is **no false positives**: a diagnostic appears only when
an annotation is provably dishonest — a literal argument against an
annotated parameter, an annotated binding initialized with a known-type
value, a declared return contradicted by what the body provably produces,
a call to a known function with the wrong number of arguments. Anything
it cannot prove stays silent; unannotated, dynamic code is never judged.

The checker also goes where the runtime deliberately doesn't: **element
types**. The runtime checks `List<Int>` shallowly ("a List", O(1)), but
the checker decomposes literals element by element — `[1, "a", 3]`
against `List<Int>` reports `element 1 of let binding 'xs' expects Int,
got String`. The same applies to `Map<K, V>` entries, tuple arity and
elements, struct-literal fields, and nested combinations, with the path
spelled out. Deep types flow through annotated bindings and known return
types, so `f(ys)` is flagged when `f` wants `List<Int>` and `ys` is
declared `List<String>`.

The two kinds of finding are labeled distinctly: violations the
runtime's own checks would also catch say *"this would fail at runtime"*
and carry the runtime's exact error text, word for word; element-level
breaks the shallow runtime checks let pass say *"the annotation's
promise is broken here"* — the program may run, but an annotation in the
flagged chain is provably false.

The same checker runs in the language server, so editors surface these
as error squiggles while you type. Exit is non-zero when a violation or
parse error is found, so `olang check` slots directly into CI.

## The examples harness

`examples/run_all.ol` — a test harness written *in olang* — runs every
example program (standalone scripts and packages) in its own subprocess and
reports a pass/fail summary. It complements `olang test`: the harness
checks that whole programs run; the test runner checks `test`-block
assertions. See [examples/README.md](../examples/README.md).
