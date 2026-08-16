// A self-contained HTTP load test — the server and its concurrent client
// fleet in one program. It boots the API in a spawned task, waits for the
// port, fans a fleet of client workers out across spawned threads (each
// firing a burst of requests and timing them), merges the per-worker stats
// after `await`, and checks the result against the server's own hit count.
//
//   olang main.ol [clients] [requests_per_client] [server_workers]
//
// The point: exercise http.serve under genuine concurrent load, driven by
// olang's own spawn/await — no external benchmark tool.

use lib.server { make_server, server_hit_count }
use lib.stats { empty_stats, record, merge }

let args = unwrap(os.args())
let clients = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 12
let per_client = if len(args) > 2 => unwrap(str.parse_int(args[2])) else => 25
let server_workers = if len(args) > 3 => unwrap(str.parse_int(args[3])) else => 8

let port = 18450
let app = make_server(port, server_workers)
let base = "http://127.0.0.1:" + show(port)

// ── wait for the listener to come up ──
fn wait_ready(tries) = if tries <= 0 => false
    else => match http.get(base + "/count") {
        Ok(r) => true,
        Err(e) => { time.sleep(25); wait_ready(tries - 1) }
    }
if !wait_ready(40) => {
    println("server never became ready")
    os.exit(1)
}

// ── one client worker: fire `per_client` requests, most normal, some to the
// deliberately-failing route, timing each ──
fn run_client(id) = {
    let mut s = empty_stats()
    for i in 0..per_client {
        let expect_error = (i % 10) == 7          // ~10% hit the failing route
        let path = if expect_error => "/oops" else => "/w" + show(id) + "-" + show(i)
        let t0 = time.monotonic_ms()
        let status = match http.get(base + path) {
            Ok(r) => r.status,
            Err(e) => 0
        }
        let ms = time.monotonic_ms() - t0
        s = record(s, status, ms, expect_error)
    }
    s
}

// ── fan out, await, merge ──
println(str.fmt("load test: {} clients x {} requests -> {} ({} server workers)",
    clients, per_client, base, server_workers))
let t0 = time.monotonic_ms()
let workers = range(0, clients) |> map((id) => spawn run_client(id))
let per_worker = workers |> map((w) => try { await w } catch (e) { empty_stats() })
let elapsed = time.monotonic_ms() - t0
let total = per_worker |> fold(empty_stats(), merge)

// ── report ──
let requests_that_should_persist = total.oks + total.expected_5xx
let recorded = server_hit_count(app.conn)
let throughput = if elapsed > 0 => (total.requests * 1000) / elapsed else => 0

println("")
println(str.fmt("  requests:      {}", total.requests))
println(str.fmt("  2xx ok:        {}", total.oks))
println(str.fmt("  expected 500:  {}   (the /oops route)", total.expected_5xx))
println(str.fmt("  shed (503):    {}", total.shed_503))
println(str.fmt("  failures:      {}", total.failures))
println(str.fmt("  latency:       avg {}ms  max {}ms",
    if total.requests > 0 => total.total_ms / total.requests else => 0, total.max_ms))
println(str.fmt("  wall clock:    {}ms  (~{} req/s)", elapsed, throughput))
println("")
println(str.fmt("  server recorded {} hits; clients persisted {} (2xx + expected 500)",
    recorded, requests_that_should_persist))

// ── self-check: the server counts exactly what the clients think they landed,
// no request is unaccounted for, and no unexpected failures slipped in ──
test "http.serve stays correct under concurrent load" {
    assert_eq(total.requests, clients * per_client, "every request is accounted for")
    assert_eq(total.failures, 0, "no unexpected failures")
    assert_eq(recorded, requests_that_should_persist,
        "server-side hit count matches client successes exactly")
    assert_true(total.oks > 0, "the fleet actually did work")
}

