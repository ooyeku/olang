// gallery — what the data stack can draw.
//
// Every still piece is plot SVG: data computed in olang (math, random,
// stats), rendered as text, landed with one set_html. The centerpiece
// is live: a draw-list rebuilt every frame. As the viz stack grows,
// new capabilities land here first.

use viz

fn o(title) = #{ "theme": "dark", "responsive": true, "title": title }
fn spec(s, title) =
    map_set(map_set(map_set(s, "title", title), "theme", "dark"), "responsive", true)
fn card(id, svg) = dom.set_html(dom.query("#" + id), svg)
fn jitter(s) = (random.random() + random.random() + random.random() - 1.5) * s

// ── damped oscillations: three springs released together ─────────────
let t = map(range(0, 240), (i) => to_float(i) * 0.05)
let xs = ods.series(t)
fn wave(freq, decay, phase) = ods.series(map(t, (x) =>
    math.sin(x * freq + phase) * math.exp(0.0 - x * decay)))
card("g-osc", plot.lines(xs, [
    ["w=1.0", wave(1.0, 0.10, 0.0)],
    ["w=1.7", wave(1.7, 0.16, 1.1)],
    ["w=2.6", wave(2.6, 0.24, 2.2)]
], o("damped oscillations")))

// ── layered waveform: three harmonics summed under an area fill ──────
card("g-area", plot.area(xs, ods.series(map(t, (x) =>
    2.2 + math.sin(x) + 0.6 * math.sin(x * 2.7 + 0.8) + 0.35 * math.sin(x * 5.3 + 2.0))),
    o("layered waveform")))

// ── the galaxy: 12,500 stars, spinning on one parameter ──────────────
// Star positions are computed ONCE (closed-form arms + a core bulge);
// every frame re-sends the same three buffers with a new rotation —
// dom.draw_points applies the spin host-side, so olang's per-frame
// work is three calls, whatever the star count.
fn arm_x(n, phase) = ods.series(map(range(0, n), (i) => {
    let f = to_float(i) / to_float(n)
    let a = f * 5.2 + phase
    (0.08 + f * 0.92) * math.cos(a) + jitter(0.03 + 0.09 * f)
}))
fn arm_y(n, phase) = ods.series(map(range(0, n), (i) => {
    let f = to_float(i) / to_float(n)
    let a = f * 5.2 + phase
    (0.08 + f * 0.92) * math.sin(a) + jitter(0.03 + 0.09 * f)
}))
let a1x = arm_x(5000, 0.0)
let a1y = arm_y(5000, 0.0)
let a2x = arm_x(5000, 3.14159)
let a2y = arm_y(5000, 3.14159)
let core_x = ods.series(map(range(0, 2500), (i) => jitter(0.16)))
let core_y = ods.series(map(range(0, 2500), (i) => jitter(0.13)))
let gal = dom.query("#g-galaxy")
dom.on_frame((f) => {
    let rot = time.monotonic_ms() / 11000.0
    dom.draw(gal, [#{ "op": "clear", "color": "#0b0e14" }])
    dom.draw_points(gal, a1x, a1y, #{ "mode": "points", "size": 1.0, "alpha": 0.75,
        "color": "#5aa9e6", "sx": 185.0, "sy": 185.0, "tx": 260.0, "ty": 190.0, "rot": rot })
    dom.draw_points(gal, a2x, a2y, #{ "mode": "points", "size": 1.0, "alpha": 0.75,
        "color": "#3ddc97", "sx": 185.0, "sy": 185.0, "tx": 260.0, "ty": 190.0, "rot": rot })
    dom.draw_points(gal, core_x, core_y, #{ "mode": "points", "size": 1.5, "alpha": 0.9,
        "color": "#f5e9c9", "sx": 185.0, "sy": 185.0, "tx": 260.0, "ty": 190.0,
        "rot": rot * 1.4 })
})

