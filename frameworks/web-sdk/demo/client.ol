// shipit's browser half — served bundled, one file to the browser.
// The chart is the embedded `viz` module compiling a spec to SVG,
// inline in the view tree: full-stack olang, charts included.
use web { mount, action, action_arg, apply, input_value, call,
          err_details, field, form_fields,
          stack, row, spread, card, muted, badge, btn, btn_primary,
          btn_danger, topbar, div, span, h2, raw, text }
use viz

let task_fields = [
    field("title", "Title", "text", #{}),
    field("priority", "Priority", #{ "select": ["ship", "polish", "docs"] }, #{}),
    field("points", "Points", "number", #{})
]

fn priority_badge(p) = badge(p, p == "ship")

fn task_card(t) = {
    let done = map_get(t, "done") == 1
    let id = to_string(map_get(t, "id"))
    card([spread(
        row([
            btn(if done => "↺" else => "✓", "toggle:" + id),
            span(#{ "class": if done => "muted strike" else => "" },
                [map_get(t, "title")]),
            priority_badge(map_get(t, "priority")),
            muted(to_string(map_get(t, "points")) + " pt")
        ]),
        btn_danger("×", "del:" + id)
    )])
}

fn tab(label_text, name, active) =
    if active => btn_primary(label_text, "filter:" + name)
    else => btn(label_text, "filter:" + name)

fn visible(s) = {
    let f = map_get(s, "filter")
    filter(map_get(s, "tasks"), (t) =>
        if f == "open" => map_get(t, "done") == 0
        else if f == "done" => map_get(t, "done") == 1
        else => true)
}

fn burn_chart(tasks) = {
    if len(tasks) == 0 => text("")
    else => {
        let by = map(tasks, (t) => #{
            "priority": map_get(t, "priority"),
            "points": map_get(t, "points"),
            "state": if map_get(t, "done") == 1 => "done" else => "open"
        })
        card([
            h2(#{}, ["points by priority"]),
            raw(viz.chart(#{ "data": by, "mark": "bar", "x": "priority",
                "y": "points", "color": "state", "stack": true,
                "colors": ["#2f6f4f", "#9aa0a6"], "w": 640, "h": 220 }))
        ])
    }
}

fn stats_strip(s) = {
    let st = map_get(s, "stats")
    if st == () => text("")
    else => row([
        badge(to_string(map_get(st, "open")) + " open", true),
        badge(to_string(map_get(st, "done")) + " shipped", false),
        muted(to_string(map_get(st, "open_pts")) + " pts in flight, "
            + to_string(map_get(st, "done_pts")) + " landed")
    ])
}

fn view(s) = stack([
    topbar("⚡ shipit", [stats_strip(s)]),
    card([
        form_fields(task_fields, #{}, map_get(s, "errors")),
        div(#{ "class": "row" }, [btn_primary("Add task", "add")])
    ]),
    row([
        tab("All", "all", map_get(s, "filter") == "all"),
        tab("Open", "open", map_get(s, "filter") == "open"),
        tab("Shipped", "done", map_get(s, "filter") == "done")
    ]),
    stack(map(visible(s), (t) => task_card(t))),
    burn_chart(map_get(s, "tasks")),
    muted(to_string(len(visible(s))) + " of "
        + to_string(len(map_get(s, "tasks"))) + " tasks shown")
])

fn refresh() = {
    call("tasks.list", #{}, (r) => {
        match r {
            Ok(tasks) => { apply((s) => map_set(s, "tasks", tasks)) },
            Err(e) => ()
        }
    })
    call("tasks.stats", #{}, (r) => {
        match r {
            Ok(st) => { apply((s) => map_set(s, "stats", st)) },
            Err(e) => ()
        }
    })
}

action("add", (ev) =>
    call("tasks.create", #{
        "title": input_value("title"),
        "priority": input_value("priority"),
        "points": input_value("points")
    }, (r) => {
        match r {
            Ok(t) => {
                let cleared = apply((s) => map_set(s, "errors", []))
                refresh()
            },
            Err(e) => { apply((s) => map_set(s, "errors", err_details(e))) }
        }
    }))

action("toggle", (ev) =>
    call("tasks.toggle", #{ "id": unwrap(str.parse_int(action_arg(ev))) },
        (r) => refresh()))

action("del", (ev) =>
    call("tasks.delete", #{ "id": unwrap(str.parse_int(action_arg(ev))) },
        (r) => refresh()))

action("filter", (ev) => {
    let f = action_arg(ev)
    let n = apply((s) => map_set(s, "filter", f))
})

mount("#app", view, #{ "tasks": [], "stats": (), "filter": "all", "errors": [] })
refresh()
