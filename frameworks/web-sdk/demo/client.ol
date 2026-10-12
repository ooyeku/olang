// shipit's browser half — served bundled, one file to the browser.
// The chart is the embedded `viz` module compiling a spec to SVG,
// inline in the view tree: full-stack olang, charts included.
use web { mount, action, action_arg, apply, input_value, call,
          err_details, field, form_fields,
          stack, row, spread, card, muted, badge_tone, stat,
          icon_btn, tabs, list_card, list_row, btn, btn_primary,
          btn_danger, topbar, div, span, h2, raw, text }
use viz

let task_fields = [
    field("title", "Title", "text", #{}),
    field("priority", "Priority", #{ "select": ["ship", "polish", "docs"] }, #{}),
    field("points", "Points", "number", #{})
]

fn priority_tone(p) =
    if p == "ship" => "accent" else if p == "polish" => "blue" else => "amber"

fn task_row(t) = {
    let done = t.done == 1
    let id = to_string(t.id)
    list_row([
        icon_btn(if done => "✓" else => "", "toggle:" + id, done),
        span(#{ "class": if done => "grow muted strike" else => "grow" },
            [t.title]),
        badge_tone(t.priority, priority_tone(t.priority)),
        muted(to_string(t.points) + " pt"),
        btn_danger("×", "del:" + id)
    ])
}

fn tab(label_text, name, active) =
    if active => btn_primary(label_text, "filter:" + name)
    else => btn(label_text, "filter:" + name)

fn visible(s) = {
    let f = s.filter
    filter(s.tasks, (t) =>
        if f == "open" => t.done == 0
        else if f == "done" => t.done == 1
        else => true)
}

fn burn_chart(tasks) = {
    if len(tasks) == 0 => text("")
    else => {
        let by = map(tasks, (t) => #{
            "priority": t.priority,
            "points": t.points,
            "state": if t.done == 1 => "shipped" else => "open"
        })
        card([stack([
            h2(#{}, ["points by priority"]),
            raw(viz.chart(#{ "data": by, "mark": "bar", "x": "priority",
                "y": "points", "color": "state", "stack": true,
                "colors": ["#2e6e4e", "#b9beb6"], "w": 640, "h": 200 }))
        ])])
    }
}

fn stats_strip(s) = {
    let st = s.stats
    if st == () => text("")
    else => row([
        stat(to_string(st.open), "open"),
        stat(to_string(st.done), "shipped"),
        stat(to_string(st.open_pts), "pts in flight")
    ])
}

fn view(s) = stack([
    topbar("shipit", [stats_strip(s)]),
    div(#{ "class": "card form-inline" }, [stack([
        row([
            div(#{ "class": "grow", "style": "flex: 1" }, [
                form_fields(task_fields, #{}, s.errors)
            ]),
            btn_primary("Add", "add")
        ])
    ])]),
    spread(
        tabs([
            tab("All", "all", s.filter == "all"),
            tab("Open", "open", s.filter == "open"),
            tab("Shipped", "done", s.filter == "done")
        ]),
        muted(to_string(len(visible(s))) + " of " +
            to_string(len(s.tasks)) + " tasks")
    ),
    list_card(map(visible(s), (t) => task_row(t))),
    burn_chart(s.tasks)
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
