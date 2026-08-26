// app.ol — the tracker frontend, in olang, running as WebAssembly.
//
// Architecture, unchanged in spirit from v1 but grown into a real app:
//
//   * Stateless logic. The server is the source of truth for data; the
//     little client state that exists (sort column/order, status filter,
//     the selected issue) lives IN THE DOM as hidden inputs — never in
//     olang variables. Every render derives everything fresh.
//   * Event delegation. A fixed set of listeners is bound once at boot
//     to elements that exist at boot; rows re-render as pure HTML through
//     one set_html, so the handler registry never changes size. Clicks
//     deliver the target's id, changes deliver "id\nvalue".
//   * The backend API does the heavy lifting: filtering, search, and
//     sorting are ?query parameters; comments are endpoints; the stats
//     strip renders the /api/stats payload whose quantiles the server
//     computes on the ods data stack.

// ── escaping ───────────────────────────────────────────────────────────

fn esc(s) = show(s)
    |> str.replace("&", "&amp;")
    |> str.replace("<", "&lt;")
    |> str.replace(">", "&gt;")
    |> str.replace("\"", "&quot;")

fn json_esc(s) = show(s)
    |> str.replace("\\", "\\\\")
    |> str.replace("\"", "\\\"")
    |> str.replace("\n", "\\n")
    |> str.replace("\t", "\\t")

// ── DOM-resident client state ──────────────────────────────────────────

fn st(name) = dom.value(dom.query("#st-" + name))
fn set_st(name, v) = dom.set_value(dom.query("#st-" + name), v)

fn issues_url() = {
    let q = str.trim(dom.value(dom.query("#search")))
    "/api/issues?limit=200&sort=" + st("sort") + "&order=" + st("order")
        + (if st("status") != "" => "&status=" + st("status") else => "")
        + (if q != "" => "&q=" + q else => "")
}

// ── rendering: rows ────────────────────────────────────────────────────

fn pill(kind, value, id_prefix, id) =
    "<button class=\"pill " + kind + "-" + value + "\" id=\"" + id_prefix + id + "\">"
        + esc(value) + "</button>"

fn row_html(issue) = {
    let id = show(map_get(issue, "id"))
    let status = map_get(issue, "status")
    let selected = if st("sel") == id => " selected" else => ""
    "<tr class=\"status-" + status + selected + "\">"
        + "<td class=\"id\">" + id + "</td>"
        + "<td class=\"title-cell\" id=\"open-" + id + "\">" + esc(map_get(issue, "title")) + "</td>"
        + "<td>" + pill("st", status, "adv-", id) + "</td>"
        + "<td>" + pill("pr", map_get(issue, "priority"), "pri-", id) + "</td>"
        + "<td><input class=\"cell\" id=\"asg-" + id + "\" value=\""
        + esc(map_get(issue, "assignee")) + "\"></td>"
        + "<td class=\"num\"><input class=\"cell num\" id=\"pts-" + id + "\" value=\""
        + show(map_get(issue, "points")) + "\"></td>"
        + "<td class=\"del\"><button id=\"del-" + id + "\" title=\"delete\">×</button></td>"
        + "</tr>"
}

fn update_footer(items, total) = {
    let by = (s) => len(items |> filter((i) => map_get(i, "status") == s))
    dom.set_text(dom.query("#count"), show(total) + " issues")
    dom.set_text(dom.query("#summary"),
        show(by("open")) + " open · " + show(by("in-progress"))
        + " in progress · " + show(by("done")) + " done")
}

fn render_rows(items, total) = {
    dom.set_html(dom.query("#rows"), items |> map(row_html) |> join(""))
    dom.set_class(dom.query("#empty"), if len(items) == 0 => "show" else => "")
    update_footer(items, total)
}

// ── rendering: stats (ods computes the median, in the browser) ────────

fn stat_card(n, label, tone) =
    "<div class=\"stat " + tone + "\"><div class=\"n\">" + n
        + "</div><div class=\"l\">" + label + "</div></div>"

fn status_count(rows, name) = {
    let hit = rows |> filter((r) => map_get(r, "status") == name)
    if len(hit) == 0 => 0 else => map_get(hit[0], "n")
}

fn render_stats() = {
    dom.fetch("GET", "/api/stats", "", (resp) => {
        let s = unwrap(json.parse(resp))
        let by = map_get(s, "by_status")
        let totals = map_get(s, "totals")
        // point_quantiles is computed server-side on the ods data stack.
        let median = show(to_int(map_get(map_get(s, "point_quantiles"), "p50")))
        dom.set_html(dom.query("#stats"),
            stat_card(show(map_get(totals, "issues")), "issues", "")
            + stat_card(show(status_count(by, "open")), "open", "blue")
            + stat_card(show(status_count(by, "in-progress")), "in progress", "amber")
            + stat_card(show(status_count(by, "done")), "done", "mint")
            + stat_card(show(map_get(totals, "points")), "total points", "")
            + stat_card(median, "median points · ods", "mint"))
    })
}

