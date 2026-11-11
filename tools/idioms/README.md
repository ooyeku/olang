# Idioms rules and ratchet

The parts of [docs/idioms.md](../../docs/idioms.md) a machine can see, as
`olang check` rules, plus a ratchet that keeps existing code from getting
worse while it is rewritten. Draft, not committed.

## The rules (`rules.ol`)

| Rule | Finds | Write instead |
|---|---|---|
| `rule_map_set_chain` | `map_set(map_set(r, "a", x), "b", y)` with literal name keys (the rebind `m = map_set(map_set(m, …), …)`, updated in place, is left) | one spread of the same kind: `#{ ...m, "a": x, "b": y }` or `{ ...r, a: x, b: y }` |
| `rule_record_as_map` | `#{ "kind": "x" }`, where every key is a literal name (a spread's own entries, and a map whose keys are all capitals such as an `env`, are left alone) | a record `{ kind: "x" }`; a map that a hot loop grows with `map_set` stays a map |
| `rule_kind_chain` | three or more `if x.f == "…"` tests of one field in an `else if` chain | a `match` |
| `rule_swallowed_err` | `match r { Ok(v) => v, Err(e) => <literal> }` | `unwrap_or(r, …)`, or `?` |
| `rule_long_fn` | a function over 80 lines | named steps |

A line that is a deliberate exception says `// allow: rule_<name>`.

Run them by hand:

```bash
olang check --rules ~/Code/olang-world/olang/tools/idioms/rules.ol path/
```

## The ratchet (`ratchet.ol`)

```bash
olang tools/idioms/ratchet.ol check  ~/Code/olang-world/apps/studio
olang tools/idioms/ratchet.ol update ~/Code/olang-world/apps/studio
```

`check` fails (exit 1) when a file has more findings of a rule than
`baselines/<project>.tsv` allows, and treats files not in the baseline as
zero. `update` rewrites the baseline from the project as it stands. Run it
after a cleanup so the lower counts hold. Don't run it to accept a
regression. Fixtures, `.build/`, `target/`, `dist/` and shelf pins are
skipped. `OLANG` picks the olang binary.

Studio, Loom and work-scheduler run `check` from `tests/idioms_test.ol`, so `olang test` holds it.

Once a project reaches zero for a rule, that project can list `rules.ol`
under `[check] rules` in its `olang.toml`. From then on a finding fails
`olang check` itself, and the ratchet is no longer needed for that rule.
