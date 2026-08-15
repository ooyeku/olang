// ═══════════════════════════════════════════════════════════════════
// HARBORLINE — a long-running harbor-operations system, in olang.
//
// Vessels arrive on a seeded random stream, queue in the roads, get
// berths from a depth-and-tide-aware scheduler, unload through a crew
// of real threads fed over channels, and settle tariffs computed from
// expression-tree pricing rules — every movement and charge recorded
// in SQLite, every day folded into a template-rendered digest whose
// hmac chain (RSA-signed) makes the history tamper-evident.
//
//   olang main.ol                     run forever (Ctrl-C for graceful shutdown)
//   olang main.ol --ticks 48 --fast   two bounded days, no pacing
//   olang main.ol --seed 7 --quiet    deterministic and terse
//   olang main.ol --out reports      write digests + charts (default: no files)
//   olang test .                      every module's self-checks
//
// The architecture is the demo: state lives in one world record that
// tick() maps to the next world — no hidden mutation, which is exactly
// what capture-by-value closures reward. Invariants are asserted every
// simulated day with the testing module; a failed invariant fails the
// process. Resources are bounded by design: vessels that cannot be
// served divert after 72 hours, and ledger rows older than seven days
// are pruned once their day is sealed into the signed digest chain —
// so an unbounded run holds a bounded world. This program is meant to
// RUN — hours, not seconds — as a soak test for the language itself.
// ═══════════════════════════════════════════════════════════════════

use cli
use term
use lib.prelude { clamp, money }
use lib.cargo { needs_sealed_berth }
use lib.vessels { arrival, hail_vessel }
use lib.schedule { make_berths, find_berth, occupy, release, utilization,
    Queue, Empty, enqueue, queue_order, queue_size, vessel_priority,
    tide_at, day_of, date_of }
use lib.ledger { open_ledger, close_ledger, record_movement, settle_charges,
    revenue_by_kind, ledger_totals, charges_between, prune_before }
use lib.workers { unload_all, invoice_lines, invoice_total }
use lib.report { build_digest, write_digest }
use lib.signing { chain_start, chain_link, make_keys, sign_head, verify_head,
    manifest_checksum }
use lib.manifest { log_line, events_by_kind }
use lib.console { tick_line, day_table, util_bar, announce, banner }

// A vessel that cannot be served within this many hours gives up and
// diverts to another port — the pressure valve that keeps the queue,
// the vessel map, and the run's memory bounded under any load.
let PATIENCE_HOURS = 72
// Ledger rows older than this are pruned at day close; the signed
// digest chain is the archive.
let LEDGER_WINDOW_TICKS = 7 * 24

// ── Boot: configuration, validated or refused ───────────────────────
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
    let harbor = map_get(parsed, "harbor")
    let rates = map_get(parsed, "rates")
    let berths = map_get(harbor, "berths")
    if berths < 2 => return Err(BadConfig("need at least 2 berths (one is sealed)"))
    if map_get(harbor, "arrival_chance") <= 0.0 => return Err(BadConfig("arrival_chance must be positive"))
    Ok({
        berths: berths,
        crews: map_get(harbor, "crews"),
        arrival_chance: map_get(harbor, "arrival_chance"),
        tick_ms: map_get(harbor, "tick_ms"),
        rates: rates
    })
}

let spec = #{
    "name": "harborline", "about": "a long-running harbor simulation",
    "flags": [
        #{ "name": "ticks", "short": "t", "type": "int", "default": 0,
           "help": "stop after N ticks (0 = run until interrupted)" },
        #{ "name": "seed", "short": "s", "type": "int", "default": 42,
           "help": "random seed (same seed, same voyage)" },
        #{ "name": "fast", "short": "f", "type": "bool", "help": "no pacing sleep" },
        #{ "name": "quiet", "short": "q", "type": "bool", "help": "day digests only" },
        #{ "name": "db", "type": "string", "default": ":memory:",
           "help": "ledger path (:memory: or a file)" },
        #{ "name": "out", "type": "string", "default": "",
           "help": "write digests and charts under this directory (default: no files)" }
    ],
    "args": []
}

let args = match cli.parse(spec, cli.args()) {
    Ok(a) => a,
    Err(e) => {
        println(term.red("harborline: ") + e)
        os.exit(2)
    }
}
if map_get(args, "help") => {
    println(cli.help(spec))
    os.exit(0)
}

let config = match load_config("config.toml") {
    Ok(c) => c,
    Err(BadConfig(msg)) => {
        println(term.red("config: ") + msg)
        os.exit(2)
    }
}

let max_ticks = map_get(args, "ticks")
let seed = map_get(args, "seed")
let fast = map_get(args, "fast")
let quiet = map_get(args, "quiet")
let out_dir = map_get(args, "out")

random.seed(seed)
unwrap(os.on_interrupt())

// The signing identity for this run: digests chain from genesis, and
// the chain head is RSA-signed at every day close.
let keys = make_keys()

println(banner(config.berths, seed, max_ticks))