// ── rendering: the activity ticker ─────────────────────────────────────

fn event_html(ev) =
    "<span class=\"ev\"><span class=\"k\">" + esc(map_get(ev, "action"))
        + "</span> <b>#" + show(map_get(ev, "issue_id")) + "</b> "
        + esc(map_get(ev, "detail")) + "</span>"

fn render_activity() = {
    dom.fetch("GET", "/api/activity?limit=8", "", (resp) => {
        let events = unwrap(json.parse(resp))
        dom.set_html(dom.query("#activity"), events |> map(event_html) |> join(""))
    })
}

// ── rendering: the drawer ──────────────────────────────────────────────

fn comment_html(c) =
    "<div class=\"comment\"><div class=\"who\">" + esc(map_get(c, "author"))
        + " · " + esc(map_get(c, "at")) + "</div>" + esc(map_get(c, "text")) + "</div>"

fn render_drawer(issue) = {
    let id = show(map_get(issue, "id"))
    dom.set_text(dom.query("#drawer-id"), "issue #" + id)
    dom.set_value(dom.query("#dtitle"), map_get(issue, "title"))
    dom.set_html(dom.query("#dmeta"),
        pill("st", map_get(issue, "status"), "dadv-", id)
        + pill("pr", map_get(issue, "priority"), "dpri-", id)
        + "<span>" + esc(map_get(issue, "assignee")) + "</span>"
        + "<span>" + show(map_get(issue, "points")) + " pts</span>"
        + "<span>updated " + esc(map_get(issue, "updated")) + "</span>")
    let comments = map_get(issue, "comments")
    dom.set_html(dom.query("#comments"),
        if len(comments) == 0 => "<div class=\"comment\">no comments yet</div>"
        else => comments |> map(comment_html) |> join(""))
    dom.set_class(dom.query("#drawer"), "open")
}

fn open_issue(id) = {
    set_st("sel", id)
    dom.fetch("GET", "/api/issues/" + id, "", (resp) => {
        let parsed = unwrap(json.parse(resp))
        if map_has_key(parsed, "error") => close_drawer()
        else => {
            render_drawer(parsed)
            reload_rows()
        }
    })
}

fn close_drawer() = {
    set_st("sel", "")
    dom.set_class(dom.query("#drawer"), "")
    reload_rows()
}

// ── data flow ──────────────────────────────────────────────────────────

fn reload_rows() = {
    dom.fetch("GET", issues_url(), "", (resp) => {
        let parsed = unwrap(json.parse(resp))
        if map_has_key(parsed, "error") => flash("error: " + map_get(parsed, "error"))
        else => render_rows(map_get(parsed, "items"), map_get(parsed, "total"))
    })
}

fn reload() = {
    reload_rows()
    render_stats()
    render_activity()
}

fn flash(msg) = dom.set_text(dom.query("#backend-note"), msg)

fn patch(id, body) =
    dom.fetch("PATCH", "/api/issues/" + id, body, (resp) => {
        reload()
        if st("sel") == id => open_issue(id)
    })

// ── mutations ──────────────────────────────────────────────────────────

fn next_status(s) = if s == "open" => "in-progress" else => {
    if s == "in-progress" => "done" else => "open"
}

fn next_priority(p) = if p == "low" => "medium" else => {
    if p == "medium" => "high" else => "low"
}

fn add_issue() = {
    let title = str.trim(dom.value(dom.query("#new-title")))
    if title != "" => {
        let assignee = str.trim(dom.value(dom.query("#new-assignee")))
        let pts = unwrap_or(str.parse_int(str.trim(dom.value(dom.query("#new-points")))), 0)
        let body = "{\"title\": \"" + json_esc(title) + "\""
            + ", \"assignee\": \"" + json_esc(assignee) + "\""
            + ", \"priority\": \"" + dom.value(dom.query("#new-priority")) + "\""
            + ", \"points\": " + show(pts) + "}"
        dom.fetch("POST", "/api/issues", body, (resp) => {
            dom.set_value(dom.query("#new-title"), "")
            dom.set_value(dom.query("#new-assignee"), "")
            dom.set_value(dom.query("#new-points"), "")
            dom.focus(dom.query("#new-title"))
            reload()
        })
    }
}

fn add_comment() = {
    let text = str.trim(dom.value(dom.query("#new-comment")))
    let id = st("sel")
    if text != "" && id != "" => {
        dom.fetch("POST", "/api/issues/" + id + "/comments",
            "{\"text\": \"" + json_esc(text) + "\", \"author\": \"you\"}", (resp) => {
            dom.set_value(dom.query("#new-comment"), "")
            open_issue(id)
            render_activity()
        })
    }
}

