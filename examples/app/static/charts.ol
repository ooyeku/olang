// charts — the data stack meets the page, now through the viz grammar.
//
// One fetch_json brings the tracker's issues in as parsed records, and
// records are what viz speaks: a chart is a spec map — data, a mark,
// and column-name encodings. No manual pivoting; `color` splits
// series, bar marks aggregate rows that share a category, and layers
// compose marks over shared scales. The heatmap stays on the direct
// plot API — both levels are public, use whichever fits.

use viz

fn card(id, svg) = dom.set_html(dom.query("#" + id), svg)

// Every chart on this page is dark, responsive, and titled.
fn themed(s, title) =
    map_set(map_set(map_set(s, "title", title), "theme", "dark"), "responsive", true)

let statuses = ["open", "in-progress", "done"]
let priorities = ["low", "medium", "high"]

fn count_where(items, st, pr) = len(filter(items, (i) =>
    map_get(i, "status") == st && map_get(i, "priority") == pr))

fn draw(items) = {
    // One derived column: every issue counts once. That is the whole
    // aggregation story — bar marks sum rows sharing a category.
    let rows = map(items, (i) => map_set(i, "one", 1))

    card("c-status", viz.chart(themed(#{ "data": rows, "mark": "bar",
        "x": "status", "y": "one" }, "issues by status")))

    card("c-grouped", viz.chart(themed(#{ "data": rows, "mark": "bar",
        "x": "status", "y": "one", "color": "priority" }, "priority per status")))

    card("c-stacked", viz.chart(themed(#{ "data": rows, "mark": "bar",
        "x": "status", "y": "one", "color": "priority", "stack": true },
        "priority mix, stacked")))

    card("c-hist", viz.chart(themed(#{ "data": rows, "mark": "hist",
        "y": "points", "bins": 8 }, "points distribution")))

    card("c-box", viz.chart(themed(#{ "data": rows, "mark": "box",
        "x": "status", "y": "points" }, "points by status")))

    // A matrix is not (yet) a grammar mark — the direct plot API is
    // right there for it.
    card("c-heat", plot.heatmap(statuses, priorities,
        map(priorities, (p) => map(statuses, (s) => count_where(items, s, p))),
        #{ "theme": "dark", "responsive": true, "title": "count: status × priority" }))

    // Cumulative points as a layered spec: the fill tells the trend,
    // the markers pin each issue.
    let mut total = 0
    let mut n = 1
    let mut cum = []
    for i in items {
        total = total + map_get(i, "points")
        cum = cum + [#{ "n": n, "total": total }]
        n = n + 1
    }
    card("c-area", viz.chart(themed(#{ "data": cum, "layers": [
        #{ "mark": "area", "x": "n", "y": "total" },
        #{ "mark": "point", "x": "n", "y": "total" }
    ] }, "cumulative points")))

    dom.set_text(dom.query("#hud"), show(len(items)) + " issues charted")
}

fn refresh() = dom.fetch_json("GET", "/api/issues?limit=500", "", (resp) => {
    let items = map_get(resp, "items")
    if len(items) == 0 => dom.set_text(dom.query("#hud"), "no issues to chart — add some in the tracker")
    else => draw(items)
})

dom.on(dom.query("#refresh"), "click", (e) => { refresh() })
refresh()
