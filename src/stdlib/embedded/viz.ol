// viz — the declarative chart grammar, written in olang and bundled as
// a builtin package (`use viz`).
//
// A chart is a VALUE: a spec map holding data, encodings, and a mark.
// Data is records (a list of maps — exactly what dom.fetch_json or
// ods.to_records delivers) or a Frame; encodings name columns; `color`
// splits the rows into one series per distinct value. `viz.chart(spec)`
// compiles the spec to plot SVG — pure, testable anywhere. `viz.draw`
// compiles the same xy specs to a canvas draw-list instead, for data
// too big to render as SVG nodes.
//
//   viz.chart(#{
//       "data": rows, "mark": "line", "x": "day", "y": "value",
//       "color": "series", "title": "...", "theme": "dark"
//   })
//
// Layered charts list several xy marks over shared scales:
//
//   viz.chart(#{ "data": rows, "layers": [
//       #{ "mark": "area", "x": "t", "y": "lo" },
//       #{ "mark": "line", "x": "t", "y": "mid" },
//       #{ "mark": "point", "x": "t", "y": "obs" }
//   ]})
//
// Marks: line, area, scatter/point (layerable, color-splittable);
// bar (color splits into grouped bars, `"stack": true` stacks them;
// rows sharing a category sum); hist (`"bins"`); box (one box per
// distinct x value). Line marks connect points in row order — sort
// the records first when order matters.

// ── spec plumbing ──────────────────────────────────────────────────────

fn opt(spec, key, default) =
    if map_has_key(spec, key) => map_get(spec, key) else => default

fn records_of(data) =
    if typeof(data) == "List" => data else => ods.to_records(data)

fn to_label(v) = if typeof(v) == "String" => v else => show(v)

fn col(records, name) = map(records, (r) => map_get(r, name))

// Distinct values of a column, first-seen order.
fn groups(records, name) = {
    let mut seen = []
    for r in records {
        let v = map_get(r, name)
        if contains(seen, v) == false => {
            seen = seen + [v]
        }
    }
    seen
}

// The plot options that pass straight through from the spec. `w` and
// `h` are accepted as the short spellings of `width` and `height`.
fn plot_opts(spec) = {
    let mut o = #{}
    for k in ["title", "x_label", "y_label", "width", "height", "theme",
              "responsive", "interactive", "colors", "vary", "scale",
              "font_size", "font"] {
        if map_has_key(spec, k) => {
            o = map_set(o, k, map_get(spec, k))
        }
    }
    if map_has_key(spec, "w") && !map_has_key(spec, "width") => { o = map_set(o, "width", map_get(spec, "w")) }
    if map_has_key(spec, "h") && !map_has_key(spec, "height") => { o = map_set(o, "height", map_get(spec, "h")) }
    o
}

// Every key a spec (or one of its layers) may carry. An unknown key is
// an error, not a silent default: a spec written with `size` renders
// at the default size and nothing says why — the Open AST philosophy
// applied to chart specs.
let spec_keys = ["data", "layers", "mark", "x", "y", "color", "color_by",
                 "label", "scale", "stack", "bins", "title", "x_label",
                 "y_label", "width", "height", "w", "h", "theme",
                 "responsive", "interactive", "colors", "vary",
                 "font_size", "font"]

fn check_spec_keys(spec, where_) = {
    for k in map_keys(spec) {
        if contains(spec_keys, k) == false => {
            unwrap(Err("viz: unknown key '" + k + "' in " + where_
                + " — the spec keys are " + join(spec_keys, ", ")))
        }
    }
    if map_has_key(spec, "layers") => {
        for layer in map_get(spec, "layers") { check_spec_keys(layer, "a layer") }
    }
}

fn norm_mark(m) = if m == "point" => "scatter" else => m

// Distinct values of a plain list, first-seen order.
fn distinct(vals) = {
    let mut seen = []
    for v in vals {
        if contains(seen, v) == false => {
            seen = seen + [v]
        }
    }
    seen
}

// Continuous color encoding: map values onto a named ramp, quantized
// to 24 steps — smooth to the eye, cheap to bucket on canvas.
fn ramp_colors(vals, scale) = {
    let lo = to_float(min(vals))
    let hi = to_float(max(vals))
    let span = if hi > lo => hi - lo else => 1.0
    map(vals, (v) =>
        plot.ramp(scale, math.round((to_float(v) - lo) / span * 23.0) / 23.0))
}

