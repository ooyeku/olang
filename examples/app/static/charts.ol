// charts — the data stack meets the page.
//
// One fetch_json brings the tracker's issues in as parsed records;
// everything after that is the same ods + plot code that runs
// natively: series, frames, and SVG charts as text, landed with one
// dom.set_html per card. Dark theme and responsive sizing are plot
// options — no CSS surgery on the output.

fn o(title) = #{ "theme": "dark", "responsive": true, "title": title }

fn card(id, svg) = dom.set_html(dom.query("#" + id), svg)

let statuses = ["open", "in-progress", "done"]
let priorities = ["low", "medium", "high"]

fn count_where(items, st, pr) = len(filter(items, (i) =>
    (st == "" || map_get(i, "status") == st) &&
    (pr == "" || map_get(i, "priority") == pr)))

fn points_where(items, st) =
    map(filter(items, (i) => map_get(i, "status") == st), (i) => map_get(i, "points"))

fn draw(items) = {
    // Issues by status: one bar per category.
    card("c-status", plot.bar(statuses,
        ods.series(map(statuses, (s) => count_where(items, s, ""))),
        o("issues by status")))

    // The same categories split by priority — grouped and stacked are
    // the same pairs, one flag apart.
    let pairs = map(priorities, (p) =>
        [p, ods.series(map(statuses, (s) => count_where(items, s, p)))])
    card("c-grouped", plot.bars(statuses, pairs, o("priority per status")))
    card("c-stacked", plot.stacked(statuses, pairs, o("priority mix, stacked")))

    // Distributions: histogram over every issue, boxes per status.
    card("c-hist", plot.hist(ods.series(map(items, (i) => map_get(i, "points"))),
        8, o("points distribution")))
    let active = filter(statuses, (s) => count_where(items, s, "") > 0)
    card("c-box", plot.box(map(active, (s) => [s, ods.series(points_where(items, s))]),
        o("points by status")))

    // A matrix: count at every status × priority cell.
    card("c-heat", plot.heatmap(statuses, priorities,
        map(priorities, (p) => map(statuses, (s) => count_where(items, s, p))),
        o("count: status × priority")))

    // Cumulative points in id order — the area chart's home turf.
    let mut total = 0
    let mut cum = []
    for i in items {
        total = total + map_get(i, "points")
        cum = cum + [total]
    }
    card("c-area", plot.area(ods.series(1..(len(items) + 1)), ods.series(cum),
        o("cumulative points")))

    dom.set_text(dom.query("#hud"), show(len(items)) + " issues charted")
}

fn refresh() = dom.fetch_json("GET", "/api/issues?limit=500", "", (resp) => {
    let items = map_get(resp, "items")
    if len(items) == 0 => dom.set_text(dom.query("#hud"), "no issues to chart — add some in the tracker")
    else => draw(items)
})

dom.on(dom.query("#refresh"), "click", (e) => { refresh() })
refresh()