// ── The world: one record, threaded through every tick ──────────────
// vessels: callsign -> vessel      (everyone currently known to the port)
// departures: callsign -> tick     (when each berthed vessel will leave)
// archived: totals pruned from the live ledger (the chain holds the rest)
fn new_world(conn) = {
    tick: 0,
    berths: make_berths(config.berths),
    queue: Empty,
    vessels: #{},
    departures: #{},
    tide_cache: #{},
    conn: conn,
    served: 0,
    diverted: 0,
    revenue: 0.0,
    log: [],
    chain: chain_start(),
    day_start: 0,
    archived: #{ "departures": 0, "revenue": 0.0 }
}

// Surgical world updates: map_set works on struct-likes, so a patch is
// a fold — no fifteen-field record literal at every transition.
fn patch(w, changes) = entries(changes) |> fold(w, (acc, e) => map_set(acc, e[0], e[1]))

fn say(text) = { if !quiet => println(text) }

// One simulated hour. Pure state -> state, plus its I/O at the edges
// (ledger writes, console lines).
fn tick(w) = {
    let tide_r = tide_at(w.tick, w.tide_cache)
    let w0 = patch(w, #{ "tide_cache": tide_r[1] })
    let w1 = arrive(w0)
    let w2 = try_berth(w1, queue_order(w1.queue), tide_r[0])
    let w3 = depart_due(w2)
    let w4 = divert_overdue(w3)
    say(tick_line(w4.tick, w4.berths, queue_size(w4.queue), w4.revenue))
    let w5 = if w4.tick % 24 == 23 => close_day(w4) else => w4
    patch(w5, #{ "tick": w5.tick + 1 })
}

// ── arrivals ────────────────────────────────────────────────────────
fn arrive(w) = {
    if random.random() >= config.arrival_chance => w
    else => {
        let v = arrival(w.tick)
        record_movement(w.conn, w.tick, v, "arrived", -1)
        say(announce("arrive", hail_vessel(v) + " in the roads"))
        patch(w, #{
            "queue": enqueue(w.queue, vessel_priority(v), v.callsign),
            "vessels": map_set(w.vessels, v.callsign, v),
            "log": concat(w.log, [log_line(w.tick, "arrived", v.callsign, -1, v.name)])
        })
    }
}

// ── berthing: walk the waiting list best-first, one berth per tick ──
fn try_berth(w, waiting, tide) = {
    if len(waiting) == 0 => w
    else => {
        let callsign = head(waiting)[1]
        let v = map_get(w.vessels, callsign)
        let sealed = len(v.hold |> filter(needs_sealed_berth)) > 0
        let berth = find_berth(w.berths, v, sealed, tide)
        if berth < 0 => try_berth(w, tail(waiting), tide)
        else => {
            // crews unload now; total crew-minutes set the departure
            let minutes = unload_all(v.hold, config.crews)
            let hours = clamp((minutes |> fold(0, (a, m) => a + m)) / 60, 1, 96)
            record_movement(w.conn, w.tick, v, "berthed", berth)
            say(announce("berth", `${v.callsign} berth ${berth}, ${hours}h to unload`))
            patch(w, #{
                "berths": occupy(w.berths, berth, callsign),
                "queue": requeue_without(w.queue, callsign),
                "departures": map_set(w.departures, callsign, w.tick + hours),
                "log": concat(w.log, [log_line(w.tick, "berthed", callsign, berth, v.name)])
            })
        }
    }
}

// Rebuild the queue without one callsign (BSTs are immutable here).
fn requeue_without(q, callsign) =
    queue_order(q) |> filter((e) => e[1] != callsign)
        |> fold(Empty, (acc, e) => enqueue(acc, e[0], e[1]))

// ── departures: settle the invoice atomically, free the berth ───────
fn depart_due(w) = {
    let due = map_keys(w.departures)
        |> filter((cs) => map_get(w.departures, cs) <= w.tick)
    due |> fold(w, (acc, cs) => {
        let v = map_get(acc.vessels, cs)
        let lines = invoice_lines(v.hold, config.rates)
        let total = invoice_total(lines)
        settle_charges(acc.conn, acc.tick, cs, lines)
        record_movement(acc.conn, acc.tick, v, "departed", -1)
        say(announce("depart", `${cs} sails, invoice ${money(total)}`))
        patch(acc, #{
            "berths": release(acc.berths, cs),
            "vessels": map_remove(acc.vessels, cs),
            "departures": map_remove(acc.departures, cs),
            "served": acc.served + 1,
            "revenue": acc.revenue + total,
            "log": concat(acc.log, [log_line(acc.tick, "departed", cs, -1, v.name)])
        })
    })
}

// ── diversions: nobody waits forever ────────────────────────────────
// A vessel still in the roads after PATIENCE_HOURS gives up. Without
// this valve an over-subscribed harbor accumulates queued vessels
// without bound — with it, the world stays bounded under any arrival
// rate.
fn divert_overdue(w) = {
    let overdue = queue_order(w.queue)
        |> map((e) => e[1])
        |> filter((cs) => w.tick - map_get(w.vessels, cs).eta_tick > PATIENCE_HOURS)
    overdue |> fold(w, (acc, cs) => {
        let v = map_get(acc.vessels, cs)
        record_movement(acc.conn, acc.tick, v, "diverted", -1)
        say(announce("alert", `${cs} diverts after ${PATIENCE_HOURS}h at anchor`))
        patch(acc, #{
            "queue": requeue_without(acc.queue, cs),
            "vessels": map_remove(acc.vessels, cs),
            "diverted": acc.diverted + 1,
            "log": concat(acc.log, [log_line(acc.tick, "diverted", cs, -1, v.name)])
        })
    })
}

