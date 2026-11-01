// log_noisy.ol, with a failure whose message varies from run to run of
// the one bug: the writer names the version the store lost and every
// put of its run that went unanswered, as lists ("lost [287] of \"k3\";
// unanswered: [41, 152, 287]"). A message the network lost is one more
// unanswered put, so every loss the shrinker removes changes the list.
// The shrinker's signature sets the lists aside (it is the error's site
// and its shape), so `--shrink` still takes the run down to the one cut
// that matters.
//
//   olang sim tests/fixtures/sim/log_noisy_list.ol --seed 1 --shrink

fn put(server, key, version) = match http.post(server + "/put",
        unwrap(json.stringify(#{ "key": key, "version": version })), #{ "timeout_ms": 1000 }) {
    Ok(r) => if r.status == 200 => "ok"
        else if r.status == 409 => unwrap(json.parse(r.body)).version
        else => "down",
    Err(e) => "down"
}

fn lost(key, acked, held, unanswered) = assert(false, "lost " + to_string(range(held + 1, acked + 1)) +
    " of " + unwrap(json.stringify(key)) + "; unanswered: " + to_string(unanswered))

fn writer(server, key, until) = {
    let mut acked = 0
    let mut unanswered = []
    while time.monotonic_ms() < until {
        time.sleep(random.randint(50, 400))
        let answer = put(server, key, acked + 1)
        if answer == "ok" => { acked = acked + 1 }
        else if answer != "down" => lost(key, acked, answer, unanswered)
        else => { unanswered = unanswered + [acked + 1] }
    }
    acked
}

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
