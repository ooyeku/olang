// charts — the data stack meets the page, now through the viz grammar.
//
// One fetch_json brings the tracker's issues in as parsed records, and
// records are what viz speaks: a chart is a spec map — data, a mark,
// and column-name encodings. No manual pivoting; `color` splits
// series, bar marks aggregate rows that share a category, and layers
// compose marks over shared scales. The heatmap stays on the direct
// plot API — both levels are public, use whichever fits.
//
// Interactivity is event delegation: marks render with their datum as
// data-* attributes, viz.tooltip shows it on hover, and clicking a
// status bar cross-filters every other card (click again to clear —
// the selection lives in a hidden input, DOM-resident like all state).

use viz

fn card(id, svg) = dom.set_html(dom.query("#" + id), svg)

// Every chart on this page is dark, responsive, and titled.
fn themed(s, title) =
    map_set(map_set(map_set(map_set(map_set(map_set(s, "title", title),
        "theme", "dark"), "responsive", true), "interactive", true),
        "width", 560), "height", 360)

let statuses = ["open", "in-progress", "done"]
let priorities = ["low", "medium", "high"]

fn count_where(items, st, pr) = len(filter(items, (i) =>
    map_get(i, "status") == st && map_get(i, "priority") == pr))

fn draw(items) = {
    // The cross-filter: a clicked status narrows every card but the
    // status bars themselves (so the selection stays clickable).
    // The cross-filter selection lives in session state (dom.state),
    // not a hidden input — the pattern named. Missing reads as "".
    let saved = dom.state_get("filter")
    let sel = if typeof(saved) == "String" => saved else => ""
    let picked = if sel == "" => items
        else => filter(items, (i) => map_get(i, "status") == sel)

    // One derived column: every issue counts once. That is the whole
    // aggregation story — bar marks sum rows sharing a category.
    let all_rows = map(items, (i) => map_set(i, "one", 1))
    let rows = map(picked, (i) => map_set(i, "one", 1))

    card("c-status", viz.chart(themed(#{ "data": all_rows, "mark": "bar",
        "x": "status", "y": "one", "vary": true },
        if sel == "" => "issues by status (click a bar to filter)"
        else => "issues by status · filtering: " + sel + " (click again to clear)")))

    card("c-grouped", viz.chart(themed(#{ "data": rows, "mark": "bar",
        "x": "status", "y": "one", "color": "priority" }, "priority per status")))

    card("c-stacked", viz.chart(themed(#{ "data": rows, "mark": "bar",
        "x": "status", "y": "one", "color": "priority", "stack": true },
        "priority mix, stacked")))

    card("c-hist", viz.chart(themed(#{ "data": rows, "mark": "hist",
        "y": "points", "bins": 8, "colors": ["#5aa9e6"] }, "points distribution")))

    card("c-box", viz.chart(themed(#{ "data": rows, "mark": "box",
        "x": "status", "y": "points", "colors": ["#8b7ae0"] }, "points by status")))

    // A matrix is not (yet) a grammar mark — the direct plot API is
    // right there for it.
    card("c-heat", plot.heatmap(statuses, priorities,
        map(priorities, (p) => map(statuses, (s) => count_where(picked, s, p))),
        #{ "theme": "dark", "responsive": true, "interactive": true, "scale": "ocean",
           "width": 560, "height": 360, "title": "count: status × priority" }))

    // Cumulative points as a layered spec: the fill tells the trend,
    // the markers pin each issue.
    let mut total = 0
    let mut n = 1
    let mut cum = []
    for i in picked {
        total = total + map_get(i, "points")
        cum = cum + [#{ "n": n, "total": total }]
        n = n + 1
    }
    card("c-area", viz.chart(map_set(map_set(themed(#{ "data": cum, "colors": ["#4dd0e1", "#f4b84c"], "layers": [
        #{ "mark": "area", "x": "n", "y": "total", "label": "" },
        #{ "mark": "point", "x": "n", "y": "total", "label": "" }
    ] }, "cumulative points"), "width", 1140), "height", 320)))

    dom.set_text(dom.query("#hud"),
        if sel == "" => show(len(items)) + " issues charted"
        else => show(len(picked)) + " of " + show(len(items)) + " issues · status = " + sel)
}

fn refresh() = dom.fetch_json("GET", "/api/issues?limit=500", "", (resp) => {
    // A fetch failure delivers #{ "error": ... } — keep the last render.
    if map_has_key(resp, "items") => {
        let items = map_get(resp, "items")
        if len(items) == 0 => dom.set_text(dom.query("#hud"), "no issues to chart — add some in the tracker")
        else => draw(items)
    }
})

dom.on(dom.query("#refresh"), "click", (e) => { refresh() })

// Hover any mark for its datum; click a status bar to cross-filter.
for id in ["c-status", "c-grouped", "c-stacked", "c-hist", "c-box", "c-heat", "c-area"] {
    viz.tooltip(dom.query("#" + id))
}
viz.on_mark(dom.query("#c-status"), "click", (d) => {
    let hit = map_get(d, "label")
    let saved = dom.state_get("filter")
    let cur = if typeof(saved) == "String" => saved else => ""
    dom.state_set("filter", if cur == hit => "" else => hit)
    refresh()
})
refresh()