// ── xy compilation (shared by chart and draw) ──────────────────────────
// An entry is [label, mark, xs (list), ys (list)].

// Entry columns are lists on the record path and Series on the
// columnar path — downstream code accepts either.
fn layer_entries(layer, fallback_data) = {
    let d = if map_has_key(layer, "data") => map_get(layer, "data")
        else => fallback_data
    let x = map_get(layer, "x")
    let y = map_get(layer, "y")
    let mark = norm_mark(opt(layer, "mark", "line"))
    if map_has_key(layer, "color") || typeof(d) == "List" => {
        let records = records_of(d)
        if map_has_key(layer, "color") => {
            let c = map_get(layer, "color")
            map(groups(records, c), (g) => {
                let rows = filter(records, (r) => map_get(r, c) == g)
                [to_label(g), mark, col(rows, x), col(rows, y)]
            })
        } else => {
            let base = [to_label(opt(layer, "label", y)), mark, col(records, x), col(records, y)]
            if map_has_key(layer, "color_by") => [base + [ramp_colors(
                col(records, map_get(layer, "color_by")), opt(layer, "scale", "thermal"))]]
            else => [base]
        }
    } else => {
        // A Frame with no color split: columns come out as Series —
        // no per-row conversion, the fast lane for large data.
        let base = [to_label(opt(layer, "label", y)), mark, ods.column(d, x), ods.column(d, y)]
        if map_has_key(layer, "color_by") => [base + [ramp_colors(
            ods.to_list(ods.column(d, map_get(layer, "color_by"))),
            opt(layer, "scale", "thermal"))]]
        else => [base]
    }
}

fn xy_entries(spec) = {
    let data = if map_has_key(spec, "data") => map_get(spec, "data")
        else => []
    let layers = if map_has_key(spec, "layers") => map_get(spec, "layers")
        else => [spec]
    let mut entries = []
    for layer in layers {
        entries = concat(entries, layer_entries(layer, data))
    }
    entries
}

fn as_series(v) = if typeof(v) == "List" => ods.series(v) else => v
fn as_list(v) = if typeof(v) == "List" => v else => ods.to_list(v)
fn vmin(v) = if typeof(v) == "List" => to_float(min(v)) else => ods.min(v)
fn vmax(v) = if typeof(v) == "List" => to_float(max(v)) else => ods.max(v)

// ── the SVG target ─────────────────────────────────────────────────────

share fn chart(spec) = {
    check_spec_keys(spec, "the chart spec")
    let mark = norm_mark(opt(spec, "mark", "line"))
    let mut o = plot_opts(spec)
    if map_has_key(spec, "layers") || mark == "line" || mark == "area" || mark == "scatter" => {
        plot.xy(map(xy_entries(spec), (en) =>
            if len(en) == 5 => [en[0], en[1], as_series(en[2]), as_series(en[3]), en[4]]
            else => [en[0], en[1], as_series(en[2]), as_series(en[3])]), o)
    } else => {
        let records = records_of(map_get(spec, "data"))
        if mark == "hist" => plot.hist(
            ods.series(col(records, map_get(spec, "y"))), opt(spec, "bins", 20), o)
        else => {
            let xcol = map_get(spec, "x")
            let cats = groups(records, xcol)
            let labels = map(cats, (v) => to_label(v))
            if mark == "box" => plot.box(map(cats, (cat) =>
                [to_label(cat), ods.series(col(filter(records, (r) =>
                    map_get(r, xcol) == cat), map_get(spec, "y")))]), o)
            else => {
                // bar: rows sharing a category sum; color splits series.
                let ycol = map_get(spec, "y")
                if map_has_key(spec, "color") => {
                    let ccol = map_get(spec, "color")
                    let pairs = map(groups(records, ccol), (g) => [to_label(g),
                        ods.series(map(cats, (cat) =>
                            sum(col(filter(records, (r) =>
                                map_get(r, xcol) == cat && map_get(r, ccol) == g), ycol))))])
                    if opt(spec, "stack", false) => plot.stacked(labels, pairs, o)
                    else => plot.bars(labels, pairs, o)
                } else => plot.bar(labels,
                    ods.series(map(cats, (cat) =>
                        sum(col(filter(records, (r) => map_get(r, xcol) == cat), ycol)))), o)
            }
        }
    }
}

// ── the canvas target ──────────────────────────────────────────────────
// The same xy specs, compiled to a draw-list: for point counts where
// SVG nodes would drown the DOM, or for redrawing every frame.

