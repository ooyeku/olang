// A workload for kv_server.ol: a client adds 1 to its own key every
// half-second to three seconds for an hour, retrying what times out,
// then reads its key back and checks the server counted every add once.
//
// `buggy` plants the bug: a retry goes out under a fresh request id, so
// the server cannot tell it from a new add. When the first attempt was
// applied and only its answer was lost, the add counts twice. On a
// quiet network nothing times out and the bug never shows; a partition
// that falls between a request and its answer exposes it.

let SERVER = "http://kv:7500/rpc"
let TIMEOUT = 2000

fn rpc(body) = http.post(SERVER, unwrap(json.stringify(body)),
    #{ "timeout_ms": TIMEOUT, "headers": #{ "content-type": "application/json" } })

/// Ask until answered; how many retries it took, and the value.
fn until_answered(name, body, buggy) = {
    let mut tries = 0
    let mut value = ()
    let mut req = body
    while value == () {
        match rpc(req) {
            Ok(resp) => {
                if resp.status == 200 => { value = unwrap(json.parse(resp.body)).value }
                else => { time.sleep(200) }
            }
            Err(e) => {
                tries = tries + 1
                time.sleep(500 * tries)
                // the bug: the retry is a new request, not the same one again
                if buggy => { req = map_set(req, "id", to_string(body.id) + "/retry" + to_string(tries)) }
                else => ()
            }
        }
    }
    (tries, value)
}

share fn client(name, buggy, until) = {
    let mut adds = 0
    let mut retries = 0
    while time.monotonic_ms() < until {
        time.sleep(random.randint(500, 3000))
        adds = adds + 1
        let id = name + "-" + to_string(adds)
        let (tries, v) = until_answered(name, #{ "op": "add", "key": name, "by": 1, "id": id }, buggy)
        retries = retries + tries
    }
    let (tries, total) = until_answered(name, #{ "op": "get", "key": name, "id": name + "-read" }, false)
    assert_eq(total, adds, "the server counted " + to_string(total) + " adds, the client made " + to_string(adds))
    #{ "adds": adds, "retries": retries + tries }
}
