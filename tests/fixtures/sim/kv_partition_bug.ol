// The planted bug: clients that retry an add under a fresh request id.
// The network loses nothing; the only faults are partitions. Most seeds
// pass: a partition usually cuts a request before it arrives, and the
// retry is then right. Some seed cuts an answer after the server applied
// the add: the client retries, the add counts twice, and the client's
// final check fails.
//
//   olang sim tests/fixtures/sim/kv_partition_bug.ol --seeds 30   (finds a seed)
//   olang sim tests/fixtures/sim/kv_partition_bug.ol --seed S     (replays it)

use kv_client { client }

// A WAN: 20–300 ms a leg, nothing lost.
let w = sim.world(#{ "seed": 1, "net": #{ "latency": (20, 300) } })
w.node("kv", ["olang", "kv_server.ol", "7500"])
for i in range(1, 5) {
    let name = "c" + to_string(i)
    w.client(name, () => client(name, true, sim.hours(1)))
}
// Every ten minutes, a coin: heads, one client is cut off for 5–30 s.
w.every(sim.minutes(10), () => {
    if random.random() < 0.5 => {
        let victim = "c" + to_string(random.randint(1, 4))
        w.partition(["kv"], [victim])
        time.sleep(random.randint(5000, 30000))
        w.heal()
    } else => ()
})
let run = w.run(sim.hours(2))
let timeouts = len(run.history |> filter((h) => h.outcome == "timeout"))
println(run.outcome + ": " + to_string(len(run.history)) + " requests, " + to_string(timeouts) + " timed out and were retried")
