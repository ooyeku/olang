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

// The plot options that pass straight through from the spec.
fn plot_opts(spec) = {
    let mut o = #{}
    for k in ["title", "x_label", "y_label", "width", "height", "theme", "responsive"] {
        if map_has_key(spec, k) => {
            o = map_set(o, k, map_get(spec, k))
        }
    }
    o
}

fn norm_mark(m) = if m == "point" => "scatter" else => m

// ── xy compilation (shared by chart and draw) ──────────────────────────
// An entry is [label, mark, xs (list), ys (list)].

fn layer_entries(layer, fallback_records) = {
    let records = if map_has_key(layer, "data") => records_of(map_get(layer, "data"))
        else => fallback_records
    let x = map_get(layer, "x")
    let y = map_get(layer, "y")
    let mark = norm_mark(opt(layer, "mark", "line"))
    if map_has_key(layer, "color") => {
        let c = map_get(layer, "color")
        map(groups(records, c), (g) => {
            let rows = filter(records, (r) => map_get(r, c) == g)
            [to_label(g), mark, col(rows, x), col(rows, y)]
        })
    } else => [[to_label(opt(layer, "label", y)), mark, col(records, x), col(records, y)]]
}

fn xy_entries(spec) = {
    let records = if map_has_key(spec, "data") => records_of(map_get(spec, "data"))
        else => []
    let layers = if map_has_key(spec, "layers") => map_get(spec, "layers")
        else => [spec]
    let mut entries = []
    for layer in layers {
        entries = concat(entries, layer_entries(layer, records))
    }
    entries
}

// ── the SVG target ─────────────────────────────────────────────────────

share fn chart(spec) = {
    let mark = norm_mark(opt(spec, "mark", "line"))
    let o = plot_opts(spec)
    if map_has_key(spec, "layers") || mark == "line" || mark == "area" || mark == "scatter" => {
        plot.xy(map(xy_entries(spec), (en) =>
            [en[0], en[1], ods.series(en[2]), ods.series(en[3])]), o)
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

let palette = ["#5aa9e6", "#f0854a", "#3ddc97", "#f5c542",
               "#ef8bb0", "#58c458", "#8b7ae0", "#ef6b73"]

fn lo_of(vals) = min(vals)
fn hi_of(vals) = max(vals)

// Axis labels round to two decimals — full float precision on a
// canvas label is noise.
fn fmt(v) = show(math.round(to_float(v) * 100.0) / 100.0)

share fn draw(el, spec) = {
    let w = to_float(unwrap(str.parse_int(dom.get_attr(el, "width"))))
    let h = to_float(unwrap(str.parse_int(dom.get_attr(el, "height"))))
    let entries = xy_entries(spec)
    let mut all_x = []
    let mut all_y = []
    for en in entries {
        all_x = concat(all_x, en[2])
        all_y = concat(all_y, en[3])
    }
    let x0 = to_float(lo_of(all_x))
    let x1 = to_float(hi_of(all_x))
    let y0 = to_float(lo_of(all_y))
    let y1 = to_float(hi_of(all_y))
    let xspan = if x1 > x0 => x1 - x0 else => 1.0
    let yspan = if y1 > y0 => y1 - y0 else => 1.0
    let left = 48.0
    let top = 14.0
    let pw = w - left - 14.0
    let ph = h - top - 30.0
    let px = (v) => left + (to_float(v) - x0) / xspan * pw
    let py = (v) => top + ph - (to_float(v) - y0) / yspan * ph

    let mut ops = [
        #{ "op": "clear", "color": "#0b0e14" },
        #{ "op": "line", "x1": left, "y1": top, "x2": left, "y2": top + ph,
           "stroke": "#2a3444", "line_width": 1 },
        #{ "op": "line", "x1": left, "y1": top + ph, "x2": left + pw, "y2": top + ph,
           "stroke": "#2a3444", "line_width": 1 },
        #{ "op": "text", "x": left - 6.0, "y": top + 10.0, "align": "right",
           "text": fmt(y1), "fill": "#9aa4b2", "font": "11px monospace" },
        #{ "op": "text", "x": left - 6.0, "y": top + ph, "align": "right",
           "text": fmt(y0), "fill": "#9aa4b2", "font": "11px monospace" },
        #{ "op": "text", "x": left, "y": top + ph + 16.0, "align": "left",
           "text": fmt(x0), "fill": "#9aa4b2", "font": "11px monospace" },
        #{ "op": "text", "x": left + pw, "y": top + ph + 16.0, "align": "right",
           "text": fmt(x1), "fill": "#9aa4b2", "font": "11px monospace" }
    ]
    let mut gi = 1
    while gi <= 3 {
        let gy = top + ph * to_float(gi) / 4.0
        ops = ops + [#{ "op": "line", "x1": left, "y1": gy, "x2": left + pw, "y2": gy,
                        "stroke": "#1c2430", "line_width": 1 }]
        gi = gi + 1
    }

    let mut i = 0
    for en in entries {
        let color = palette[i % len(palette)]
        let mark = en[1]
        let pts = map(range(0, len(en[2])), (j) => [px(en[2][j]), py(en[3][j])])
        if mark == "scatter" => {
            for p in pts {
                ops = ops + [#{ "op": "circle", "x": p[0], "y": p[1], "r": 2.0,
                                "fill": color }]
            }
        } else => {
            if mark == "area" => {
                let closed = concat(pts, [[px(en[2][len(en[2]) - 1]), top + ph],
                                          [px(en[2][0]), top + ph]])
                ops = ops + [#{ "op": "path", "points": closed, "close": true,
                                "fill": "rgba(90,169,230,0.18)" }]
            }
            ops = ops + [#{ "op": "path", "points": pts, "close": false,
                            "stroke": color, "line_width": 1.5 }]
        }
        i = i + 1
    }
    dom.draw(el, ops)
}
