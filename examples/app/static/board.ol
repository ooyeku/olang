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
    map_set(map_set(map_set(map_set(s, "title", title),
        "theme", "dark"), "responsive", true), "interactive", true)

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
        dash.kpi(if sel == "" => "issues" else => "issues · " + sel,
            show(len(picked)), show(len(items)) + " in tracker"),
        dash.kpi("open", show(open_n), pct(open_n, len(items)) + " of all"),
        dash.kpi("done", show(done_n), pct(done_n, len(items)) + " complete"),
        dash.kpi("points", show(total_pts),
            if len(picked) == 0 => "—"
            else => "avg " + show(to_int(math.round(to_float(total_pts) / to_float(len(picked)))))),
        dash.card("by status", "<div id=\"b-status\"></div>"),
        dash.card("priority mix", "<div id=\"b-mix\"></div>"),
        dash.card("points by status", "<div id=\"b-box\"></div>"),
        dash.card("points spread", "<div id=\"b-hist\"></div>"),
        dash.wide("cumulative points", "<div id=\"b-cum\"></div>")
    ], 4))

    if len(picked) > 0 => {
        dom.set_html(dom.query("#b-status"), viz.chart(themed(#{ "data": rows,
            "mark": "bar", "x": "status", "y": "one" }, "")))
        dom.set_html(dom.query("#b-mix"), viz.chart(themed(#{ "data": rows,
            "mark": "bar", "x": "status", "y": "one", "color": "priority",
            "stack": true }, "")))
        dom.set_html(dom.query("#b-box"), viz.chart(themed(#{ "data": rows,
            "mark": "box", "x": "status", "y": "points" }, "")))
        dom.set_html(dom.query("#b-hist"), viz.chart(themed(#{ "data": rows,
            "mark": "hist", "y": "points", "bins": 8 }, "")))
        let mut total = 0
        let mut n = 1
        let mut cum = []
        for i in picked {
            total = total + map_get(i, "points")
            cum = cum + [#{ "n": n, "total": total }]
            n = n + 1
        }
        dom.set_html(dom.query("#b-cum"), viz.chart(themed(#{ "data": cum, "layers": [
            #{ "mark": "area", "x": "n", "y": "total", "label": "" },
            #{ "mark": "point", "x": "n", "y": "total", "label": "" }
        ], "height": 260 }, "")))
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
