// workers — the unload crews: real OS threads fed over channels, and
// par_map for the batch tariff sweep. Concurrency here is deliberately
// deterministic-by-construction: crews return per-job results tagged
// with the job index, and totals are order-independent sums, so a run
// with --seed N prints the same numbers whatever the thread schedule.

use cargo { unload_effort, price_cargo, kind_of, tariff_units }

// ── The crew pool ───────────────────────────────────────────────────
// Jobs go down a bounded channel; each crew computes the unload effort
// of its cargo item (a real numeric loop — this is the soak load) and
// sends back [job_index, crew_minutes]. `done` closes the pool.
share fn unload_all(items, crews: Int) = {
    let jobs = chan.bounded(len(items) + crews)
    let results = chan.new()

    // enqueue every job, then one poison pill per crew
    let mut i = 0
    for item in items {
        unwrap(chan.send(jobs, [i, unload_effort(item)]))
        i = i + 1
    }
    for c in range(0, crews) {
        unwrap(chan.send(jobs, [-1, 0]))
    }

    // crews: real threads pulling from the shared jobs channel. `spawn
    // crew(...)` snapshots the arguments; channel handles cross the
    // boundary and stay shared.
    let mut tasks = []
    for c in range(0, crews) {
        let t = spawn crew(jobs, results)
        tasks = concat(tasks, [t])
    }

    // collect exactly one result per job, in arrival order, then reorder
    // by job index so the outcome is schedule-independent.
    let mut collected = []
    for k in range(0, len(items)) {
        collected = concat(collected, [unwrap(chan.recv(results))])
    }
    for t in tasks { await t }
    chan.close(jobs)
    chan.close(results)
    let by_index = sort_by_key(collected)
    by_index |> map((r) => r[1])
}

// One crew: pull a job, grind it, report; a negative index is the
// poison pill. Returns how many jobs this crew handled.
fn crew(jobs, results) = {
    let mut handled = 0
    let mut going = true
    while going {
        let job = unwrap(chan.recv(jobs))
        if job[0] < 0 => { going = false }
        else => {
            unwrap(chan.send(results, [job[0], grind(job[1])]))
            handled = handled + 1
        }
    }
    handled
}

// The crane-work model: effort units ground through a numeric loop.
// Deliberately hot — this is what the bytecode tier and JIT chew on
// during a long soak.
fn grind(effort: Int) = {
    let mut acc = 0.0
    let mut k = 0
    while k < effort {
        acc = acc + math.sqrt(to_float(k) + 1.0)
        k = k + 1
    }
    // crew-minutes: effort plus a fatigue term that grows with the job
    effort + to_int(acc / 100.0)
}

// Order results by job index (insertion sort over pairs).
fn sort_by_key(pairs) = {
    fn insert(sorted, p) = {
        if len(sorted) == 0 => [p]
        else => if p[0] < head(sorted)[0] => concat([p], sorted)
                else => concat([head(sorted)], insert(tail(sorted), p))
    }
    pairs |> fold([], (acc, p) => insert(acc, p))
}

// ── The batch tariff sweep ──────────────────────────────────────────
// Pricing a manifest is pure per item — the textbook par_map. The
// result must equal the sequential map exactly; the self-check pins it.
share fn invoice_lines(hold, rates) =
    par_map(hold, (c) => {
        kind: kind_of(c),
        units: tariff_units(c),
        amount: price_cargo(c, rates)
    })

share fn invoice_total(lines) = lines |> fold(0.0, (a, l) => a + l.amount)

// ── Self-checks ─────────────────────────────────────────────────────
fn test_rates() = #{
    "container": 2.5, "reefer_power": 1.25, "bulk": 1.1, "hazmat_step": 0.4
}

test "crews unload deterministically" {
    use cargo { Container, Bulk, Hazmat }
    let hold = [Container(40), Bulk(200.0), Hazmat(3, 60.0), Container(12)]
    let a = unload_all(hold, 3)
    let b = unload_all(hold, 1)
    // same answers from 3 crews and 1 crew: schedule-independence
    testing.assert_eq(a, b)
    testing.assert_eq(len(a), 4)
    // each job's crew-minutes at least its raw effort
    testing.assert_true(a[0] >= 120)     // Container(40) -> effort 120
}

test "par_map pricing equals sequential pricing" {
    use cargo { Container, Reefer, Bulk }
    let hold = [Container(40), Reefer(20), Bulk(150.0)]
    let par = invoice_lines(hold, test_rates())
    let seq = hold |> map((c) => {
        kind: kind_of(c),
        units: tariff_units(c),
        amount: price_cargo(c, test_rates())
    })
    testing.assert_eq(invoice_total(par), invoice_total(seq))
    testing.assert_eq(len(par), 3)
    testing.assert_eq(par[0].kind, "container")
    testing.assert_eq(invoice_total(par), 100.0 + 105.0 + 165.0)
}