// ── interference field: two wave systems crossing, as a heatmap ──────
let g = range(0, 26)
card("g-field", plot.heatmap(
    map(g, (c) => show(c)),
    map(g, (r) => show(r)),
    map(g, (r) => map(g, (c) =>
        math.sin(to_float(c) * 0.30) * math.cos(to_float(r) * 0.30) +
        0.5 * math.sin((to_float(c) + to_float(r)) * 0.19))),
    o("interference field")))

// ── revenue mix: growth curves stacked into a part-of-whole story ────
let quarters = map(range(1, 9), (q) => "Q" + show(q))
fn seg(base, growth, wobble, phase) = ods.series(map(range(0, 8), (q) =>
    base + growth * to_float(q) + wobble * math.sin(to_float(q) * 0.9 + phase)))
card("g-stack", plot.stacked(quarters, [
    ["platform", seg(40.0, 6.0, 3.0, 0.3)],
    ["services", seg(28.0, 3.5, 4.0, 1.4)],
    ["licenses", seg(18.0, 1.2, 2.5, 2.6)],
    ["other", seg(8.0, 0.4, 1.5, 4.0)]
], o("revenue mix by quarter")))

// ── distribution zoo: four shapes, one five-number summary each ──────
card("g-box", plot.box([
    ["tight", stats.norm.sample(240, 0.0, 0.6)],
    ["wide", stats.norm.sample(240, 0.0, 1.8)],
    ["shifted", stats.norm.sample(240, 2.5, 1.0)],
    ["skewed", ods.series(map(range(0, 240), (i) => math.exp(jitter(0.9))))]
], o("distribution zoo")))

// ── the central limit theorem, watched happening ─────────────────────
card("g-clt", plot.hist(ods.series(map(range(0, 3000), (i) =>
    random.random() + random.random() + random.random() + random.random())),
    36, o("central limit: sum of 4 uniforms")))

// ── ensemble forecast: layered marks, brushable, hoverable ───────────
let forecast = map(range(0, 30), (d) => {
    let x = to_float(d)
    let mid = 12.0 + 6.0 * math.sin(x * 0.35) + x * 0.15
    #{ "day": d, "hi": mid + 2.5 + jitter(0.4), "mid": mid, "obs": mid + jitter(1.6) }
})
fn draw_forecast(lo, hi) = {
    let window = filter(forecast, (r) =>
        map_get(r, "day") >= lo && map_get(r, "day") <= hi)
    let title = if lo == 0 && hi == 29 => "ensemble forecast (brush to zoom)"
        else => "ensemble forecast · days " + show(to_int(lo)) + "–" + show(to_int(hi)) + " (dblclick resets)"
    card("g-layers", viz.chart(map_set(spec(#{ "data": window, "layers": [
        #{ "mark": "area", "x": "day", "y": "hi", "label": "envelope" },
        #{ "mark": "line", "x": "day", "y": "mid", "label": "forecast" },
        #{ "mark": "point", "x": "day", "y": "obs", "label": "observed" }
    ] }, title), "interactive", true)))
}
draw_forecast(0, 29)
viz.brush(dom.query("#g-layers"), (b) => {
    let lo = math.round(map_get(b, "from") * 29.0)
    let hi = math.round(map_get(b, "to") * 29.0)
    if hi > lo + 1.0 => draw_forecast(lo, hi)
})
dom.on(dom.query("#g-layers"), "dblclick", (e) => { draw_forecast(0, 29) })
viz.tooltip(dom.query("#g-layers"))

// ── three species: one scatter spec, color does the splitting ────────
fn cluster(n, mx, my, sd, name) = map(range(0, n), (i) =>
    #{ "x": mx + jitter(sd), "y": my + jitter(sd), "species": name })
let blobs = concat(concat(
    cluster(90, 2.0, 3.0, 0.9, "adelie"),
    cluster(90, 5.5, 5.5, 1.1, "gentoo")),
    cluster(90, 4.0, 1.5, 0.8, "chinstrap"))
card("g-species", viz.chart(map_set(spec(#{ "data": blobs, "mark": "point",
    "x": "x", "y": "y", "color": "species" }, "clusters (hover a point)"),
    "interactive", true)))