let palette = ["#3ddc97", "#5aa9e6", "#f4b84c", "#f0854a",
               "#ef8bb0", "#8b7ae0", "#58c458", "#ef6b73"]

// Axis labels round to two decimals — full float precision on a
// canvas label is noise.
fn fmt(v) = show(math.round(to_float(v) * 100.0) / 100.0)

share fn draw(el, spec) = {
    let w = to_float(unwrap(str.parse_int(dom.get_attr(el, "width"))))
    let h = to_float(unwrap(str.parse_int(dom.get_attr(el, "height"))))
    let mut entries = xy_entries(spec)
    let x0 = min(map(entries, (en) => vmin(en[2])))
    let x1 = max(map(entries, (en) => vmax(en[2])))
    let y0 = min(map(entries, (en) => vmin(en[3])))
    let y1 = max(map(entries, (en) => vmax(en[3])))
    let xspan = if x1 > x0 => x1 - x0 else => 1.0
    let yspan = if y1 > y0 => y1 - y0 else => 1.0
    let left = 48.0
    let top = 14.0
    let pw = w - left - 14.0
    let ph = h - top - 30.0
    let px = (v) => left + (to_float(v) - x0) / xspan * pw
    let py = (v) => top + ph - (to_float(v) - y0) / yspan * ph

    let mut ops = [
        #{ "op": "clear", "color": "#101720" },
        #{ "op": "line", "x1": left, "y1": top, "x2": left, "y2": top + ph,
           "stroke": "#2b3b4f", "line_width": 1 },
        #{ "op": "line", "x1": left, "y1": top + ph, "x2": left + pw, "y2": top + ph,
           "stroke": "#2b3b4f", "line_width": 1 },
        #{ "op": "text", "x": left - 6.0, "y": top + 10.0, "align": "right",
           "text": fmt(y1), "fill": "#7f93a3", "font": "12px monospace" },
        #{ "op": "text", "x": left - 6.0, "y": top + ph, "align": "right",
           "text": fmt(y0), "fill": "#7f93a3", "font": "12px monospace" },
        #{ "op": "text", "x": left, "y": top + ph + 16.0, "align": "left",
           "text": fmt(x0), "fill": "#7f93a3", "font": "12px monospace" },
        #{ "op": "text", "x": left + pw, "y": top + ph + 16.0, "align": "right",
           "text": fmt(x1), "fill": "#7f93a3", "font": "12px monospace" }
    ]
    let mut gi = 1
    while gi <= 3 {
        let gy = top + ph * to_float(gi) / 4.0
        ops = ops + [#{ "op": "line", "x1": left, "y1": gy, "x2": left + pw, "y2": gy,
                        "stroke": "#1d2937", "line_width": 1 }]
        gi = gi + 1
    }

    // Areas still ride the draw-list (they need a closed filled
    // path); everything else goes point-first below.
    let mut i = 0
    for en in entries {
        if en[1] == "area" => {
            let xs = as_list(en[2])
            let ys = as_list(en[3])
            let pts = map(range(0, len(xs)), (j) => [px(xs[j]), py(ys[j])])
            let closed = concat(pts, [[px(xs[len(xs) - 1]), top + ph],
                                      [px(xs[0]), top + ph]])
            ops = ops + [#{ "op": "path", "points": closed, "close": true,
                            "fill": "rgba(61,220,151,0.16)" }]
        }
        i = i + 1
    }
    dom.draw(el, ops)

    // Marks cross as ONE packed f64 buffer per series — dom.draw_points
    // applies the data→pixel affine host-side, so the olang side does
    // no per-point work at all. This is what makes 50,000-point
    // scatters redraw at frame rate.
    let sx = pw / xspan
    let sy = 0.0 - ph / yspan
    let tx = left - x0 * sx
    let ty = top + ph + y0 * (ph / yspan)
    let mut j = 0
    for en in entries {
        let color = palette[j % len(palette)]
        if len(en) == 5 => {
            // Continuous color on canvas: the 24 quantized ramp steps
            // become at most 24 bulk calls — still the binary path.
            let cs = en[4]
            let xs = as_list(en[2])
            let ys = as_list(en[3])
            for c in distinct(cs) {
                let idx = filter(range(0, len(cs)), (i) => cs[i] == c)
                dom.draw_points(el, map(idx, (i) => xs[i]), map(idx, (i) => ys[i]),
                    #{ "mode": "points", "size": 2.0, "color": c,
                       "sx": sx, "sy": sy, "tx": tx, "ty": ty })
            }
        } else => {
            if en[1] == "scatter" => {
                dom.draw_points(el, en[2], en[3], #{ "mode": "points", "size": 2.5,
                    "color": color, "sx": sx, "sy": sy, "tx": tx, "ty": ty })
            } else => {
                dom.draw_points(el, en[2], en[3], #{ "mode": "path", "size": 1.5,
                    "color": color, "sx": sx, "sy": sy, "tx": tx, "ty": ty })
            }
        }
        j = j + 1
    }
}

