// The SQLite store under crashes. Its disk tears unsynced writes; with
// `lies` it also lies about fsync that share of the time. The power goes
// every 20 s on a coin's toss, and (with `points`) at random effects and
// inside SQLite's own writes and syncs: in the middle of a commit.
//
//   sqlite_honest.ol  every seed passes: SQLite keeps what it committed
//   sqlite_lies.ol    a lying fsync loses acknowledged puts

use log_client { writer }

share fn run_sqlite(lies, points, minutes) = {
    let w = sim.world(#{ "seed": 1, "net": #{ "latency": (1, 20) } })
    let crash = if points => #{ "p": 0.0002, "restart_after": sim.seconds(1) } else => #{ "p": 0 }
    w.node("store", ["olang", "sqlite_store.ol"], #{
        "disk": sim.disk(#{ "torn_writes": true, "fsync_lies": lies }),
        "crash": crash })
    for i in range(1, 4) {
        let key = "k" + to_string(i)
        w.client("w" + to_string(i), () => writer("http://store:7600", key, sim.minutes(minutes)))
    }
    w.every(sim.seconds(20), () => {
        if random.random() < 0.5 => {
            let crashed = attempt(() => w.crash("store", #{ "restart_after": sim.seconds(1) }))
        } else => ()
    })
    w.run(sim.minutes(minutes + 5))
}