viz.tooltip(dom.query("#g-species"))

// ── a strange attractor: the same grammar, compiled to canvas ────────
// 4,000 points would drown a DOM in SVG nodes; viz.draw takes the
// identical spec shape and emits one draw-list instead.
let a = 0.0 - 2.0
let b = 0.0 - 2.0
let c = 0.0 - 1.2
let d = 2.0
let mut ax = 0.1
let mut ay = 0.1
let mut pts = []
let mut n = 0
while n < 4000 {
    let nx = math.sin(a * ay) - math.cos(b * ax)
    let ny = math.sin(c * ax) - math.cos(d * ay)
    ax = nx
    ay = ny
    pts = pts + [#{ "x": ax, "y": ay }]
    n = n + 1
}
viz.draw(dom.query("#g-attractor"),
    #{ "data": pts, "mark": "point", "x": "x", "y": "y" })

// ── thirty thousand points at frame rate: the binary bulk path ───────
// The curtain is computed ONCE as two Series; every frame re-sends the
// same packed f64 buffer with a new affine + color. No JSON, no
// per-point olang work — dom.draw_points is one memcpy and one native
// loop per frame.
let curtain_t = map(range(0, 50000), (i) => to_float(i) * 0.00126)
let curtain_x = ods.series(map(curtain_t, (t) => math.sin(t * 3.01)))
let curtain_y = ods.series(map(curtain_t, (t) => math.sin(t * 3.97 + 1.57)))
let big = dom.query("#g-big")
let big_hud = dom.query("#g-big-hud")
dom.on_frame((f) => {
    let tt = time.monotonic_ms() / 1000.0
    dom.draw(big, [#{ "op": "clear", "color": "rgba(11,14,20,0.5)" }])
    dom.draw_points(big, curtain_x, curtain_y, #{
        "mode": "points", "size": 1.0, "alpha": 0.6,
        "color": "hsl(" + show(165.0 + 45.0 * math.sin(tt * 0.4)) + " 70% 62%)",
        "sx": 500.0 * (1.0 + 0.1 * math.sin(tt * 0.6)),
        "sy": 165.0 * (1.0 + 0.1 * math.cos(tt * 0.8)),
        "tx": 560.0, "ty": 190.0 })
    dom.set_text(big_hud, "50,000 points · " +
        show(to_int(math.round(1000.0 / math.max(map_get(f, "delta"), 1.0)))) + " fps")
})

// ── the live piece: rose curves morphing, one draw-list per frame ────
let stage = dom.query("#g-live")
let cx = 560.0
let cy = 190.0
fn rose(k, radius, phase) = map(range(0, 241), (i) => {
    let th = to_float(i) * 0.0261799
    let r = radius * math.sin(th * k + phase)
    [cx + r * math.cos(th), cy + r * math.sin(th)]
})
dom.on_frame((f) => {
    let tt = time.monotonic_ms() / 1000.0
    let k = 3.0 + 1.5 * math.sin(tt * 0.23)
    dom.draw(stage, [
        #{ "op": "clear", "color": "rgba(11,14,20,0.16)" },
        #{ "op": "path", "points": rose(k, 170.0, tt * 0.4),
           "close": false, "stroke": "#7fd1b9", "line_width": 1.5 },
        #{ "op": "path", "points": rose(k * 1.5, 120.0, 0.0 - tt * 0.3),
           "close": false, "stroke": "#5aa9e6", "line_width": 1 },
        #{ "op": "path", "points": rose(k * 0.5, 80.0, tt * 0.6),
           "close": false, "stroke": "#f5c542", "line_width": 1 }
    ])
})
