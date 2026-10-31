// A failing world buried in faults that have nothing to do with the
// failure, for the shrinker. The log store acknowledges before it syncs
// (log_bug.ol's bug) and loses a put to the one power cut that matters,
// at 70 s. Around it: a spare node that crashes at random while it
// scribbles on its own disk, slows its disk every 5 s and is cut off
// every 7 s, buggify points on it, and links to the store that lose 1%
// of messages (the writers retry). `--shrink` takes it down to the cut.
//
//   olang sim tests/fixtures/sim/log_noisy.ol --seed 1 --shrink

use log_client { writer }

fn scribble(until) = {
    if !fs.exists("/data") => unwrap(fs.create_dir("/data")) else => ()
    let mut n = 0
    while time.monotonic_ms() < until {
        n = n + 1
        let wrote = fs.append_file("/data/scratch", to_string(n) + "\n")
        if sim.maybe("spare_sync") => { let synced = fs.sync("/data/scratch") } else => ()
        time.sleep(random.randint(100, 400))
    }
    n
}

let w = sim.world(#{ "seed": 1, "net": #{ "latency": (1, 20) } })
w.node("store", ["olang", "log_store.ol"], #{ "env": #{ "MODE": "ack_then_sync" },
    "disk": sim.disk(#{ "torn_writes": true }) })
w.client("spare", () => scribble(sim.seconds(100)),
    #{ "crash": #{ "p": 0.01, "restart_after": 300 }, "optional": true })
for i in range(1, 4) {
    let key = "k" + to_string(i)
    let name = "w" + to_string(i)
    w.client(name, () => writer("http://store:7600", key, sim.seconds(100)))
    w.link(name, "store", #{ "latency": (1, 20), "loss": 0.01 })
}
w.every(sim.seconds(5), () => w.slow_disk("spare", 1 + random.randint(0, 4)))
w.every(sim.seconds(7), () => {
    w.partition(["spare"], ["w1", "w2", "w3"])
    time.sleep(1000)
    w.heal()
})
w.at(sim.seconds(70), () => w.crash("store", #{ "restart_after": sim.seconds(1) }))
let run = w.run(sim.minutes(3))
println(run.outcome + ": " + to_string(len(run.faults)) + " faults")
