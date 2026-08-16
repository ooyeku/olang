# Openness

Part of [the olang book](README.md).

A distinguishing design goal of olang is that three aspects of a program are
available to olang programs as inspectable data: its code, its compiled
artifacts, and its execution. This chapter describes each and links to its
reference.

- **Open code:** `meta.parse` returns a program's syntax tree as olang
  values.
- **Open artifacts:** a compiled binary embeds its exact source and
  declares the capabilities it is allowed to use.
- **Open execution:** `olang record` logs a run's nondeterministic
  inputs, and `olang replay` reproduces the run from that log.

Each capability follows from an existing design decision: a stable
grammar, immutable values, and a small, explicit effect boundary. This
page summarizes each capability and links to its reference.

See also: [Packages and dependencies](packages.md),
[Command-line tooling](tooling.md),
[Standard library reference](stdlib.md).

## Open code: the program as data

olang's grammar is stable (see [Stability](stability.md)), so the parse
tree is a supported data format. `meta.parse(source)` returns a program as
a list of `kind`-tagged maps. Process these maps with the same `map`,
`filter`, `fold`, and `match` operations you use on any other data. Use
this format to write linters, codemods, and import extractors in olang
instead of as compiler changes.

The following example lists a file's imports:

```olang
// List a file's imports.
let program = unwrap(meta.parse("use geometry { area }\nuse fmt\nfn f() = 1"))
for node in program |> filter((n) => map_get(n, "kind") == "use") {
    println(map_get(node, "path") + " " + show(map_get(node, "items")))
}
// geometry ["area"]
// fmt ["*"]
```

For the full node vocabulary, see the [meta
reference](stdlib.md#meta--the-program-as-data-the-open-ast). For a
complete linter, see [examples/metatool](../examples/metatool/main.ol).

The toolchain uses the same format. `olang check --rules` runs
project-specific lint rules, written in olang over the meta AST, alongside
the built-in type checker. See [Project rules](tooling.md#project-rules).

## Open artifacts: the transparent binary

`olang build` embeds a program's complete source, its `olang.toml` and
`olang.lock`, its capability manifest, and a checksum over all of them.
Use `olang inspect` to read and verify these files:

```bash
olang inspect ./tool --source     # Print the embedded source.
olang inspect ./tool --verify     # Verify the checksum.
olang inspect ./tool --against .  # Compare the binary to a source tree.
olang inspect ./tool -o dir/      # Extract source, manifest, and lockfile.
```

The checksum covers the source, the manifest, and the lockfile together,
so `--verify` detects a change to the embedded capability grant as well as
a change to the code. `--against <dir>` compares the embedded files to a
checkout and reports whether the binary was built from that source tree.

Capabilities restrict what a program can do. A `[capabilities]` block in
`olang.toml` controls access to the effectful parts of the standard
library: `fs`, `net`, `db`, `proc`, and environment access. Pure
computation is never restricted. When no manifest is present, all
capabilities are allowed, so existing code is unaffected. A dependency can
be granted fewer capabilities than the application, but never more. For
details, see [Capabilities](packages.md#capabilities).

## Open execution: the timeline

olang programs are deterministic given their inputs. Values are immutable,
closures capture by value, and the only observable nondeterminism comes
from a small, fixed set of standard-library calls. As a result, a run can
be recorded and reproduced exactly:

```bash
olang record program.ol             # Record a run; writes program.olt.
olang record program.ol -o bug.olt  # Or name the trace explicitly.
olang replay bug.olt                 # Reproduce the run from the recording.
```

`olang record <file>` is the command form; `olang --record <trace> <file>`
is the equivalent run option when you are already invoking the file
directly.

Replay produces the same random values, timestamps, and environment as the
original run. The `.olt` trace embeds the program source, so replay works
on a machine that does not have the program. A run that crashes is still
recorded, so the failure reproduces. If a program's sequence of effects no
longer matches the trace, replay stops at that point and reports the
divergence. For the full model, see [The Open
Timeline](tooling.md#olang-record-and-olang-replay--the-open-timeline).

## Design decisions

The three capabilities depend on four design decisions:

| Decision | Capability it enables |
|---|---|
| Stable syntax | The AST shapes are published as a data format (open code). |
| Immutable, acyclic values | A recorded value is never a live handle, and a state snapshot is a pointer clone (open execution). |
| A small, explicit effect boundary | A single place to record inputs and to enforce capabilities (open execution and open artifacts). |
| Source-carrying binaries | No decompilation is required to read a binary (open artifacts). |

For the roadmap and remaining work, see [the openness
campaign](roadmap.md#the-openness-campaign--what-open-language-means-mechanically).
