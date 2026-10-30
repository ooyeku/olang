// A lost update, found by a seed.
//
// A counter lives in a server task. Two clients each add one to it, but
// through a get-then-set protocol: the read and the write are two
// messages, so a client can read, be overtaken by the other client, and
// write back a stale value. Most interleavings are fine; some lose an
// increment, and the final assertion fails.
//
//   olang sim tests/fixtures/sim/lost_update.ol --seeds 50   (finds it)
//   olang sim tests/fixtures/sim/lost_update.ol --seed S     (replays it)

fn server(requests) = {
    let mut value = 0
    let mut serving = true
    while serving {
        match chan.recv(requests) {
            Ok(msg) => {
                if msg.req.op == "get" => chan.send(msg.reply, value)
                else if msg.req.op == "set" => {
                    value = msg.req.value
                    chan.send(msg.reply, value)
                }
                else => {
                    chan.send(msg.reply, value)
                    serving = false
                }
            }
            Err(e) => { serving = false }
        }
    }
    value
}

fn client(requests) = {
    let seen = unwrap(chan.ask(requests, #{ "op": "get" }))
    unwrap(chan.ask(requests, #{ "op": "set", "value": seen + 1 }))
}

let requests = chan.new()
let s = spawn server(requests)
let a = spawn client(requests)
let b = spawn client(requests)
unwrap(task.join(a))
unwrap(task.join(b))
let total = unwrap(chan.ask(requests, #{ "op": "stop" }))
assert_eq(total, 2, "lost update: two increments, counter is " + to_string(total))
println("counter = " + to_string(total))