// ── filters and sorting ────────────────────────────────────────────────

fn set_filter(value) = {
    set_st("status", value)
    let mark = (id, name) => dom.set_class(dom.query("#" + id),
        if value == name => "active" else => "")
    mark("flt-all", "")
    mark("flt-open", "open")
    mark("flt-in-progress", "in-progress")
    mark("flt-done", "done")
    reload_rows()
}

share let sort_labels = [
    ["sort-id", "#"], ["sort-title", "Title"], ["sort-status", "Status"],
    ["sort-priority", "Priority"], ["sort-assignee", "Assignee"], ["sort-points", "Pts"]
]

fn set_sort(col) = {
    if st("sort") == col => {
        set_st("order", if st("order") == "asc" => "desc" else => "asc")
    } else => {
        set_st("sort", col)
        set_st("order", "asc")
    }
    for pair in sort_labels {
        let id = pair[0]
        let label = pair[1]
        let suffix = if str.replace(id, "sort-", "") == st("sort") => {
            if st("order") == "asc" => " ↑" else => " ↓"
        } else => ""
        dom.set_text(dom.query("#" + id), label + suffix)
    }
    reload_rows()
}

// ── delegated handlers ─────────────────────────────────────────────────

fn on_rows_click(tid) = {
    let id = str.substring(tid, 4, len(tid))
    if starts_with(tid, "open-") => open_issue(str.substring(tid, 5, len(tid)))
    else => { if starts_with(tid, "adv-") => {
        patch(id, "{\"status\": \"" + next_status(dom.get_text(dom.query("#" + tid))) + "\"}")
    } else => { if starts_with(tid, "pri-") => {
        patch(id, "{\"priority\": \"" + next_priority(dom.get_text(dom.query("#" + tid))) + "\"}")
    } else => { if starts_with(tid, "del-") => {
        dom.fetch("DELETE", "/api/issues/" + id, "", (resp) => {
            if st("sel") == id => close_drawer()
            reload()
        })
    } } } }
}

fn on_rows_change(e) = {
    let tid = map_get(e, "id")
    let value = map_get(e, "value")
    let id = str.substring(tid, 4, len(tid))
    if starts_with(tid, "asg-") => {
        patch(id, "{\"assignee\": \"" + json_esc(value) + "\"}")
    } else => { if starts_with(tid, "pts-") => {
        patch(id, "{\"points\": " + show(unwrap_or(str.parse_int(str.trim(value)), 0)) + "}")
    } }
}

fn on_meta_click(tid) = {
    // Status/priority pills inside the drawer cycle too.
    let id = str.substring(tid, 5, len(tid))
    if starts_with(tid, "dadv-") => {
        patch(id, "{\"status\": \"" + next_status(dom.get_text(dom.query("#" + tid))) + "\"}")
    } else => { if starts_with(tid, "dpri-") => {
        patch(id, "{\"priority\": \"" + next_priority(dom.get_text(dom.query("#" + tid))) + "\"}")
    } }
}

fn on_filter_click(tid) = {
    if tid == "flt-all" => set_filter("")
    else => { if starts_with(tid, "flt-") => set_filter(str.substring(tid, 4, len(tid))) }
}

fn on_sort_click(tid) = {
    if starts_with(tid, "sort-") => set_sort(str.substring(tid, 5, len(tid)))
}

// ── boot: bind once, render forever ────────────────────────────────────

dom.on(dom.query("#rows"), "click", (e) => { on_rows_click(map_get(e, "id")) })
dom.on(dom.query("#rows"), "change", (e) => { on_rows_change(e) })
dom.on(dom.query("#filters"), "click", (e) => { on_filter_click(map_get(e, "id")) })
dom.on(dom.query("#sort-row"), "click", (e) => { on_sort_click(map_get(e, "id")) })
dom.on(dom.query("#dmeta"), "click", (e) => { on_meta_click(map_get(e, "id")) })
dom.on(dom.query("#search"), "input", (e) => { reload_rows() })
dom.on(dom.query("#new-title"), "enter", (e) => { add_issue() })
dom.on(dom.query("#add-btn"), "click", (e) => { add_issue() })
dom.on(dom.query("#new-comment"), "enter", (e) => { add_comment() })
dom.on(dom.query("#comment-btn"), "click", (e) => { add_comment() })
dom.on(dom.query("#drawer-close"), "click", (e) => { close_drawer() })
dom.on(dom.query("#dtitle"), "change", (e) => {
    if st("sel") != "" => patch(st("sel"), "{\"title\": \"" + json_esc(map_get(e, "value")) + "\"}")
})

flash("frontend: olang (wasm) · backend: olang · sqlite")
reload()
