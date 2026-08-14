// board — the ops dashboard: the whole viz stack in one page.
//
// dash builds the shell (KPI tiles, cards, the grid — pure HTML from
// an embedded olang package), viz fills the cards from the same
// records, the status filter lives in the URL (bookmarkable, back
// button correct), and the board re-fetches itself every five
// seconds. State lives in the DOM and the URL; the program is a pure
// pipeline from records to pixels.

use viz
use dash

let mount = dom.query("#board")

fn themed(s, title) =
    map_set(map_set(map_set(map_set(map_set(map_set(s, "title", title),
        "theme", "dark"), "responsive", true), "interactive", true),
        "width", 560), "height", 340)

fn sel_status() = {
    let q = map_get(dom.location(), "query")
    if map_has_key(q, "status") => map_get(q, "status") else => ""
}

fn pct(a, b) = if b == 0 => "0%"
    else => show(to_int(math.round(to_float(a) * 100.0 / to_float(b)))) + "%"

fn render(items) = {
    let sel = sel_status()
    let picked = if sel == "" => items
        else => filter(items, (i) => map_get(i, "status") == sel)
    let rows = map(picked, (i) => map_set(i, "one", 1))
    let points = map(picked, (i) => map_get(i, "points"))
    let open_n = len(filter(items, (i) => map_get(i, "status") == "open"))
    let done_n = len(filter(items, (i) => map_get(i, "status") == "done"))
    let total_pts = sum(points)

    dom.set_html(mount, dash.styles() + dash.grid([
        dash.stat(if sel == "" => "issues" else => "issues · " + sel,
            show(len(picked)), show(len(items)) + " in tracker", "#3ddc97"),
        dash.stat("open", show(open_n), pct(open_n, len(items)) + " of all", "#5aa9e6"),
        dash.stat("done", show(done_n), pct(done_n, len(items)) + " complete", "#8b7ae0"),
        dash.stat("points", show(total_pts),
            if len(picked) == 0 => "—"
            else => "avg " + show(to_int(math.round(to_float(total_pts) / to_float(len(picked))))),
            "#f4b84c"),
        dash.half("by status", "<div id=\"b-status\"></div>"),
        dash.half("priority mix", "<div id=\"b-mix\"></div>"),
        dash.half("points by status", "<div id=\"b-box\"></div>"),
        dash.half("points spread", "<div id=\"b-hist\"></div>"),
        dash.wide("cumulative points", "<div id=\"b-cum\"></div>")
    ], 4))

    if len(picked) > 0 => {
        dom.set_html(dom.query("#b-status"), viz.chart(themed(#{ "data": rows,
            "mark": "bar", "x": "status", "y": "one", "vary": true }, "")))
        dom.set_html(dom.query("#b-mix"), viz.chart(themed(#{ "data": rows,
            "mark": "bar", "x": "status", "y": "one", "color": "priority",
            "stack": true }, "")))
        dom.set_html(dom.query("#b-box"), viz.chart(themed(#{ "data": rows,
            "mark": "box", "x": "status", "y": "points", "colors": ["#f4b84c"] }, "")))
        dom.set_html(dom.query("#b-hist"), viz.chart(themed(#{ "data": rows,
            "mark": "hist", "y": "points", "bins": 8, "colors": ["#5aa9e6"] }, "")))
        let mut total = 0
        let mut n = 1
        let mut cum = []
        for i in picked {
            total = total + map_get(i, "points")
            cum = cum + [#{ "n": n, "total": total }]
            n = n + 1
        }
        dom.set_html(dom.query("#b-cum"), viz.chart(map_set(map_set(
            themed(#{ "data": cum, "colors": ["#4dd0e1", "#f4b84c"], "layers": [
                #{ "mark": "area", "x": "n", "y": "total", "label": "" },
                #{ "mark": "point", "x": "n", "y": "total", "label": "" }
            ] }, ""), "width", 1140), "height", 300)))
    }
}

fn refresh() = dom.fetch_json("GET", "/api/issues?limit=500", "", (resp) => {
    // A fetch failure delivers #{ "error": ... } — keep the last render.
    if map_has_key(resp, "items") => render(map_get(resp, "items"))
})

// The filter writes the URL; the URL drives the render. Back and
// forward replay filters for free.
dom.on(dom.query("#b-flt"), "change", (e) => {
    let v = map_get(e, "value")
    dom.push_state(if v == "" => "/board.html" else => "/board.html?status=" + v)
    refresh()
})
dom.on_route((r) => { refresh() })

// One delegated tooltip on the mount covers every chart the board
// will ever render into it.
viz.tooltip(mount)

dom.set_value(dom.query("#b-flt"), sel_status())
dom.set_interval(5000, () => { refresh() })
refresh()
