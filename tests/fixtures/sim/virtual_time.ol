// An hour of sleeping, in milliseconds of real time: a ticker sleeps a
// minute sixty times while the main program sleeps the hour.
let start = time.now_ms()
let ticks = chan.new()
let ticker = spawn {
    for i in range(0, 60) {
        time.sleep(60000)
        chan.send(ticks, i)
    }
    chan.close(ticks)
}
time.sleep(3600000)
let mut n = 0
let mut going = true
while going {
    match chan.recv(ticks) {
        Ok(_) => { n = n + 1 }
        Err(_) => { going = false }
    }
}
println(to_string(n) + " ticks in " + to_string(time.now_ms() - start) + " ms")
