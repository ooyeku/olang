// A key-value store on an append-only log, served over http:
//
//   POST /put  #{ "key": k, "version": n }   -> 200 once the put is kept
//   GET  /get?key=k                          -> #{ "version": n } (0 if none)
//
// A key's versions go up by one: a put of the version the store holds is
// a retry (200, nothing written), and any other but the next is refused
// with 409 and the version the store holds. A writer that hears 409 for
// the version after the one it was acknowledged has lost a write.
//
// One task owns the log (an actor the four workers ask), so the log is
// written in one order. On start it replays the log; a torn last line (a
// crash kept part of an append that was never synced) is cut off, and the
// cut synced, before anything is served.
//
// MODE (env) is when a put is acknowledged:
//   "sync_then_ack"  append, fs.sync, then answer        (correct)
//   "ack_then_sync"  append, answer; a task syncs the log every 500 ms
//                    (group commit done wrong: an acked put can be lost)

let LOG = "/data/store.log"
let mode = match os.get_env("MODE") { Ok(m) => m, Err(e) => "sync_then_ack" }

/// The log as a map of key to version; the log's torn tail cut off.
fn replay() = {
    let text = match fs.read_file(LOG) { Ok(t) => t, Err(e) => "" }
    let parts = str.split(text, "\n")
    let whole = take(parts, len(parts) - 1)
    if text != "" && !str.ends_with(text, "\n") => {
        let kept = if len(whole) == 0 => "" else => join(whole, "\n") + "\n"
        unwrap(fs.write_file(LOG, kept))
        unwrap(fs.sync(LOG))
    } else => ()
    fold(whole, #{}, (acc, line) => {
        let kv = str.split(line, " ")
        if len(kv) == 2 => map_set(acc, kv[0], unwrap(str.parse_int(kv[1]))) else => acc
    })
}

fn store(requests, start) = {
    let mut versions = start
    let mut serving = true
    while serving {
        match chan.recv(requests) {
            Ok(msg) => {
                let r = msg.req
                let held = map_get(versions, r.key) ?? 0
                if r.op == "put" && r.version == held + 1 => {
                    unwrap(fs.append_file(LOG, r.key + " " + to_string(r.version) + "\n"))
                    if mode == "sync_then_ack" => unwrap(fs.sync(LOG)) else => ()
                    versions = map_set(versions, r.key, r.version)
                    chan.send(msg.reply, #{ "ok": true, "version": r.version })
                }
                else if r.op == "put" => chan.send(msg.reply, #{ "ok": r.version == held, "version": held })
                else => chan.send(msg.reply, #{ "ok": true, "version": held })
            }
            Err(e) => { serving = false }
        }
    }
}

if !fs.exists("/data") => unwrap(fs.create_dir("/data")) else => ()
let requests = chan.new()
spawn store(requests, replay())
if mode == "ack_then_sync" => {
    let syncer = spawn {
        while true {
            time.sleep(500)
            let synced = fs.sync(LOG)
        }
    }
} else => ()

fn handle(req) = {
    if req.method == "POST" && req.path == "/put" => {
        let r = unwrap(json.parse(req.body))
        let a = unwrap(chan.ask(requests, #{ "op": "put", "key": r.key, "version": r.version }))
        http.response(if a.ok => 200 else => 409, unwrap(json.stringify(#{ "version": a.version })))
    }
    else if req.path == "/get" => {
        let a = unwrap(chan.ask(requests, #{ "op": "get", "key": req.query.key }))
        http.response(200, unwrap(json.stringify(#{ "version": a.version })))
    }
    else => http.response(404, "no route for " + req.path)
}

http.serve(7600, handle)
