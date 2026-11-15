// compat: concurrency — spawn and task.join, join_timeout, channels,
// par_map and par_filter, par for, shared, cancel. Output is ordered by
// the program, never by the scheduler.

fn work(n) = n * n
let tasks = range(1, 6) |> map((n) => spawn work(n))
println(tasks |> map((t) => unwrap(task.join(t))))

let slow = spawn { time.sleep(200); "late" }
println(task.join_timeout(slow, 5))
println(task.join(slow))

let c = chan.new()
let producer = spawn {
    for i in range(0, 4) { chan.send(c, i * 10) }
    chan.close(c)
}
let mut got = []
loop {
    match chan.recv(c) {
        Ok(v) => got = got + [v],
        Err(_) => break
    }
}
task.join(producer)
println(got)

let bounded = chan.bounded(2)
chan.send(bounded, "a")
println(chan.try_recv(bounded))
println(chan.try_recv(bounded))

println(par_map(range(0, 8), (v) => v * 3))
println(par_filter(range(0, 10), (v) => v % 3 == 0))

let counter = shared.new(0)
let workers = map(range(0, 4), (i) => spawn {
    for k in range(0, 100) { shared.swap(counter, (n) => n + 1) }
})
for w in workers { unwrap(task.join(w)) }
println(shared.get(counter))
println(shared.compare_and_set(counter, 400, 0))
println(shared.get(counter))

let out = chan.new()
par for i in range(0, 4) { chan.send(out, i) }
chan.close(out)
let mut seen = []
loop { match chan.recv(out) { Ok(v) => seen = seen + [v], Err(_) => break } }
println(sort(seen))

let token = cancel.token()
println(cancel.is_set(token))
cancel.set(token)
println(cancel.is_set(token))
