# Case study: building robust systems

Part of [the olang book](README.md) ·
[Architecture and internals](internals.md) ·
[Language reference](language.md)

`examples/concurrency/demo/` is Harborline, a harbor-operations simulator and the largest
program in this repository intended to be run rather than only read. Vessels
arrive on a seeded random stream, queue in the roads, receive berths from a
depth- and tide-aware scheduler, unload through a crew of operating-system
threads fed over channels, and settle tariffs computed from expression-tree
pricing rules. Every movement and charge is recorded in a SQLite ledger. Each
simulated day is rendered into a digest whose RSA-signed HMAC chain makes the
history tamper-evident, and each day the simulation state is checked against
the ledger using the `testing` module. A failed invariant terminates the
process.

```bash
cd examples/concurrency/demo && olang main.ol
```

runs it forever (Ctrl-C triggers a graceful shutdown: the partial day is
closed, the invoice book settled, the verdict printed). Bounded and
deterministic:

```bash
olang main.ol --ticks 48 --fast --quiet --seed 7
```

Two simulated days, no pacing, day digests only — the same seed always
produces the same voyage, the same revenue, the same chain head. `olang
test .` runs every module's self-checks; a run writes **no files**
unless you pass `--out DIR`, in which case digests and revenue charts
land there.

This chapter reads the demo as a design study: the patterns that make an
olang system robust, each pointing at the module that demonstrates it.

Part of [the olang book](README.md) · [Tour](tour.md) ·
[Language Reference](language.md) · [Pitfalls](pitfalls.md)

## The map

```text
examples/concurrency/demo/
  main.ol           the world record, the tick loop, CLI, config,
                    graceful shutdown, daily invariants
  config.toml       the simulation's dials (validated at boot)
  lib/
    prelude.ol      pure toolkit: Option, sorting strategies, search,
                    money/percent formatting, the cache-threading idiom
    cargo.ol        the domain ADTs: Cargo sum type, Tariff expression
                    trees, one evaluator + one printer over one recursion
    vessels.ol      generation from the seeded stream, regex-validated
                    callsigns, a trait with a default method
    schedule.ol     BST priority queue, berth allocation, the tide
                    model, calendar arithmetic
    ledger.ol       SQLite: bound parameters everywhere, transactions
                    around each invoice, the aggregate queries
    metrics.ol      fold-based group-by, leaderboards, stddev
                    cross-checked against the stats module
    manifest.ol     regex log-line parsing, a template engine,
                    JSON/CSV round-trips — every format a parse/render pair
    report.ol       the daily digest and its revenue chart
    signing.ol      sha256 manifests, the hmac digest chain, RSA
                    signatures, bcrypt for the console gate
    workers.ol      crews as threads over channels; par_map pricing
    console.ol      the term dashboard — pure string builders, tty-gated
```

Dependencies flow one way: `main` imports the services; services import
`prelude` and `cargo`; nothing imports `main`. The deepest chain —
`report → metrics → prelude`, with `metrics` re-sharing prelude helpers
via `share use` — is deliberate: it exercises the module system the way
layered applications do.

## Pattern 1: one world, threaded through

There is no global mutable state anywhere in the demo. The entire
simulation lives in one record, and the tick function maps world to
world:

```olang no-run
fn tick(w) = {
    let w1 = arrive(w)          // maybe a new vessel in the roads
    let w2 = try_berth(w1)      // best-priority vessel that fits
    let w3 = depart_due(w2)     // settle invoices, free berths
    if w3.tick % 24 == 23 => close_day(w3) else => w3
}
```

This is not a stylistic preference — it is what olang's semantics
reward. Closures capture **by value** ([Pitfalls](pitfalls.md)), so a
captured "world" would be a dead snapshot; `olang check` flags exactly
that mistake. Passing state in and returning new state out is the idiom
that cannot go wrong, and it makes every transition testable in
isolation: `occupy(berths, 3, "DEEP-100")` returns a new berth list, and
the old one is still there to compare against.

The same idiom scales down to memoization. `lib/schedule.ol`'s tide
model rides a Fibonacci-weighted swell whose cache is *threaded*, never
mutated:

```olang no-run
share fn tide_at(tick: Int, cache) = {
    let springs = memo_fib(15 + (tick / 144) % 5, cache)
    [swell_from(springs[0]), springs[1]]   // value AND the grown cache
}
```

The caller owns the cache and passes back what it receives — the
functional replacement for in-place memoization, pinned by a test that
the cache actually grows.

## Pattern 2: make illegal states unrepresentable

`lib/cargo.ol` applies algebraic data types. Cargo is a sum type —
`Container(teu)`, `Bulk(tonnes)`, `Reefer(teu)`,
`Hazmat(class, tonnes)` — so a cargo item that is somehow both bulk and
containerized cannot exist, and `match` handles every kind or the
checker complains. Policy lives in guards:

