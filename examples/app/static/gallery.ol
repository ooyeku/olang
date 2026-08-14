// gallery — what the data stack can draw.
//
// Every still piece is plot SVG: data computed in olang (math, random,
// stats), rendered as text, landed with one set_html. The centerpiece
// is live: a draw-list rebuilt every frame. As the viz stack grows,
// new capabilities land here first.

fn o(title) = #{ "theme": "dark", "responsive": true, "title": title }
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

// ── spiral galaxy: two noisy arms, brightness by scatter density ─────
fn arm(n, phase) = map(range(0, n), (i) => {
    let a = to_float(i) * 0.03
    let r = 0.34 * a + 0.12
    [r * math.cos(a * 3.2 + phase) + jitter(0.10 * r + 0.02),
     r * math.sin(a * 3.2 + phase) + jitter(0.10 * r + 0.02)]
})
let stars = concat(arm(260, 0.0), arm(260, 3.14159))
card("g-spiral", plot.scatter(
    ods.series(map(stars, (p) => p[0])),
    ods.series(map(stars, (p) => p[1])),
    o("spiral galaxy")))

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
