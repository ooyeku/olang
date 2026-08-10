// app.ol — the tracker frontend, in olang, event-delegated.
//
// Architecture: rows are pure HTML; exactly THREE listeners exist for
// the whole table (click and change on #rows via bubbling, enter on the
// new-issue input), bound once at boot. The shim's payload conventions
// carry the target: clicks deliver the clicked element's id, changes
// deliver "id\nvalue". No per-row binding, so re-renders are one
// set_html and the handler registry stays constant — the fix for the
// sluggish per-row rebinding this replaces.
//
// Stateless throughout: the server is the source of truth; a cell's
// current value lives in the DOM (a status IS its button label).

fn esc(s) = s
    |> str.replace("&", "&amp;")
    |> str.replace("<", "&lt;")
    |> str.replace(">", "&gt;")
    |> str.replace("\"", "&quot;")

fn next_status(s) = if s == "open" => "in-progress" else => {
    if s == "in-progress" => "done" else => "open"
}

fn next_priority(p) = if p == "low" => "medium" else => {
    if p == "medium" => "high" else => "low"
}

fn row_html(issue) = {
    let id = show(map_get(issue, "id"))
    let status = map_get(issue, "status")
    "<tr class=\"status-" + status + "\">"
        + "<td class=\"id\">" + id + "</td>"
        + "<td>" + esc(map_get(issue, "title")) + "</td>"
        + "<td><button class=\"status\" id=\"adv-" + id + "\">" + status + "</button></td>"
        + "<td><button class=\"status pri\" id=\"pri-" + id + "\">"
        + esc(map_get(issue, "priority")) + "</button></td>"
        + "<td><input class=\"cell\" id=\"asg-" + id + "\" value=\""
        + esc(map_get(issue, "assignee")) + "\"></td>"
        + "<td class=\"num\"><input class=\"cell num\" id=\"pts-" + id + "\" value=\""
        + show(map_get(issue, "points")) + "\"></td>"
        + "<td class=\"del\"><button id=\"del-" + id + "\">×</button></td>"
        + "</tr>"
}

fn update_footer(items) = {
    let by = (s) => len(items |> filter((i) => map_get(i, "status") == s))
    dom.set_text(dom.query("#count"), show(len(items)) + " issues")
    dom.set_text(dom.query("#summary"),
        show(by("open")) + " open · " + show(by("in-progress"))
        + " in progress · " + show(by("done")) + " done")
}

fn flash(msg) = dom.set_text(dom.query("#backend-note"), msg)

fn render(items) = {
    dom.set_html(dom.query("#rows"), items |> map(row_html) |> join(""))
    update_footer(items)
}

fn reload() = {
    dom.fetch("GET", "/api/issues", "", (resp) => {
        let parsed = unwrap(json.parse(resp))
        if map_has_key(parsed, "error") => {
            flash("error: " + map_get(parsed, "error"))
        } else => {
            render(map_get(parsed, "items"))
            flash("frontend: olang (wasm) · backend: olang · in-memory sqlite")
        }
    })
}

fn patch(id, body) =
    dom.fetch("PATCH", "/api/issues/" + id, body, (resp) => { reload() })

// One delegated click handler: route by the target id's prefix.
fn on_click(tid) = {
    let id = str.substring(tid, 4, len(tid))
    if starts_with(tid, "adv-") => {
        patch(id, "{\"status\": \"" + next_status(dom.get_text(dom.query("#" + tid))) + "\"}")
    } else => { if starts_with(tid, "pri-") => {
        patch(id, "{\"priority\": \"" + next_priority(dom.get_text(dom.query("#" + tid))) + "\"}")
    } else => { if starts_with(tid, "del-") => {
        dom.fetch("DELETE", "/api/issues/" + id, "", (resp) => { reload() })
    }}}
}

// One delegated change handler: assignee and points inputs.
fn on_change(payload) = {
    let parts = split(payload, "\n")
    let tid = parts[0]
    let value = parts[1]
    let id = str.substring(tid, 4, len(tid))
    if starts_with(tid, "asg-") => {
        patch(id, "{\"assignee\": \"" + esc(value) + "\"}")
    } else => { if starts_with(tid, "pts-") => {
        let pts = unwrap_or(str.parse_int(str.trim(value)), 0)
        patch(id, "{\"points\": " + show(pts) + "}")
    }}
}

fn add_issue(raw) = {
    let title = str.trim(raw)
    if title != "" => {
        dom.fetch("POST", "/api/issues", "{\"title\": \"" + esc(title) + "\"}", (resp) => {
            let input = dom.query("#new-title")
            dom.set_value(input, "")
            dom.focus(input)
            reload()
        })
    }
}

dom.on(dom.query("#rows"), "click", (tid) => { on_click(tid) })
dom.on(dom.query("#rows"), "change", (payload) => { on_change(payload) })
dom.on(dom.query("#new-title"), "enter", (val) => { add_issue(val) })
dom.on(dom.query("#add-btn"), "click", (tid) => { add_issue(dom.value(dom.query("#new-title"))) })
flash("frontend: olang (wasm) · backend: olang · in-memory sqlite")
reload()
