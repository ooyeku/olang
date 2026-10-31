// S2's partition bug, caught as a linearizability violation: the clients
// retry an add under a fresh request id (KV_BUGGY, default on), record
// every add and their final read in the history "kv", and check nothing
// themselves; the world checks the history against a key-value model at
// the end of the run. An add the server applied twice answers a total
// one too high, which no order of the adds explains.
//
//   olang sim tests/fixtures/sim/kv_linear.ol --seeds 30 --jobs 4 --shrink

use kv_client { recorded_client }

let buggy = match os.get_env("KV_BUGGY") { Ok(b) => b != "0", Err(e) => true }
let w = sim.world(#{ "seed": 1, "net": #{ "latency": (20, 300) } })
w.node("kv", ["olang", "kv_server.ol", "7500"])
for i in range(1, 5) {
    let name = "c" + to_string(i)
    w.client(name, () => recorded_client(name, buggy, sim.hours(1)), #{ "optional": i > 1 })
}
w.every(sim.minutes(10), () => {
    if random.random() < 0.5 => {
        let victim = "c" + to_string(random.randint(1, 4))
        w.partition(["kv"], [victim])
        time.sleep(random.randint(5000, 30000))
        w.heal()
    } else => ()
})
let run = w.run(#{ "for": sim.hours(2), "props": [sim.linearizable("every add counts once", "kv", sim.kv())] })
println(run.outcome + ": " + to_string(len(run.history("kv"))) + " operations recorded")