```olang no-run
share fn needs_sealed_berth(c) = match c {
    Hazmat(class, t) if class > 5 => true,
    _ => false
}
```

Tariffs go further: pricing rules are a *recursive expression tree*
(`Flat`, `PerUnit`, `Surcharge`, `Sum`, `AtLeast`), built as data and
run by a ten-line structural interpreter. New pricing policy is a new
tree, not new code — and the pretty-printer is the same recursion with a
second interpretation, so what you can evaluate you can always display.

## Pattern 3: Result discipline at every edge

Everything that can fail returns a `Result`, and the demo consumes them
deliberately: `unwrap` where failure is a bug (a schema statement),
`match` where failure is input (a manifest line), and structured errors
where the caller must react. Boot is the showcase — `config.toml` is
validated field by field, and nonsense refuses to start:

```olang no-run
error BootError {
    BadConfig: { msg: String }
}

fn load_config(path) = {
    let text = match fs.read_file(path) {
        Ok(t) => t,
        Err(e) => return Err(BadConfig("cannot read " + path + ": " + e))
    }
    let parsed = match toml.parse(text) {
        Ok(p) => p,
        Err(e) => return Err(BadConfig("bad toml: " + e))
    }
    let berths = map_get(map_get(parsed, "harbor"), "berths")
    if berths < 2 => return Err(BadConfig("need at least 2 berths"))
    Ok({ berths: berths })
}
```

`error` declarations ([Language Reference](language.md)) give the
failure a name and a payload; the caller matches `Err(BadConfig(msg))`
and exits with a message instead of a stack trace.

## Pattern 4: concurrency you can test

`lib/workers.ol` runs the unload crews as real OS threads fed over
channels — and is written so the *answers* are schedule-independent:
jobs carry indices, results are re-sorted by index, and totals are
order-independent sums. The self-check is the point:

```olang no-run
test "crews unload deterministically" {
    let a = unload_all(hold, 3)   // three crews
    let b = unload_all(hold, 1)   // one crew
    testing.assert_eq(a, b)       // same answers, any schedule
}
```

The batch tariff sweep uses `par_map`, with a test pinning its result equal
to the sequential `map`. Because `par_map` carries `spawn`'s snapshot
semantics, code that produces the same result under `par_map` as under `map`
is parallel-safe, and the test verifies this.

## Pattern 5: the system audits itself

The demo checks its own behavior at runtime in three ways, so that a defect
is detected rather than producing silently incorrect output:

- **Module self-tests.** Every `lib/*.ol` carries `test` blocks — 95
  assertions across the suite — runnable with `olang test .` and pinned
  in CI. They test round-trips (parse ∘ render = identity), agreements
  (fold-stddev vs the `stats` module), and policies (the sealed-berth
  guard).
- **Daily invariants.** Every simulated day, `check_invariants` compares
  the world against the ledger: departures counted by SQLite must equal
  vessels the world served, ledger revenue must equal the world's
  running total to the cent, every berth occupant must be a known vessel
  with a scheduled departure. `testing.assert_*` tallies feed the final
  verdict, and a failed invariant makes the process exit nonzero.
- **The tamper-evident record.** Each day's digest is hmac-chained to
  the previous day's link and the chain head is RSA-signed
  (`lib/signing.ol`) — rewrite one day's history and every later link
  breaks. The signing module's tests *prove* the chain detects a cooked
  ledger.

Long soaks are the reason the demo exists: it has already flushed out
real interpreter and tooling bugs that no small example ever hit. Run
it for hours; that is what it is for.

## Pattern 6: honest floats, stable tests

Two lessons the demo's own test suite learned the hard way, kept in the
code as worked examples: compare floats within an epsilon (`1.1 * 100.0`
is `110.00000000000001` in IEEE arithmetic, and a robust suite never
pretends otherwise), and when asserting on *formatted* floats, build the
expectation with the same interpolation the code uses, so the assertion
is stable under any display convention.

## Determinism as a feature

`--seed N` makes the whole voyage reproducible — arrivals, holds,
drafts, revenue, the chain head. That is what turns "it crashed
overnight" into a bug report: the seed and the tick number are the
reproduction. The generator draws everything from the stdlib `random`
stream, seeded once at boot; the worker threads are the only
nondeterminism, and Pattern 4 confines them.

## Where to go from here

Read `main.ol` top to bottom — it is under 350 lines and every pattern
above appears in it. Then pick the module closest to what you are
building: talking to SQLite, start at `lib/ledger.ol`; parsing text
formats, `lib/manifest.ol`; pricing/rules engines, `lib/cargo.ol`;
worker pools, `lib/workers.ol`. The [Tour](tour.md) teaches the language
constructs; this program shows them holding up under load.
