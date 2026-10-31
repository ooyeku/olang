// A workload for log_store.ol (and sqlite_store.ol): a writer puts its
// own key at versions 1, 2, 3, ... every 50–400 ms, counting a put as
// acknowledged only when the store answered 200. The property: an
// acknowledged write survives a crash. The store holds versions in
// order, so the first put after a crash that lost an acknowledged one is
// refused (409) with the version the store fell back to.
//
// Each put is recorded in the history "puts" (sim.record: nothing outside
// a world): invoked, then ok (acknowledged), fail (refused) or info (no
// answer: it may or may not have been kept). A property reads it
// (log_props.ol).

/// "ok", "down" (no answer: not acknowledged, retried), or the version
/// the store holds when it refused the put.
fn put(server, key, version) = match http.post(server + "/put",
        unwrap(json.stringify(#{ "key": key, "version": version })), #{ "timeout_ms": 1000 }) {
    Ok(r) => if r.status == 200 => "ok"
        else if r.status == 409 => unwrap(json.parse(r.body)).version
        else => "down",
    Err(e) => "down"
}

/// The version the store holds, or () when it cannot be reached.
fn version_of(server, key) = match http.get(server + "/get?key=" + key, #{ "timeout_ms": 1000 }) {
    Ok(r) => if r.status == 200 => unwrap(json.parse(r.body)).version else => (),
    Err(e) => ()
}

fn lost(key, acked, held) = assert(false, "lost an acknowledged write: " + key + " was acked at version " +
    to_string(acked) + ", the store holds " + to_string(held))

share fn writer(server, key, until) = {
    let mut acked = 0
    let mut attempts = 0
    while time.monotonic_ms() < until {
        time.sleep(random.randint(50, 400))
        attempts = attempts + 1
        sim.record("puts", #{ "type": "invoke", "f": "put", "value": [key, acked + 1] })
        let answer = put(server, key, acked + 1)
        if answer == "ok" => {
            sim.record("puts", #{ "type": "ok", "value": [key, acked + 1] })
            acked = acked + 1
        }
        else if answer != "down" => {
            sim.record("puts", #{ "type": "fail" })
            lost(key, acked, answer)
        }
        else => sim.record("puts", #{ "type": "info" })
    }
    // the last word: wait for the store to be back, then read
    let mut v = version_of(server, key)
    while v == () {
        time.sleep(500)
        v = version_of(server, key)
    }
    if v < acked => lost(key, acked, v) else => ()
    #{ "acked": acked, "attempts": attempts }
}
