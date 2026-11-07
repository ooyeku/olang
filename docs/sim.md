# Simulating whole systems

Some bugs only show up when things happen in an unlucky order: two
requests arrive at the same moment, the network drops an answer after
the server has acted on it, or the power goes between a write and the
moment it reaches the disk. Ordinary tests rarely catch them, because
an ordinary run takes whatever order the machine happens to give it.
When one of these bugs does turn up, it is usually gone again by the
time anyone looks.

`olang sim` runs a program, or a whole system of programs, inside a
simulated world. The world has its own clock, its own network and a
disk for each machine in it, and a number called the seed decides every
choice the world makes: which task runs next, how long a message takes,
when a crash happens. Running a different seed explores a different
order of events, and running the same seed again gives exactly the same
run. So you can search thousands of orders for a failure, and when one
fails, replay it as often as you need to understand it.

Nothing in a world touches the real machine. Its files, sockets and
clock are all simulated, which is why a world can crash a server over
and over without hurting anything.

`sim` is new in this release and
[Experimental](stability.md#experimental--may-change-in-a-minor-release):
the same seed will always give the same run, but the options and a
run's fields may still change in a minor release.

Part of [the olang book](README.md) ·
[Command-line tooling](tooling.md#olang-sim) ·
[Standard library](stdlib.md#sim--simulated-worlds)

## Table of contents

- [A race, found and replayed](#a-race-found-and-replayed)
- [Nodes and the network](#nodes-and-the-network)
- [The disk and crashes](#the-disk-and-crashes)
- [Saying what must hold](#saying-what-must-hold)
- [Many seeds, shrinking and replaying](#many-seeds-shrinking-and-replaying)
- [Seeing a run](#seeing-a-run)
- [Buggify points](#buggify-points)
- [Simulation in `olang test`](#simulation-in-olang-test)
- [What a world simulates, and what it refuses](#what-a-world-simulates-and-what-it-refuses)

## A race, found and replayed

Here is a bank account kept by one task. Other tasks talk to it over a
channel. To withdraw money, a task asks for the balance, checks there
is enough, and then tells the account the new balance:

```olang no-run
// The account lives in one task. It answers two requests: "balance"
// (how much is there) and "set" (make it this much).
fn account(inbox) = {
    let mut balance = 100
    let mut open = true
    while open {
        match chan.recv(inbox) {
            Ok(msg) => {
                if msg.req.op == "set" => { balance = msg.req.to } else => ()
                chan.send(msg.reply, balance)
            }
            Err(_) => { open = false }
        }
    }
}

// Withdraw by asking for the balance, then setting the new one.
fn withdraw(inbox, amount) = {
    time.sleep(random.randint(0, 10))
    let balance = unwrap(chan.ask(inbox, #{ "op": "balance" }))
    if balance >= amount => {
        unwrap(chan.ask(inbox, #{ "op": "set", "to": balance - amount }))
        true
    } else => false
}

let inbox = chan.new()
spawn account(inbox)
let a = spawn withdraw(inbox, 30)
let b = spawn withdraw(inbox, 50)
unwrap(task.join(a))
unwrap(task.join(b))
let left = unwrap(chan.ask(inbox, #{ "op": "balance" }))
println("left: " + to_string(left))
assert_eq(left, 20, "two withdrawals of 30 and 50 from 100")
```

Save it as `bank.ol` and run it, and it almost certainly works:

```bash
$ olang bank.ol
left: 20
```

The bug is that a withdrawal is two messages. If both tasks read the
balance before either of them sets it, they both see 100, and one
withdrawal is lost. On a real machine that needs the two tasks to wake
within microseconds of each other, so it hardly ever happens. In a
simulated world it is just one of the orders the seed can pick:

```bash
$ olang sim bank.ol --seeds 50
seed 17: failed at 0.006s virtual: two withdrawals of 30 and 50 from 100
seed 29: failed at 0.004s virtual: two withdrawals of 30 and 50 from 100
seed 31: failed at 0.009s virtual: two withdrawals of 30 and 50 from 100
seed 33: failed at 0.002s virtual: two withdrawals of 30 and 50 from 100
seed 44: failed at 0.008s virtual: two withdrawals of 30 and 50 from 100
olang sim: 50 seed(s) · 45 passed · 5 failed · 14.7s simulated per second (0.00 virtual hours) · 1 job(s) · 0.024 s real
digest: 6888db0dd89379d8
reproduce: olang sim bank.ol --seed 17
```

Two things are different inside a world. The first is time. The clock
is virtual: running code takes no time at all, and when every task is
waiting, the clock jumps straight to the next thing due to happen. A
`time.sleep(6)` costs nothing real, and neither would a sleep of six
hours. The second is scheduling. Only one task runs at a time, and at
every point where a task could give way to another (a `spawn`, a send
or receive on a channel, a `task.join`, a sleep), the seed picks which
task goes next. Random numbers come from the seed too, so each task's
`random.randint` gives the same answers in every run of that seed.

Because of that, seed 17 fails every time. Ask for its events to see
what happened:

```bash
$ olang sim bank.ol --seed 17 --events
left: 50
events:
          0.000s  main     task 0   start    seed 17
          0.000s  main     task 0   spawn    task 1
          0.000s  main     task 0   spawn    task 2
          0.000s  main     task 0   spawn    task 3
          0.000s  main     task 2   sleep    6 ms
          0.000s  main     task 3   sleep    6 ms
          0.006s  main     task 3   send     #1 <- #{"req": #{"op": "balance"}, "reply": <channel>}
          0.006s  main     task 1   recv     #1 -> #{"req": #{"op": "balance"}, "reply": <channel>}
          0.006s  main     task 2   send     #1 <- #{"req": #{"op": "balance"}, "reply": <channel>}
          0.006s  main     task 1   send     #2 <- 100
          0.006s  main     task 1   recv     #1 -> #{"req": #{"op": "balance"}, "reply": <channel>}
          0.006s  main     task 1   send     #3 <- 100
          0.006s  main     task 3   recv     #2 -> 100
          0.006s  main     task 2   recv     #3 -> 100
          0.006s  main     task 2   send     #1 <- #{"req": #{"op": "set", "to": 70}, "reply": <channel>}
          0.006s  main     task 3   send     #1 <- #{"req": #{"op": "set", "to": 50}, "reply": <channel>}
          ...
          0.006s  main     task 0   print    left: 50
          0.006s  main     task 0   failed   two withdrawals of 30 and 50 from 100
olang sim: seed 17: failed at 0.006s virtual: two withdrawals of 30 and 50 from 100
```

Each line is a time, the machine (here there is only `main`), the
task, what it did and the details. Task 1 is the account, tasks 2 and
3 are the withdrawals. Both drew a 6 ms sleep, both woke at the same
instant, both asked for the balance and were told 100, and then they
set it to 70 and to 50 in turn. The channels are numbered in the order
the world first saw them, so the same seed prints the same numbers in
any process, on any machine.

The fix is to let the account do the check and the subtraction itself,
in one step that nothing can interrupt:

```olang no-run
fn account(inbox) = {
    let mut balance = 100
    let mut open = true
    while open {
        match chan.recv(inbox) {
            Ok(msg) => {
                let r = msg.req
                if r.op == "withdraw" && balance >= r.amount => {
                    balance = balance - r.amount
                    chan.send(msg.reply, true)
                } else if r.op == "withdraw" => chan.send(msg.reply, false)
                else => chan.send(msg.reply, balance)
            }
            Err(_) => { open = false }
        }
    }
}

fn withdraw(inbox, amount) = {
    time.sleep(random.randint(0, 10))
    unwrap(chan.ask(inbox, #{ "op": "withdraw", "amount": amount }))
}
```

Now a thousand seeds pass:

```bash
$ olang sim bank.ol --seeds 1000
olang sim: 1000 seed(s) · 1000 passed · 0 failed · 18.8s simulated per second (0.01 virtual hours) · 1 job(s) · 0.364 s real
digest: 00e50f83cbbcc4ec
```

A program like `bank.ol` needs no changes to run in a world. `olang
sim` runs it as the world's main program, and the same file still runs
normally with `olang bank.ol`.

## Nodes and the network

Most of the bugs worth finding live between programs. A world can hold
several machines, called nodes, joined by a simulated network. A node
is either an olang program, started as if from the command line, or an
olang function, which is handy for the clients that drive a test.

Here is a small counter service. A client posts a key, the server adds
one to that key and answers the new total:

```olang no-run
// A tiny counter service. POST /add with #{ "key": k } adds one to k
// and answers the new total; GET /get?key=k answers the total.
// The counts live in one task, which the server's workers ask.
fn counts(inbox) = {
    let mut totals = #{}
    let mut open = true
    while open {
        match chan.recv(inbox) {
            Ok(msg) => {
                let r = msg.req
                let now = (totals[r.key] ?? 0) + (if r.op == "add" => 1 else => 0)
                totals = map_set(totals, r.key, now)
                chan.send(msg.reply, now)
            }
            Err(_) => { open = false }
        }
    }
}

let inbox = chan.new()
spawn counts(inbox)

fn handle(req) = {
    if req.method == "POST" && req.path == "/add" => {
        let body = unwrap(json.parse(req.body))
        let total = unwrap(chan.ask(inbox, #{ "op": "add", "key": body.key }))
        http.response(200, to_string(total))
    }
    else if req.path == "/get" => {
        let total = unwrap(chan.ask(inbox, #{ "op": "get", "key": req.query.key }))
        http.response(200, to_string(total))
    }
    else => http.response(404, "no such route")
}

http.serve(7500, handle)
```

That is `counter.ol`, an ordinary server you could run with `olang
counter.ol`. The world around it is a second file, `counter_world.ol`:

```olang no-run
// One counter server and three clients on a slow network. Each client
// adds to its own key for half an hour, then checks the server's total.
let w = sim.world(#{ "net": #{ "latency": (10, 200) } })
w.node("counter", ["olang", "counter.ol"])

// Keep asking until the server answers. A request that times out is
// sent again.
fn add(key) = {
    let mut answered = false
    while !answered {
        let body = unwrap(json.stringify(#{ "key": key }))
        match http.post("http://counter:7500/add", body, #{ "timeout_ms": 1000 }) {
            Ok(resp) => { answered = resp.status == 200 }
            Err(_) => time.sleep(500)
        }
    }
}

fn client(key) = {
    let mut added = 0
    while time.monotonic_ms() < sim.minutes(30) {
        time.sleep(random.randint(1000, 3000))
        add(key)
        added = added + 1
    }
    let resp = unwrap(http.get("http://counter:7500/get?key=" + key))
    assert(resp.body == to_string(added), "the server counted " + resp.body + ", the client added " + to_string(added))
}

for name in ["a", "b", "c"] {
    w.client(name, () => client(name))
}

// Now and then, one client loses touch with the server for a while.
w.every(sim.minutes(5), () => {
    let victim = ["a", "b", "c"][random.randint(0, 2)]
    w.partition(["counter"], [victim])
    time.sleep(random.randint(2000, 20000))
    w.heal()
})

let run = w.run(sim.hours(1))
println(run.outcome)
```

`sim.world` makes the world, and `"latency": (10, 200)` says each
message takes between 10 and 200 ms. `w.node` adds the server, named
`counter`, which is also its host name: a client in the world reaches
it at `http://counter:7500`. `w.client` adds a node that runs a
function. `w.every` runs a function every five simulated minutes. Here
it cuts one client off from the server with `w.partition`, waits, and
joins them again with `w.heal`. Finally `w.run` runs the world for up
to an hour. It ends sooner, as `done`, once every client has finished.

When the file is given to `olang sim`, it runs as an ordinary program,
and every world it builds takes its seed from the command line:

```bash
$ olang sim counter_world.ol --seeds 10
seed 2: failed at 30m 10.722s virtual: client c: the server counted 819, the client added 818
seed 4: failed at 30m 05.992s virtual: client c: the server counted 807, the client added 806
seed 8: failed at 30m 17.903s virtual: client c: the server counted 807, the client added 806
seed 10: failed at 30m 00.370s virtual: client c: the server counted 794, the client added 793
olang sim: 10 seed(s) · 6 passed · 4 failed · 4h 20m simulated per second (4.34 virtual hours) · 1 job(s) · 1.158 s real
digest: 793b3e5b795db6ae
reproduce: olang sim counter_world.ol --seed 2
```

Ten half-hour runs of four programs took just over a second. The
server counted one add too many. Replaying seed 2 with `--events` and
looking around the end of the run shows why (the server's own lines
are trimmed here):

```text
     29m 59.800s  c        task 4   http     #2447 POST http://counter:7500/add
     29m 59.921s  counter  net      deliver  request #2447 c → counter POST /add
     29m 59.921s  counter  task 9   handle   #2447 POST /add
     29m 59.921s  counter  task 9   respond  #2447 200
     30m 00.000s  hook-1   task 5   partition [counter] | [c]
     30m 00.000s  hook-1   task 5   sleep    9224 ms
     30m 00.038s  c        net      drop     response #2447 200 counter → c: partition (in flight)
     30m 00.800s  c        task 4   timeout  #2447 after 1000 ms
     30m 00.800s  c        task 4   sleep    500 ms
```

Client `c` sent an add, the server counted it and answered, and the
partition began while the answer was on its way back. The client never
heard, timed out and sent the add again, so the server counted it
twice. A client cannot tell a lost request from a lost answer, so
retrying is only safe if the server can recognise a retry.

The usual fix is to give each request an id and have the server
remember the ids it has already applied. Save the fixed server as
`counter_ids.ol`, so the first one stays around for later:

```olang no-run
fn counts(inbox) = {
    let mut totals = #{}
    let mut seen = #{}
    let mut open = true
    while open {
        match chan.recv(inbox) {
            Ok(msg) => {
                let r = msg.req
                if r.op == "add" && map_has_key(seen, r.id) => chan.send(msg.reply, seen[r.id])
                else => {
                    let now = (totals[r.key] ?? 0) + (if r.op == "add" => 1 else => 0)
                    totals = map_set(totals, r.key, now)
                    if r.op == "add" => { seen = map_set(seen, r.id, now) } else => ()
                    chan.send(msg.reply, now)
                }
            }
            Err(_) => { open = false }
        }
    }
}
```

The handler passes `body.id` along, and in the world the node becomes
`["olang", "counter_ids.ol"]` and the client sends the same id on every
retry:

```olang no-run
fn add(key, id) = {
    let body = unwrap(json.stringify(#{ "key": key, "id": id }))
    let mut answered = false
    while !answered {
        match http.post("http://counter:7500/add", body, #{ "timeout_ms": 1000 }) {
            Ok(resp) => { answered = resp.status == 200 }
            Err(_) => time.sleep(500)
        }
    }
}
```

with `add(key, key + "-" + to_string(added))` in the loop. Now a
hundred seeds pass, run four at a time:

```bash
$ olang sim counter_world.ol --seeds 100 --jobs 4
olang sim: 100 seed(s) · 100 passed · 0 failed · 9h 06m simulated per second (9.11 virtual hours) · 4 job(s) · 5.527 s real
digest: 8b0d901382138f97
```

Partitions are only one thing the network can do. The `net` option
also takes `loss` and `dup`, the chance that a message is lost or
delivered twice, and `w.link(a, b, opts)` sets these for one pair of
nodes. A partition that starts while a message is in flight drops it,
as you saw. Requests and answers between nodes are ordinary `http`
calls: `http.serve`, `http.get`, `http.post` and the rest work as they
do outside, and nothing opens a real socket.

## The disk and crashes

Each node has a disk of its own, kept in memory. It behaves the way a
real disk behaves when the power fails, which is worse than most
people expect. A write is visible to the program at once, but it is
only safe once it has been synced, with `fs.sync`. Until then it sits
in the operating system's cache, and a power cut loses it. A real
machine usually writes its cache out within half a minute or so, so
the window is small and the bug is rare. The simulated disk keeps
nothing that was not synced, which makes the bug easy to find.

Here is a store that keeps every put as a line in a log file. To save
work, it does not sync each put; a task syncs the log twice a second.
This is `fast_store.ol`:

```olang no-run
// A store that keeps every put as a line in a log on its disk.
//   POST /put    body "key n"   answers 200 once the put is kept
//   GET  /get?key=k             answers the numbers kept for k, one a line
// To save work it does not sync each put: a task syncs the log twice
// a second.
let LOG = "/data/puts.log"

fn kept(key) = match fs.read_file(LOG) {
    Ok(text) => str.split(text, "\n")
        |> filter((line) => str.starts_with(line, key + " "))
        |> map((line) => str.split(line, " ")[1]),
    Err(_) => []
}

fn handle(req) = {
    if req.method == "POST" && req.path == "/put" => {
        unwrap(fs.append_file(LOG, req.body + "\n"))
        http.response(200, "kept")
    }
    else if req.path == "/get" => http.response(200, join(kept(req.query.key), "\n"))
    else => http.response(404, "no such route")
}

if !fs.exists("/data") => unwrap(fs.create_dir("/data")) else => ()
spawn {
    while true {
        time.sleep(500)
        let synced = fs.sync(LOG)
    }
}
http.serve(7600, handle)
```

The world, `store_world.ol`, has three writers that each put
numbers 1, 2, 3 and so on for five minutes, remember which puts the
store acknowledged, and then check that the store still has every one
of them. Every 20 seconds a coin decides whether the power goes:

```olang no-run
let w = sim.world(#{ "net": #{ "latency": (1, 20) } })
w.node("store", ["olang", "fast_store.ol"])

fn writer(key) = {
    let mut acked = []
    let mut n = 0
    while time.monotonic_ms() < sim.minutes(5) {
        time.sleep(random.randint(50, 400))
        n = n + 1
        let body = key + " " + to_string(n)
        match http.post("http://store:7600/put", body, #{ "timeout_ms": 1000 }) {
            Ok(resp) => if resp.status == 200 => { acked = acked + [to_string(n)] } else => (),
            Err(_) => ()
        }
    }
    // Read back what the store kept, waiting for it if it is down.
    let mut kept = ()
    while kept == () {
        match http.get("http://store:7600/get?key=" + key, #{ "timeout_ms": 1000 }) {
            Ok(resp) => { kept = str.split(resp.body, "\n") }
            Err(_) => time.sleep(500)
        }
    }
    let lost = filter(acked, (n) => !contains(kept, n))
    assert(len(lost) == 0, key + " lost acknowledged puts " + to_string(lost))
}

for key in ["x", "y", "z"] {
    w.client(key, () => writer(key))
}

// Every 20 seconds, a coin: heads, the power goes at a random moment
// and comes back a second later.
w.every(sim.seconds(20), () => {
    if random.random() < 0.5 => {
        time.sleep(random.randint(0, 19000))
        w.crash("store", #{ "restart_after": sim.seconds(1) })
    } else => ()
})

let run = w.run(sim.minutes(10))
println(run.outcome)
```

Every seed loses acknowledged puts:

```bash
$ olang sim store_world.ol --seeds 6
seed 1: failed at 5m 00.163s virtual: client x: x lost acknowledged puts ["530", "591", "638", "639", "640", "755", "1030"]
seed 2: failed at 5m 00.105s virtual: client z: z lost acknowledged puts ["130", "245", "246", "360", "361", "362", "947", "948", "974"]
seed 3: failed at 5m 00.042s virtual: client y: y lost acknowledged puts ["131", "306", "318", "429", "987", "988", "989"]
seed 4: failed at 5m 00.165s virtual: client x: x lost acknowledged puts ["294", "506", "1005"]
seed 5: failed at 5m 00.113s virtual: client z: z lost acknowledged puts ["181", "639", "640", "641", "810", "811", "880", "958", "1036"]
seed 6: failed at 5m 00.043s virtual: client z: z lost acknowledged puts ["773", "774"]
olang sim: 6 seed(s) · 0 passed · 6 failed · 40m 07s simulated per second (0.67 virtual hours) · 1 job(s) · 0.748 s real
digest: 85b87243367bdc87
reproduce: olang sim store_world.ol --seed 1
```

The events of seed 4 show the first crash. Writer `y` is told its put
is kept 29 ms before the power goes, and the sync that would have kept
it was due later:

```text
         46.821s  store    task 7   handle   #567 POST /put
         46.821s  store    task 7   fs       append_file /data/puts.log (6 bytes)
         46.821s  store    task 7   respond  #567 200
         46.826s  y        net      deliver  response #567 200 store → y
         46.826s  y        task 3   reply    #567 200 kept
         46.826s  y        task 3   sleep    93 ms
         46.855s  z        task 4   http     #568 POST http://store:7600/put
         46.855s  store    net      crash    store (w.crash, restart after 1000 ms): 1 unsynced write(s) lost
         46.855s  hook-1   task 5   spawn    task 11: restart store
         46.855s  store    task 8   stop     its node crashed
         ...
         46.868s  store    net      drop     request #568 z → store: the node is down
         47.855s  world    task 11  restart  store
```

A crash ends every task on the node and throws away its memory. A
second later the world starts `fast_store.ol` again on a fresh
interpreter, with the disk as the crash left it.

The rule to take away is: sync, then acknowledge. Save the fixed store
as `store.ol`, with the syncing task gone and the handler syncing each
put before it answers:

```olang no-run
if req.method == "POST" && req.path == "/put" => {
    unwrap(fs.append_file(LOG, req.body + "\n"))
    unwrap(fs.sync(LOG))
    http.response(200, "kept")
}
```

With the world pointed at `store.ol`, every seed passes:

```bash
$ olang sim store_world.ol --seeds 40 --jobs 4
olang sim: 40 seed(s) · 40 passed · 0 failed · 1h 13m simulated per second (1.22 virtual hours) · 4 job(s) · 2.724 s real
digest: 48ebc74302c95d31
```

`fs.sync` of a file keeps its bytes and its name. `fs.sync` of a folder
keeps the files created, removed or renamed in it. A sync takes a few
milliseconds of virtual time, and a crash during it keeps nothing of
it.

### Harsher disks, and other ways to stop

A disk can be made to misbehave in more ways with `sim.disk`, passed
as a node's `disk` option:

```olang no-run
w.node("store", ["olang", "store.ol"], #{
    "disk": sim.disk(#{ "fsync_lies": 0.01, "torn_writes": true, "full_at": 64 * 1048576, "slow": 2 }),
    "crash": #{ "p": 0.001, "restart_after": sim.seconds(1) }
})
```

`fsync_lies` is the chance that a sync answers success and keeps
nothing. With `torn_writes`, a crash may keep part of an unsynced write
(its first sectors), the way a real disk can, so a program that reads
its own log back must cope with a broken last line. `full_at` is the
size at which writes start failing, and `slow` makes syncs that many
times slower. The `crash` option crashes the node at random: at every
effect it makes there is a chance `p` that the power goes right there,
between an append and its sync, say. Crashing a node that is already
down does nothing, as pulling the plug of a machine that is off does
nothing: `w.crash`, `w.kill` and `w.stop` on a node that crashed and
has not come back yet (or whose program ended) leave an `ignored`
event and are not faults. So a hook that crashes the server every few
minutes needs no check, even when a random crash got there first.

A node can also be stopped more gently. These are the ways, and what
each keeps:

| How the node stops | Its memory | Its disk | `os.on_shutdown` handlers |
|---|---|---|---|
| `w.crash(n)`, the power going | gone | only what was synced | not run |
| `w.kill(n)`, like SIGKILL | gone | every write (the operating system has them, unsynced) | not run |
| `w.stop(n)`, like SIGTERM | gone once its program returns | every write | run |
| its program returns, or calls `os.exit` | gone | every write | not run |

The first three take `#{ "restart_after": ms }` to bring the node
back. A node also has its own environment (`env`), working directory
(`cwd`), files on its disk from the start (`files`) and standard input
(`stdin`, and `w.write_stdin` later). SQLite works in a world too:
`db.open` opens the database on the node's disk, a crash keeps only
what SQLite had synced, and the next open recovers from that as it
would after a real power cut.

## Saying what must hold

So far each client checked the system itself, at the end. That works,
but it finds the bug late, and the checking code is mixed in with the
workload. A world can instead be given properties, statements about
the whole system that it checks while it runs.

Here is the store's world again, with the writers checking nothing.
They record each put in a history with `sim.record` instead: an
`invoke` when they ask, then `ok` when the store acknowledges or `info`
when they never heard back. Two properties watch the run:

```olang no-run
let w = sim.world(#{ "net": #{ "latency": (1, 20) } })
w.node("store", ["olang", "fast_store.ol"])

fn writer(key) = {
    let mut n = 0
    while time.monotonic_ms() < sim.minutes(5) {
        time.sleep(random.randint(50, 400))
        n = n + 1
        sim.record("puts", #{ "type": "invoke", "f": "put", "value": [key, n] })
        match http.post("http://store:7600/put", key + " " + to_string(n), #{ "timeout_ms": 1000 }) {
            Ok(_) => sim.record("puts", #{ "type": "ok", "value": [key, n] }),
            Err(_) => sim.record("puts", #{ "type": "info" })
        }
    }
}

for key in ["x", "y", "z"] {
    w.client(key, () => writer(key))
}

// (the same power cuts as in store_world.ol)

// The last put the store acknowledged, if any. A property runs at
// every instant of the run, so this reads the history from its end.
fn last_ack(s) = {
    let h = s.history("puts")
    let mut i = len(h) - 1
    while i >= 0 && h[i].type != "ok" { i = i - 1 }
    if i < 0 => () else => h[i].value
}

let durable = sim.always("an acknowledged put is on the disk", (s) => match last_ack(s) {
    () => true,
    [key, n] => {
        let log = s.disk("store").read("/data/puts.log") ?? ""
        str.contains("\n" + log, "\n" + key + " " + to_string(n) + "\n")
    }
})
let back = sim.eventually("the store comes back", sim.seconds(5), (s) => s.up("store"))

let run = w.run(#{ "for": sim.minutes(10), "props": [durable, back] })
println(run.outcome)
```

A property's function gets the world's state, `s`. `s.history(name)`
is a recorded history, `s.up(node)` says whether a node is running, and
`s.disk(node)` is the node's disk as a power cut right now would leave
it: `.read(path)` answers only what was synced. So `durable` says that
the latest put a writer was told was kept is already safe on the disk.
`back` says the store is never down for more than five seconds at a
stretch. The `fast_store.ol` version breaks the first property within
a fraction of a second, without any crash at all:

```bash
$ olang sim store_check.ol --seeds 5
seed 1: "an acknowledged put is on the disk" broke at 0.271s (step 21)
seed 2: "an acknowledged put is on the disk" broke at 0.220s (step 20)
seed 3: "an acknowledged put is on the disk" broke at 0.082s (step 21)
seed 4: "an acknowledged put is on the disk" broke at 0.165s (step 20)
seed 5: "an acknowledged put is on the disk" broke at 0.168s (step 20)
olang sim: 5 seed(s) · 0 passed · 5 failed · 1m 58s simulated per second (0.03 virtual hours) · 1 job(s) · 0.008 s real
digest: ef0c37d2c17f7e56
reproduce: olang sim store_check.ol --seed 1
```

The property does not have to wait for a crash to lose something. It
sees the moment a writer holds an acknowledgement that the disk does
not, and the run stops right there, at the first put, with the events
leading up to it. With `store.ol` in its place, twenty seeds pass.

There are four kinds of property:

- `sim.always(name, f)`: `f` is true at every check.
- `sim.never(name, f)`: `f` is false at every check.
- `sim.eventually(name, within, f)`: `f` is never false for longer than
  `within` ms at a stretch. With a store that takes eight seconds to
  restart, `back` breaks five seconds after the crash with `false since
  36.295s (5001 ms, allowed 5000)`.
- `sim.linearizable(name, history, model)`: checked once, at the end
  (below).

A property is checked at every instant at which something happened:
whenever the world is about to move its clock on, and once more at the
end. Since running code takes no virtual time, that is the finest
grain at which anything can be observed. It also means a property's
function runs thousands of times a run, so it should be cheap. Reading
the history from its end, as `last_ack` does, keeps it so; filtering
the whole history at every instant would make the run many times
slower. A property's function cannot change the world or reach outside
it. A call to `time.now_ms`, `fs`, `http` or anything else with an
effect breaks the property, and says why.

`sim.check(run, props)` checks properties after a run that was made
without them. It runs the same seed again, now watched, and raises
naming the property and the moment it broke.

### Linearizability

Some systems are easier to check by their history than by their state.
Go back to the counter. Each client can record what it asked and what
it was told, and the world can check afterwards that some order of all
the operations, one at a time, explains every answer. That property is
called linearizability, and `sim.kv()` is a model of a key-value store
to check it against. Here is `counter_check.ol`, which runs the first
`counter.ol` (the one that ignores ids):

```olang no-run
let w = sim.world(#{ "net": #{ "latency": (10, 200) } })
w.node("counter", ["olang", "counter.ol"])

// Ask until answered (a retry carries the same body, id and all), and
// answer the total the server sent back.
fn ask(url, body) = {
    let mut total = ()
    while total == () {
        let reply = if body == () => http.get(url, #{ "timeout_ms": 1000 })
            else => http.post(url, body, #{ "timeout_ms": 1000 })
        match reply {
            Ok(resp) => { total = to_int(resp.body) }
            Err(_) => time.sleep(500)
        }
    }
    total
}

fn client(key) = {
    let mut n = 0
    while time.monotonic_ms() < sim.minutes(30) {
        time.sleep(random.randint(1000, 3000))
        n = n + 1
        let body = unwrap(json.stringify(#{ "key": key, "id": key + "-" + to_string(n) }))
        sim.record("counts", #{ "type": "invoke", "f": "add", "value": [key, 1] })
        sim.record("counts", #{ "type": "ok", "value": ask("http://counter:7500/add", body) })
    }
    sim.record("counts", #{ "type": "invoke", "f": "get", "value": [key] })
    sim.record("counts", #{ "type": "ok", "value": ask("http://counter:7500/get?key=" + key, ()) })
}

for name in ["a", "b", "c"] {
    w.client(name, () => client(name))
}

// (the same partitions as in counter_world.ol)

let run = w.run(#{ "for": sim.hours(1), "props": [
    sim.linearizable("every add counts once", "counts", sim.kv())
] })
println(run.outcome)
```

```bash
$ olang sim counter_check.ol --seeds 16 --jobs 4
seed 2: "every add counts once" broke at 30m 10.722s (step 28542): key c: no order of the 819 operations explains the history: after 817 placed, c/4 add(["c", 1]) → 819 completed before it could take effect
seed 4: "every add counts once" broke at 30m 05.992s (step 28664): key c: no order of the 807 operations explains the history: after 805 placed, c/4 add(["c", 1]) → 807 completed before it could take effect
...
olang sim: 16 seed(s) · 8 passed · 8 failed · 5h 45m simulated per second (5.75 virtual hours) · 4 job(s) · 1.399 s real
digest: d9a0809e98e4c9da
reproduce: olang sim counter_check.ol --seed 2
```

The message says the checker placed 817 of the 819 operations on key
`c` in some order, and then found an add answered with 819 when, at
that point, only 818 adds could have happened. Pointed at
`counter_ids.ol`, a hundred seeds pass. The check splits a key-value
history by key, so an hour of three clients is checked in
milliseconds.

`sim.register(init)` is a model of a single value that is read,
written and compared-and-set. Any other model is a map with an `init`
state and a `step` function from a state and an operation to
`Ok(next state)` or `Err(why)`. `sim.linearize` runs the checker on any
history, inside a world or not:

```olang
// p1 writes 1 while p2 reads twice: first 1, then 0.
let history = [
    #{ "process": "p1", "type": "invoke", "f": "write", "value": 1 },
    #{ "process": "p2", "type": "invoke", "f": "read" },
    #{ "process": "p2", "type": "ok", "value": 1 },
    #{ "process": "p2", "type": "invoke", "f": "read" },
    #{ "process": "p2", "type": "ok", "value": 0 },
    #{ "process": "p1", "type": "ok" }
]
println(sim.linearize(history, sim.register(0)).ok)                       // false
println(sim.linearize(take(history, 3) + [history[5]], sim.register(0)).ok)  // true
```

The first history is impossible: once a read has seen the new value, a
later read cannot see the old one. Without the second read it is fine,
because the write may have taken effect at any moment while it was in
progress.

## Many seeds, shrinking and replaying

`--jobs J` runs seeds in J worker processes side by side. Each seed is
still one world on one thread, so the results do not depend on J. The
summary ends with a digest of every event of every seed, and it is the
same for one job as for the four used above:

```bash
$ olang sim counter_check.ol --seeds 16
...
olang sim: 16 seed(s) · 8 passed · 8 failed · 3h 43m simulated per second (3.72 virtual hours) · 1 job(s) · 2.161 s real
digest: d9a0809e98e4c9da
reproduce: olang sim counter_check.ol --seed 2
```

A failing run usually carries more than it needs: faults that played
no part, nodes that only added noise, and a schedule full of switches
between tasks. `--shrink` takes the first failing seed and makes it as
small as it can. To see it work, take `store_check.ol` and change two
things, saving the result as `crash_check.ol`. First, give it one
property, `sim.always("an acknowledged put survives a crash", ...)`,
which reads `s.disk("store").current(path)` (what the store would read
now, synced or not) where `durable` read `.read(path)`. A crash takes
away what was never synced, so this property breaks at the crash that
loses a put. Second, add some noise that has nothing to do with the
store:

```olang no-run
fn scribble() = {
    let mut n = 0
    while time.monotonic_ms() < sim.minutes(5) {
        n = n + 1
        let wrote = fs.append_file("/scratch", to_string(n) + "\n")
        time.sleep(random.randint(100, 400))
    }
}
w.client("spare", scribble, #{ "crash": #{ "p": 0.01, "restart_after": 300 }, "optional": true })
w.every(sim.seconds(7), () => {
    w.partition(["spare"], ["x", "y", "z"])
    time.sleep(1000)
    w.heal()
})
```

The spare node crashes at random and is cut off from the writers every
seven seconds. `"optional": true` tells the shrinker it may leave the
node out. Seed 6 is the interesting one, since its first crash lost
nothing:

```bash
$ olang sim crash_check.ol --seed 6 --shrink --jobs 4
...
shrinking seed 6 (4 job(s), at most 300 runs)...
shrunk: 53 faults → 1 · 24 runs · preemptions 1899 → 0 (schedule stick 1) · left out: spare
      3m 15.308s  crash      store    store (w.crash, restart after 0 ms): 3 unsynced write(s) lost
  then: "an acknowledged put survives a crash" broke at 3m 15.308s (step 17627)
plan: .sim/6.simplan   (olang sim crash_check.ol --replay .sim/6.simplan)
```

Of 53 faults, one matters: the power cut at 3m 15.308s. The shrinker
also left the spare node out, shortened the restart delay to nothing,
and found that the failure does not depend on the order of the tasks
(it happens even when a task keeps running until it has to wait,
which is what `stick 1` means). Every step is a whole run of the
world, and a change is kept only if the run still fails the same way.

The result is saved as a plan, a small JSON file that names the seed
and every fault the run should make. `--replay` runs it again and
checks it reproduced exactly:

```bash
$ olang sim --replay .sim/6.simplan
...
olang sim: seed 6: "an acknowledged put survives a crash" broke at 3m 15.308s (step 17627)
olang sim: replayed .sim/6.simplan bit for bit (digest 5025667c758bb849, 1 fault(s))
```

If the program has changed since, the replay says so and exits with
status 3, as in `the replay diverged from .sim/8.simplan: digest
2219d15872f62dea, recorded e6e77af3466f91a8`. `olang sim shrink
.sim/6.simplan` shrinks a saved plan further, with a fresh budget.

"Fails the same way" means the same property broke or, without
properties, the same error raised at the same place in the code (file,
line and column), with the values in its message set aside: numbers,
text in double quotes, and whatever a pair of brackets holds. The
writers in `store_world.ol` end with `key + " lost acknowledged puts "
+ to_string(lost)`, and the list in it has a different length in
almost every run; to the shrinker these are all `w# lost acknowledged
puts […]` at one line, one failure however many puts a run lost. A value written without quotes or brackets
(`"answer was " + word`) still counts as part of the wording, so put
values that vary in brackets, or use a property, whose name never
changes.

Shrinking works on faults, nodes, restart delays and how often the
schedule switches tasks, not on single messages. The seed's choices are
tied to where and when they are made, not drawn one after another: a
message's latency and loss come from its link and the millisecond it
was sent, a scheduling choice from its instant. So removing a fault
moves nothing else at instants it never reached, and a partition that
cut another client does not stay just because removing it reshuffled
every later message. A fault that really does change the timing of
what follows (an earlier cut that delays the failing client's next
request) stays, because without it the failure does not happen.

## Seeing a run

A failing seed is a story told in events: who sent what to whom, which
answer never arrived, when the power went. `--events` prints that story
as text, which is fine for twenty lines and hopeless for twenty
thousand. There are two better ways to read a long one.

The first is to treat the run as data. `run.events()` gives every event
as a table (an `ods` Frame), one row per event in the order they
happened, so the questions you would ask of a log take a line each.
Add these lines to the end of `counter_world.ol`:

```olang no-run
let f = run.events()
println(f.group_by("kind").agg(#{ "n": ods.count() }))   // what happened, and how often
println(ods.filter(f, f["fault"]))                       // every fault the world caused
let arrivals = ods.filter(f, f["kind"] == "deliver")
println(ods.filter(arrivals, arrivals["t"] - arrivals["sent"] > 190))   // the slow messages
```

On seed 2, the faults are the six partitions, one every five minutes:

```text
┌───────┬─────────┬────────┬──────┬───────────┬─────────────────┬───
│   seq │       t │ node   │ task │ kind      │ detail          │ …
├───────┼─────────┼────────┼──────┼───────────┼─────────────────┼───
│  4559 │  300000 │ hook-1 │    5 │ partition │ [counter] | [a] │ …
│  8967 │  600000 │ hook-1 │    5 │ partition │ [counter] | [c] │ …
│ 13467 │  900000 │ hook-1 │    5 │ partition │ [counter] | [a] │ …
…
```

Each row has the event's time `t` in virtual milliseconds, its `node`
and `task`, its `kind` and `detail`. A message's arrival also carries
who sent it (`peer`) and when (`sent`), so a message's latency is a
subtraction rather than a parsing job. `fault` marks what the world did
on purpose. (`run.events`, without the call, is the same events as a
list of maps.)

The second way is to look at it. `olang sim view` draws a run as a
timeline in your browser, with one lane per node:

```bash
olang sim view counter_world.ol --seed 2         # run seed 2 and draw it
olang sim counter_check.ol --seed 8 --shrink     # saves .sim/8.simplan
olang sim view .sim/8.simplan                    # the shrunk run, exactly as it failed
```

Messages are arrows from the sender's lane to the receiver's, and a
lost one ends in a cross. Faults are red marks. A node that is down is
a band across its lane, and a partition shades the lanes it cut for as
long as it lasted. Drag along the time axis, or the strip at the
bottom, to move the playhead: everything after it fades, so what you
see is the world at that instant. Click an event to see it in full,
with what its node did just before and after. `⇧→` jumps to the next
fault, `F` fits the whole run on screen, and `/` filters.

The page is a single file that fetches nothing, so you can attach it to
a bug report, or have CI write one for each failing seed with
`--html FILE`. `--json FILE` writes the same timeline as data, for your
own tools. Because a saved plan replays bit for bit, the picture is the
run that failed, not one like it; if the program has changed since the
plan was saved, `olang sim view` says so.

## Buggify points

Some code paths are rare enough that even a simulated world seldom
takes them: an answer slow enough to time out, a retry, a cache miss.
`sim.maybe(name)` marks such a place. Outside a world it is always
`false`, so it costs nothing in production. Inside one, the seed turns
each named point on or off for the whole run (about a quarter are on),
and a point that is on answers `true` about a quarter of the times it
is reached. Across many seeds, a point reached once answers `true` in
about one run in sixteen, and a point reached three times in about one
run in seven: most runs never take the rare path, and the ones that do
take it often. Each point (on each node) draws on its own, so adding a
point, or reaching another one more often, changes no other point's
answers.

Here is the first counter with one line added to its `/add` handler,
before it answers:

```olang no-run
let total = unwrap(chan.ask(inbox, #{ "op": "add", "key": body.key }))
if sim.maybe("slow_answer") => time.sleep(1500) else => ()
http.response(200, to_string(total))
```

A slow answer arrives after the client's one-second timeout, so the
client retries an add the server has already counted. That is the
same double count as before, now found without any partitions at all:

```bash
$ olang sim quiet_world.ol --seeds 12 --jobs 4
seed 1: failed at 30m 00.684s virtual: client a: the server counted 884, the client added 678
seed 7: failed at 30m 00.827s virtual: client c: the server counted 897, the client added 647
seed 11: failed at 30m 00.261s virtual: client a: the server counted 880, the client added 654
olang sim: 12 seed(s) · 9 passed · 3 failed · 8h 58m simulated per second (8.98 virtual hours) · 4 job(s) · 0.669 s real
digest: a8e6f930db9ea124
reproduce: olang sim quiet_world.ol --seed 1
```

Each time a point answers `true` it is recorded as a fault, so the
shrinker can remove the ones that did not matter.

## Simulation in `olang test`

A test can build and run a world like any other code. `w.run_fn(f)`
runs a function as a world's main program and answers the run:

```olang no-run
test "two withdrawals leave the right balance" {
    let run = sim.world(#{}).run_fn(two_withdrawals)
    assert_eq(run.value, 20, "balance after withdrawing 30 and 50 from 100")
}
```

Here `two_withdrawals` is the bank program from the start of the
chapter, wrapped in a function that answers the final balance. A test
block that runs a world is run once for each of three seeds, and every
world in it takes that seed. Three seeds are not enough for this bug,
so the test passes, until the search is widened with `--seeds`:

```bash
$ olang test bank_test.ol
bank_test.ol
  ✓ two withdrawals leave the right balance
────────────────────────────────────────
  1 passed, 0 failed  (1 test file, 2ms)
$ olang test bank_test.ol --seeds 20
bank_test.ol
  ✗ two withdrawals leave the right balance
      sim seed 17: Runtime error: balance after withdrawing 30 and 50 from 100 (reproduce: its worlds with seed 17; olang test --seeds N widens the search)
────────────────────────────────────────
  0 passed, 1 failed  (1 test file, 5ms)
```

It is the same seed 17 that `olang sim` found, since the seed decides
the run however it is started. `sim.seed()` answers the seed a test is
running under.

## What a world simulates, and what it refuses

A program can only reach the outside world through a known set of
standard library calls, the same set that [record and
replay](tooling.md#olang-record-and-olang-replay--the-open-timeline)
and the capability checks rely on. Inside a world, each of them is
either answered by the world or refused:

| What | Inside a world |
|---|---|
| Time: `time.now_ms`, `time.sleep`, `dates.now` and the rest | a virtual clock that starts at 2026-01-01 (or `start`); local time is UTC; a node's `skew` shifts its wall clock |
| Randomness: `random`, `crypto.random_bytes` | streams split from the seed, one per task (the world's own choices, of latency, loss, scheduling and disk faults, are tied to where and when they are made) |
| Tasks: `spawn`, `task`, `chan` | one task at a time, chosen by the seed; `par_map` runs in order |
| Network: `http` | messages between the world's nodes, with latency, loss, duplication and partitions |
| Files and SQLite: `fs`, `db` | each node's own disk, in memory |
| The process: `os.args`, `os.get_env`, `os.read_line`, `os.exit`, `os.on_shutdown` | each node's own arguments, environment, standard input, exit and handlers |
| Everything else with an effect: `os.exec`, `proc`, `pty`, `tty`, `gui`, `runtime`, `fs.watch`, `fs.trash`, and the `crypto` calls that need the operating system's randomness (`crypto.ed25519_keypair` among them) | refused |

A refused call is an error, and the real call is never made:

```text
sim: proc.spawn is refused inside a world: processes is not simulated yet (a world simulates time, randomness, spawn, task, chan, http between its nodes, each node's files and SQLite on its own disk, and its environment, stdin and exit), and a world never reaches the real one
```

This is what makes a world safe to run anywhere. Nothing in it can
reach the real disk, network or processes, so a seed that crashes a
node a thousand times does no harm, and a run comes out the same on
any machine. A database opened inside a world, for instance, lives on
the world's disk and nowhere else:

```olang
let run = sim.world(#{ "seed": 1 }).run_fn(() => {
    let conn = unwrap(db.open("notes.db"))
    unwrap(db.execute(conn, "create table notes (body text)", []))
    unwrap(db.execute(conn, "insert into notes values (?)", ["hello"]))
    time.sleep(sim.hours(6))
    unwrap(db.query(conn, "select body from notes", []))
})
println(run.value)                // [#{"body": "hello"}]
println(run.now)                  // 21600015
println(fs.exists("notes.db"))    // false
```

`run.now` is the virtual time at which the run ended, in milliseconds:
the six-hour sleep, which took no real time, and 15 ms that the
database spent syncing its writes to the simulated disk.

A few limits are worth knowing. Worlds run on the interpreter, not
the faster engines, so a heavy application simulates more slowly than
the small examples here. A program that waits by reading the clock in
a loop never sees it move, because only waiting lets time pass. A
node's `use` still loads code from the real files, while its `fs`
calls see only its own disk. And `print` inside a world also reaches
the real standard output, which `--seeds` keeps quiet.

[The standard library reference](stdlib.md#sim--simulated-worlds) lists
every function in `sim`, and [the tooling
chapter](tooling.md#olang-sim) lists every flag of `olang sim`.