// ── interactivity ──────────────────────────────────────────────────────
// Marks rendered with `"interactive": true` carry their datum as
// data-* attributes, and every dom event delivers the target's data
// map — so hovering and clicking marks is ordinary event delegation.

// True when an event's data map came from a chart mark.
fn is_mark(d) =
    map_has_key(d, "x") || map_has_key(d, "value") || map_has_key(d, "med")

// One line per mark family; values arrive as attribute strings.
fn tip_text(d) = {
    let prefix = if map_has_key(d, "s") && map_get(d, "s") != "" => map_get(d, "s") + " · " else => ""
    if map_has_key(d, "med") => prefix + "median " + map_get(d, "med")
            + "  [" + map_get(d, "q1") + " – " + map_get(d, "q3") + "]"
    else => {
        if map_has_key(d, "xl") => map_get(d, "xl") + " × " + map_get(d, "yl") + ": " + map_get(d, "value")
        else => {
            if map_has_key(d, "value") => prefix + map_get(d, "label") + ": " + map_get(d, "value")
            else => prefix + "(" + map_get(d, "x") + ", " + map_get(d, "y") + ")"
        }
    }
}

// Attach a hover tooltip to a chart container: hovering any
// interactive mark shows its datum, leaving hides it.
share fn tooltip(el) = {
    let tip = dom.create("div")
    for (k, v) in entries(#{
        "position": "fixed", "display": "none", "pointer-events": "none",
        "background": "var(--surface, #131c27)", "border": "1px solid var(--line, #1d2937)",
        "border-radius": "6px", "padding": "3px 8px", "font": "12px var(--mono, monospace)",
        "color": "var(--ink, #d9e6ef)", "z-index": "50", "box-shadow": "var(--shadow, none)"
    }) {
        dom.set_style(tip, k, v)
    }
    dom.append(dom.query("body"), tip)
    dom.on(el, "pointermove", (e) => {
        let d = map_get(e, "data")
        if is_mark(d) => {
            dom.set_text(tip, tip_text(d))
            dom.set_style(tip, "left", show(map_get(e, "x") + 14) + "px")
            dom.set_style(tip, "top", show(map_get(e, "y") + 12) + "px")
            dom.set_style(tip, "display", "block")
        } else => dom.set_style(tip, "display", "none")
    })
    dom.on(el, "pointerleave", (e) => {
        dom.set_style(tip, "display", "none")
    })
}

// Delegated mark events: the handler fires only when the event target
// was an interactive mark, and receives the mark's data map.
share fn on_mark(el, event, handler) =
    dom.on(el, event, (e) => {
        let d = map_get(e, "data")
        if is_mark(d) => handler(d)
    })

// Horizontal brush: press, drag, release inside `el`; the handler
// receives #{ "from", "to" } as fractions of the element's width
// (0.0 at the left edge, 1.0 at the right). Taps shorter than 2% of
// the width are ignored — they are clicks, not brushes.
share fn brush(el, handler) = {
    let anchor = dom.create("input")
    dom.set_attr(anchor, "type", "hidden")
    dom.append(dom.query("body"), anchor)
    dom.on(el, "pointerdown", (e) => {
        dom.set_value(anchor, show(x_frac(el, e)))
    })
    dom.on(el, "pointerup", (e) => {
        let start = dom.value(anchor)
        if start != "" => {
            dom.set_value(anchor, "")
            let a = unwrap(str.parse_float(start))
            let b = x_frac(el, e)
            if math.abs(b - a) > 0.02 => {
                handler(#{ "from": math.min(a, b), "to": math.max(a, b) })
            }
        }
    })
}

fn x_frac(el, e) = {
    let r = dom.measure(el)
    let f = (to_float(map_get(e, "x")) - map_get(r, "x")) / map_get(r, "width")
    math.max(0.0, math.min(1.0, f))
}
