// olang-track's headless server (api.ol, lib/server.ol's run_server) as
// a node, unchanged, with agents that file issues over its rpc and read
// each acknowledged one back after the server crashed.
//
//   OLANG_TRACK_DIR=<olang-track checkout> olang sim track_world.ol --seeds N
//
// The server runs on its own disk: the sources it reads at boot (lib/,
// static/, packages/, the entries) are copied onto it here, outside the
// world, and its database lives at /data/track.db. Nothing is written to
// the checkout or its database.
//
// CRASH (env): "kill" (SIGKILL or SIGTERM: memory gone, every write
// kept: the default), "power" (the power goes: what was not synced is
// lost), "none". MINUTES (env): how long the agents write (5). EVERY
// (env): seconds between chances of a crash (30).

let TRACK = match os.get_env("OLANG_TRACK_DIR") { Ok(d) => d, Err(e) => "../../../../libraries/olang-track" }
let CRASH = match os.get_env("CRASH") { Ok(c) => c, Err(e) => "kill" }
let MINUTES = match os.get_env("MINUTES") { Ok(m) => unwrap(str.parse_int(m)), Err(e) => 5 }

/// The shelf libraries the server reads as files at boot (the web SDK's
/// stylesheet and browser modules, Shuttle's runtime for the build
/// stamp), from this machine's shelf, to /shelf/<name> on the node's
/// disk, and a shelf.toml there naming them.
fn shelf_files() = {
    let home = unwrap(os.get_env("HOME"))
    let shelf = unwrap(toml.parse(unwrap(fs.read_file(home + "/.olang/shelf.toml"))))
    let mut files = #{}
    let mut lines = ["version = 1", "", "[libraries]"]
    for name in ["web", "shuttle", "markdown", "validate"] {
        let dir = map_get(shelf.libraries, name)
        lines = lines + [name + " = \"/shelf/" + name + "\""]
        for f in unwrap(fs.walk(dir)) {
            let rel = str.substring(f, str.length(dir) + 1, str.length(f))
            let kept = (str.ends_with(rel, ".ol") || str.ends_with(rel, ".css") || str.ends_with(rel, ".js") || str.ends_with(rel, ".toml"))
                && !str.starts_with(rel, "tests/") && !str.contains(rel, "node_modules") && !str.starts_with(rel, ".")
            if kept => { files = map_set(files, "/shelf/" + name + "/" + rel, unwrap(fs.read_bytes(f))) } else => ()
        }
    }
    map_set(files, "/home/sim/.olang/shelf.toml", join(lines, "\n") + "\n")
}

/// The checkout's sources, as the node's files under /repo.
fn sources() = {
    let mut files = shelf_files()
    for top in ["lib", "static", "packages"] {
        for f in unwrap(fs.walk(TRACK + "/" + top)) {
            let rel = str.substring(f, str.length(TRACK) + 1, str.length(f))
            if !str.contains(rel, "/tests/") && !str.ends_with(rel, ".db") => {
                files = map_set(files, "/repo/" + rel, unwrap(fs.read_bytes(f)))
            } else => ()
        }
    }
    for top in ["api.ol", "main.ol", "client.ol", "olang.toml", "olang.lock"] {
        files = map_set(files, "/repo/" + top, unwrap(fs.read_bytes(TRACK + "/" + top)))
    }
    // the folder the database lives in
    map_set(files, "/data/.keep", "")
}

let URL = "http://track:7500/api/rpc/"

fn rpc(name, payload) = match http.post(URL + name, unwrap(json.stringify(payload)),
        #{ "timeout_ms": 5000, "headers": #{ "content-type": "application/json" } }) {
    Ok(r) => if r.status == 200 => Ok(unwrap(json.parse(r.body))) else => Err("status " + to_string(r.status) + ": " + r.body),
    Err(e) => Err(show(e))
}

/// An agent: files an issue every 1–5 s, remembers each one the server
/// acknowledged (its id and summary), and at the end checks the server
/// still has every one.
fn agent(name, until) = {
    let mut acked = []
    let mut n = 0
    let mut refused = 0
    while time.monotonic_ms() < until {
        time.sleep(random.randint(1000, 5000))
        n = n + 1
        let summary = name + " issue " + to_string(n)
        match rpc("issues.create", #{ "project_id": 1, "summary": summary }) {
            Ok(r) => { acked = acked + [#{ "id": r.data.id, "summary": summary }] },
            Err(e) => { refused = refused + 1 }
        }
    }
    // the server must be back; then every acknowledged issue is there
    let mut ready = is_ok(rpc("version", #{}))
    while !ready {
        time.sleep(1000)
        ready = is_ok(rpc("version", #{}))
    }
    for a in acked {
        // an answer, however long the server is down or draining (crashes
        // go on): only a 4xx or another issue under the id is a loss
        let mut got = rpc("issues.get", #{ "id": a.id })
        while is_err(got) && !str.starts_with(show(got), "Err(\"status 4") {
            time.sleep(1000)
            got = rpc("issues.get", #{ "id": a.id })
        }
        assert(is_ok(got) && unwrap(got).data.issue.summary == a.summary,
            "lost an acknowledged write: " + name + "'s issue " + to_string(a.id) + " (" + a.summary + "): " + show(got))
    }
    #{ "acked": len(acked), "refused": refused }
}

let w = sim.world(#{ "seed": 1, "net": #{ "latency": (1, 10) } })
w.node("track", [TRACK + "/api.ol", "7500", "/data/track.db", "--data-dir", "/data/track.d"], #{
    "cwd": "/repo",
    "files": sources(),
    "env": #{ "HOME": "/home/sim", "SHUTTLE_BANNER": "off", "SHUTTLE_LOG": "warn", "OLANG_HTTP_WORKERS": "4" } })
for i in range(1, 3) {
    let name = "agent" + to_string(i)
    w.client(name, () => agent(name, sim.minutes(MINUTES)))
}
// every EVERY seconds (30), a coin: heads, the server goes down somewhere
// in the next two thirds of the interval — killed, stopped (SIGTERM: its
// shutdown handler drains it), or with "power", the power goes — and
// comes back three seconds after it is down
let EVERY = match os.get_env("EVERY") { Ok(e) => unwrap(str.parse_int(e)), Err(e) => 30 }
if CRASH != "none" => w.every(sim.seconds(EVERY), () => {
    if random.random() < 0.6 => {
        time.sleep(random.randint(0, EVERY * 666))
        let back = #{ "restart_after": sim.seconds(3) }
        let r = attempt(() => if CRASH == "power" => w.crash("track", back)
            else if random.random() < 0.5 => w.stop("track", back)
            else => w.kill("track", back))
    } else => ()
}) else => ()
let run = w.run(sim.minutes(MINUTES + 10))
println(run.outcome + " " + to_string(run.value) + " crashes: " +
    to_string(len(filter(run.faults, (f) => f.kind == "crash" || f.kind == "kill" || f.kind == "stop"))))
