// orbits — rich graphics from olang, out of the box.
//
// Every frame builds a draw-list (plain olang maps) and crosses the
// wasm boundary ONCE via dom.draw; the page replays it onto Canvas 2D.
// State lives in a hidden input as JSON (the session's handlers capture
// their environment by value, so the DOM is the store — the same
// pattern as the tracker). Click anywhere to add a body.

let sky = dom.query("#sky")
let hud = dom.query("#hud")
let state = dom.query("#state")

let cx = 360.0
let cy = 240.0

fn body(r, speed, size, hue, angle) = #{
    "r": r, "speed": speed, "size": size, "hue": hue, "angle": angle
}

fn seed_bodies() = [
    body(60.0, 1.4, 6.0, 30, 0.0),
    body(100.0, 1.0, 9.0, 160, 1.7),
    body(150.0, 0.7, 7.0, 200, 3.1),
    body(200.0, 0.45, 12.0, 280, 4.6),
    body(230.0, 0.3, 5.0, 340, 0.9)
]

fn save_bodies(bodies) = dom.set_value(state, unwrap(json.stringify(bodies)))
fn load_bodies() = unwrap_or(json.parse(dom.value(state)), seed_bodies())

fn step_body(b, dt) = body(
    map_get(b, "r"),
    map_get(b, "speed"),
    map_get(b, "size"),
    map_get(b, "hue"),
    map_get(b, "angle") + map_get(b, "speed") * dt
)

fn hsl(hue) = "hsl(" + show(hue) + " 80% 62%)"

fn body_ops(b) = {
    let a = map_get(b, "angle")
    let r = map_get(b, "r")
    let x = cx + r * math.cos(a)
    let y = cy + r * math.sin(a)
    let size = map_get(b, "size")
    let color = hsl(map_get(b, "hue"))
    [
        #{ "op": "circle", "x": cx, "y": cy, "r": r,
           "stroke": "rgba(154,164,178,0.12)", "line_width": 1 },
        #{ "op": "line", "x1": cx, "y1": cy, "x2": x, "y2": y,
           "stroke": "rgba(154,164,178,0.06)", "line_width": 1 },
        #{ "op": "circle", "x": x, "y": y, "r": size, "fill": color },
        #{ "op": "circle", "x": x, "y": y, "r": size + 3.0,
           "stroke": color, "line_width": 1 }
    ]
}

fn scene(bodies) = {
    let mut ops = [
        #{ "op": "clear", "color": "rgba(11,14,20,0.35)" },
        #{ "op": "circle", "x": cx, "y": cy, "r": 16.0, "fill": "#f5c542" },
        #{ "op": "circle", "x": cx, "y": cy, "r": 22.0,
           "stroke": "rgba(245,197,66,0.35)", "line_width": 2 }
    ]
    for b in bodies {
        ops = ops + body_ops(b)
    }
    ops
}

fn on_frame(f) = {
    let dt = map_get(f, "delta") / 1000.0
    let mut stepped = []
    for b in load_bodies() {
        stepped = stepped + [step_body(b, dt)]
    }
    save_bodies(stepped)
    dom.draw(sky, scene(stepped))
    dom.set_text(hud, show(len(stepped)) + " bodies")
}

fn on_click(e) = {
    let rect = dom.measure(sky)
    let px = map_get(e, "x") - map_get(rect, "x")
    let py = map_get(e, "y") - map_get(rect, "y")
    let dx = px - cx
    let dy = py - cy
    let r = math.sqrt(dx * dx + dy * dy)
    if r > 24.0 => {
        let hue = random.randint(0, 359)
        let speed = 0.3 + 90.0 / r
        let angle = math.atan2(dy, dx)
        save_bodies(load_bodies() + [body(r, speed, 4.0 + r / 40.0, hue, angle)])
    }
}

save_bodies(seed_bodies())
dom.on(sky, "click", (e) => { on_click(e) })
dom.on_frame((f) => { on_frame(f) })
