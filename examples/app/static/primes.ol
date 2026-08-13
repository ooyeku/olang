// primes — browser parallelism from olang, out of the box.
//
// dom.worker boots a SECOND olang (its own wasm instance, its own
// thread) from /primes-worker.ol. Values cross as plain maps — send
// with dom.worker_send, receive with dom.worker_on. The dial below is
// the proof: it redraws every frame on the main thread while the
// worker counts primes flat out.

let bar = dom.query("#bar")
let status = dom.query("#status")
let out = dom.query("#out")
let btn = dom.query("#go")
let dial = dom.query("#dial")

let w = dom.worker("/primes-worker.ol")

dom.worker_on(w, (m) => {
    let kind = map_get(m, "kind")
    if kind == "ready" => dom.set_text(status, "worker ready")
    if kind == "progress" => {
        let pct = map_get(m, "done") * 100 / map_get(m, "upto")
        dom.set_style(bar, "width", show(pct) + "%")
        dom.set_text(status, show(pct) + "% · " + show(map_get(m, "count")) + " primes so far")
    }
    if kind == "done" => {
        dom.set_style(bar, "width", "100%")
        dom.set_text(status, "done")
        dom.set_text(out, show(map_get(m, "count")) + " primes below " + show(map_get(m, "upto")))
    }
})

dom.on(btn, "click", (e) => {
    dom.set_text(out, "")
    dom.set_style(bar, "width", "0%")
    dom.set_text(status, "counting…")
    dom.worker_send(w, #{ "upto": 200000 })
})

// The liveness dial: a hand sweeping once every ~2s, redrawn every
// frame by the MAIN thread. If workers blocked the page, it would
// freeze the moment counting starts.
let cx = 60.0
let cy = 60.0
dom.on_frame((f) => {
    let a = time.monotonic_ms() / 320.0
    let x = cx + 44.0 * math.cos(a)
    let y = cy + 44.0 * math.sin(a)
    dom.draw(dial, [
        #{ "op": "clear" },
        #{ "op": "circle", "x": cx, "y": cy, "r": 52.0,
           "stroke": "#1e2430", "line_width": 2 },
        #{ "op": "line", "x1": cx, "y1": cy, "x2": x, "y2": y,
           "stroke": "#7fd1b9", "line_width": 3 },
        #{ "op": "circle", "x": x, "y": y, "r": 5.0, "fill": "#7fd1b9" }
    ])
})
