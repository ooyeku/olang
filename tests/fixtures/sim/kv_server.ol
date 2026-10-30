// A counter store served the way olang-track serves its rpc: POST /rpc
// with a JSON body, a JSON answer. The state lives in one task (an
// actor the handlers ask), so the server's four workers share it.
//
//   #{ "op": "add", "key": k, "by": n, "id": request_id }  -> #{ ok, value }
//   #{ "op": "get", "key": k }                              -> #{ ok, value }
//
// `add` is idempotent by `id`: a request id seen before answers what it
// answered the first time and changes nothing. A client that retries
// with the same id is safe; one that retries under a new id is not.

let args = os.args()
let port = if len(args) > 1 => to_int(args[1]) else => 7500

fn store(requests) = {
    let mut values = #{}
    let mut seen = #{}
    let mut serving = true
    while serving {
        match chan.recv(requests) {
            Ok(msg) => {
                let r = msg.req
                if r.op == "add" => {
                    if map_has_key(seen, r.id) => chan.send(msg.reply, map_get(seen, r.id))
                    else => {
                        let v = (map_get(values, r.key) ?? 0) + r.by
                        values = map_set(values, r.key, v)
                        seen = map_set(seen, r.id, v)
                        chan.send(msg.reply, v)
                    }
                }
                else => chan.send(msg.reply, map_get(values, r.key) ?? 0)
            }
            Err(e) => { serving = false }
        }
    }
}

let requests = chan.new()
spawn store(requests)

fn handle(req) = {
    if req.method != "POST" || req.path != "/rpc" => http.response(404, "no route for " + req.path)
    else => match json.parse(req.body) {
        Ok(r) => {
            let v = unwrap(chan.ask(requests, r))
            http.response_with_headers(200, unwrap(json.stringify(#{ "ok": true, "value": v })),
                #{ "Content-Type": "application/json" })
        }
        Err(e) => http.response(400, "bad json")
    }
}

http.serve(port, handle)
