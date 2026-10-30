// The log store under crashes: the power goes at random moments (a
// coin every 20 s) and at random effects of the store's own (between an
// append and its sync, between a sync and the answer), and the store
// comes back a second later on what its disk really kept, torn tail and
// all. MODE comes from the file that uses this one.
//
//   log_safe.ol  sync_then_ack: every seed passes
//   log_bug.ol   ack_then_sync: crashes lose acknowledged puts; some
//                seed finds it, and --seed replays it

use log_client { writer }

share fn run_store(mode, minutes) = {
    let w = sim.world(#{ "seed": 1, "net": #{ "latency": (1, 20) } })
    w.node("store", ["olang", "log_store.ol"], #{ "env": #{ "MODE": mode },
        "disk": sim.disk(#{ "torn_writes": true }),
        "crash": #{ "p": 0.0005, "restart_after": sim.seconds(1) } })
    for i in range(1, 4) {
        let key = "k" + to_string(i)
        w.client("w" + to_string(i), () => writer("http://store:7600", key, sim.minutes(minutes)))
    }
    // every 20 s, a coin: heads, the power goes, back a second later
    w.every(sim.seconds(20), () => {
        if random.random() < 0.5 => {
            time.sleep(random.randint(0, 19000))
            let crashed = attempt(() => w.crash("store", #{ "restart_after": sim.seconds(1) }))
        } else => ()
    })
    w.run(sim.minutes(minutes + 5))
}
