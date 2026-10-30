// A key-value server and four clients for a simulated hour, over a
// network that loses 2% of messages, duplicates 1%, and is partitioned
// now and then. The clients retry with the same request id, so every
// add counts once whatever the network does: every seed passes.
//
//   olang sim tests/fixtures/sim/kv_world.ol --seed 1
//   olang sim tests/fixtures/sim/kv_world.ol --seeds 20

use kv_client { client }

let w = sim.world(#{ "seed": 1, "net": #{ "latency": (2, 40), "loss": 0.02, "dup": 0.01 } })
w.node("kv", ["olang", "kv_server.ol", "7500"])
for i in range(1, 5) {
    let name = "c" + to_string(i)
    w.client(name, () => client(name, false, sim.hours(1)))
}
// Every five minutes, a coin: heads, one client is cut off for 5–30 s.
w.every(sim.minutes(5), () => {
    if random.random() < 0.5 => {
        let victim = "c" + to_string(random.randint(1, 4))
        w.partition(["kv"], [victim])
        time.sleep(random.randint(5000, 30000))
        w.heal()
    } else => ()
})
let run = w.run(sim.hours(2))
let timeouts = len(run.history |> filter((h) => h.outcome == "timeout"))
println(run.outcome + ": " + to_string(len(run.history)) + " requests, " + to_string(timeouts) + " timed out and were retried; net " + to_string(run.net))