// ── day close: digest, chain, signature, invariants, archive ────────
fn close_day(w) = {
    let rows = charges_between(w.conn, w.day_start, w.tick + 1)
    let digest = build_digest(w.tick, rows, w.served, queue_size(w.queue))
    let link = chain_link(w.chain, digest.text)
    let sig = sign_head(link, keys.private_key)

    println("")
    println(digest.text)
    println(`  utilization ${util_bar(utilization(w.berths))}`)
    println(day_table(revenue_by_kind(w.conn)))
    // Files are strictly opt-in: a soak run leaves no trace unless the
    // operator asked for the paper trail.
    if out_dir != "" => {
        println(`  digest -> ${write_digest(out_dir, w.tick, digest)}`)
    }
    println(`  chain ${str.substring(link, 0, 16)}…  signed: ${verify_head(link, sig, keys.public_key)}`)
    println(`  traffic ${show(events_by_kind(w.log))}`)
    println(`  manifest sha256 ${str.substring(manifest_checksum(w.log), 0, 16)}…`)
    println("")

    check_invariants(w)

    // Archive: the day is sealed into the chain; prune the live window
    // so an infinite run holds a bounded ledger.
    let w2 = if w.tick + 1 > LEDGER_WINDOW_TICKS => {
        let dropped = prune_before(w.conn, w.tick + 1 - LEDGER_WINDOW_TICKS)
        patch(w, #{ "archived": #{
            "departures": map_get(w.archived, "departures") + map_get(dropped, "departures"),
            "revenue": map_get(w.archived, "revenue") + map_get(dropped, "revenue")
        } })
    } else => w
    patch(w2, #{ "log": [], "chain": link, "day_start": w2.tick + 1 })
}

// The soak test's teeth: every day, the world must agree with the
// ledger — live rows plus the archived totals. A failure names itself
// and fails the run.
fn expect(label, cond: Bool) = {
    testing.assert_true(cond)
    if !cond => println(announce("alert", "INVARIANT: " + label))
    cond
}

fn check_invariants(w) = {
    let totals = ledger_totals(w.conn)
    let departures = map_get(totals, "departures") + map_get(w.archived, "departures")
    expect(`departures ledger=${departures} world=${w.served}`, departures == w.served)
    let ledger_rev = map_get(totals, "revenue") + map_get(w.archived, "revenue")
    expect(`revenue ledger=${ledger_rev} world=${w.revenue}`,
        math.abs(ledger_rev - w.revenue) < 0.01)
    let u = utilization(w.berths)
    expect("utilization in [0,1]", u >= 0.0 && u <= 1.0)
    // every occupant is a known vessel with a departure scheduled
    for b in w.berths {
        if b.occupant != "" => {
            expect(`occupant ${b.occupant} known`, map_has_key(w.vessels, b.occupant))
            expect(`occupant ${b.occupant} scheduled`, map_has_key(w.departures, b.occupant))
        }
    }
    // the world is bounded: every known vessel is either queued or
    // berthed, so diversions really do reclaim their entries
    expect("no orphan vessels",
        len(map_keys(w.vessels)) == queue_size(w.queue) + len(map_keys(w.departures)))
}

// ── The run loop ────────────────────────────────────────────────────
let conn = open_ledger(map_get(args, "db"))
let mut world = new_world(conn)

loop {
    if max_ticks > 0 && world.tick >= max_ticks => break
    if unwrap(os.interrupted()) => {
        println(announce("alert", "interrupt — closing the day and settling up"))
        break
    }
    world = tick(world)
    if !fast => time.sleep(config.tick_ms)
}

// ── Graceful shutdown: final digest, summary, verdict ───────────────
// Only close the partial day if it has any ticks; a run that stopped on
// a day boundary already closed it.
let final_world = if world.tick > world.day_start => close_day(world) else => world
let totals = ledger_totals(conn)
println(term.bold("HARBORLINE SHUTDOWN"))
println(`  ticks: ${world.tick}   days: ${day_of(world.tick)}   date: ${date_of(world.tick)}`)
println(`  vessels served: ${final_world.served}   diverted: ${final_world.diverted}   revenue: ${money(final_world.revenue)}`)
println(`  live ledger rows: ${map_get(totals, "movements")} movements, ${map_get(totals, "charge_rows")} charges (older days archived into the chain)`)
close_ledger(conn)

let verdict = testing.test_summary()
println(`  invariants: ${map_get(verdict, "passed")} passed, ${map_get(verdict, "failed")} failed`)
if map_get(verdict, "failed") > 0 => {
    println(term.red("INVARIANT FAILURES — the soak found something"))
    os.exit(1)
}
println(term.green("all invariants held"))
