# Openness — what "Open Language" means, mechanically

olang is the **Open Language** — and it means the word literally, at three
layers, as mechanical facts rather than a slogan:

- **Open code** — a program's own structure is a stable, public data
  format your olang code can read and transform.
- **Open artifacts** — a compiled binary carries its own source and
  declares exactly what it is allowed to touch; it can be neither a black
  box nor a silent over-reacher.
- **Open execution** — any run can be recorded and replayed bit-for-bit,
  anywhere.

No other language offers all three, and the reason is not ambition — it
is that each one falls out of a decision olang already made. This chapter
is the map; each pillar's mechanics live in its own chapter, linked below.

Part of [the olang book](README.md) · [Packages](packages.md) ·
[Tooling](tooling.md) · [Standard Library](stdlib.md)

## Open code — the program as data

Because olang [stabilized its syntax early](stability.md), the parse tree
can be a stable public format. `meta.parse(source)` hands a program back
as ordinary olang values — a list of `kind`-tagged maps you walk with the
same `map`/`filter`/`fold`/`match` you use on any data. Linters, codemods,
and import extractors become olang scripts, not compiler changes.

```olang
// `otc deps` — list a file's imports — in four lines.
let program = unwrap(meta.parse("use geometry { area }\nuse fmt\nfn f() = 1"))
for node in program |> filter((n) => map_get(n, "kind") == "use") {
    println(map_get(node, "path") + " " + show(map_get(node, "items")))
}
// geometry ["area"]
// fmt ["*"]
```

The full node vocabulary is in [the `meta`
reference](stdlib.md#meta--the-program-as-data-the-open-ast); a linter
that counts bare `unwrap()` calls is in
[`examples/metatool`](../examples/metatool/main.ol). Python cannot promise
this — its `ast` module breaks most releases, because its grammar is an
internal detail that changes. olang's grammar is a commitment, so the
shapes built on it can be too.

## Open artifacts — the transparent binary

`olang build` already embeds a program's complete source (so runtime
errors render a snippet); openness turns that into a guarantee. A built
binary carries its exact source, its `olang.toml` and `olang.lock`, a
sha256 of the source, and its capability manifest — all extractable and
verifiable with `olang inspect`:

```bash
olang inspect ./tool             # a summary: version, checksum, capabilities
olang inspect ./tool --source    # print the exact embedded source
olang inspect ./tool --verify    # recompute the checksum; nonzero on tamper
olang inspect ./tool -o dir/     # extract source + manifest + lockfile
```

You cannot ship an olang program as a black box: every binary can be
opened, diffed against the repo it claims to come from, and audited — and
it doubles as a built-in software bill of materials.

The other half is **capabilities**. A `[capabilities]` block in
`olang.toml` gates the effectful stdlib surface (`fs`, `net`, `db`,
`proc`, environment access) at the module boundary; pure computation is
never gated, and absent, everything is allowed — so it is opt-in and never
breaks existing code. The part no mainstream ecosystem has is
**per-dependency attenuation**: a dependency can be granted *less* than
the app, never more, so a supply-chain compromise that adds `fs`/`net`
behaviour to a package that never had it dies at the gate. The mechanics
are in [the Packages chapter](packages.md#capabilities).

## Open execution — the timeline

Because olang programs are deterministic given their inputs (immutable
values, capture-by-value closures, a seeded RNG) and the only
nondeterminism a program can observe is a small, explicit set of stdlib
calls, a run can be recorded and reproduced exactly:

```bash
olang --record bug.olt program.ol   # run, logging every nondeterministic input
olang replay bug.olt                # re-run: identical, from the trace alone
```

Replay yields the same random rolls, the same timestamps, the same
environment, to the last digit. The `.olt` trace embeds the program
source, so it is portable — replay works from a machine where the program
does not exist — and a *crashed* run records on the way down, so the
failure replays exactly. A bug report becomes a file. Divergence is
detected, not hidden: if the program's effect sequence no longer matches
the trace, replay stops at the exact point and says so. The full model
(and its boundaries) is in [the Tooling
chapter](tooling.md#olang---record-and-olang-replay--the-open-timeline).

## Why olang, and no one else

The three pillars rest on four choices, each load-bearing:

| Choice | What it makes possible |
|---|---|
| **Syntax stabilized early** (a documented commitment) | The AST shapes can be frozen and published — *open code* |
| **Immutable, acyclic values** | A recorded result can never be a live handle in disguise, and a state snapshot is a pointer clone — *open execution*, consistent zero-pause capture |
| **A small, explicit effect boundary** (a handful of stdlib modules) | The one place to record inputs, and the one place to gate capabilities — *open execution* and *open artifacts* |
| **Source-carrying binaries** | Nothing to reverse-engineer — *open artifacts* |

An incumbent cannot follow: Python cannot freeze its AST, Go will not
embed source, and no mainstream runtime is deterministic enough to promise
replay. Openness is not a feature olang added; it is what olang *is* once
those decisions are taken to their conclusion.

The campaign that established these pillars — and the enhancements still
open (value provenance in replay, project-authored checker rules) — is
tracked as [the openness campaign](roadmap.md#the-openness-campaign--what-open-language-means-mechanically).
