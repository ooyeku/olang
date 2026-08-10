// app.ol — the tracker frontend, in olang. Runs in the browser: the
// page shim (olang-dom.js) boots this file in a persistent wasm session;
// every handler below is an ordinary olang closure re-entered per event.
//
// Deliberately stateless: olang closures capture environments by value
// (the same snapshot semantics as spawn/par_map), so cross-handler
// mutable state does not exist — and is not needed. The server is the
// source of truth; each response flows DOWN through arguments, and the
// only client state is the DOM itself (a row's current status is its
// button's label).

fn esc(s) = s
    |> str.replace("&", "&amp;")
    |> str.replace("<", "&lt;")
    |> str.replace(">", "&gt;")

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
        + "<td>" + esc(map_get(issue, "assignee")) + "</td>"
        + "<td class=\"num\">" + show(map_get(issue, "points")) + "</td>"
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

fn bind_row(id) = {
    let adv = dom.query("#adv-" + id)
    dom.on(adv, "click", () => {
        // The button's own label is the row's current status.
        let body = "{\"status\": \"" + next_status(dom.get_text(adv)) + "\"}"
        dom.fetch("PATCH", "/api/issues/" + id, body, (resp) => { reload() })
    })
    let pri = dom.query("#pri-" + id)
    dom.on(pri, "click", () => {
        let body = "{\"priority\": \"" + next_priority(dom.get_text(pri)) + "\"}"
        dom.fetch("PATCH", "/api/issues/" + id, body, (resp) => { reload() })
    })
    dom.on(dom.query("#del-" + id), "click", () => {
        dom.fetch("DELETE", "/api/issues/" + id, "", (resp) => { reload() })
    })
}

fn render(items) = {
    dom.set_html(dom.query("#rows"), items |> map(row_html) |> join(""))
    update_footer(items)
    for issue in items {
        bind_row(show(map_get(issue, "id")))
    }
}

fn flash(msg) = dom.set_text(dom.query("#backend-note"), msg)

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

// Enter in the input delivers its value as the payload; the button
// reads the field itself.
dom.on(dom.query("#new-title"), "enter", (val) => { add_issue(val) })
dom.on(dom.query("#add-btn"), "click", () => { add_issue(dom.value(dom.query("#new-title"))) })
dom.set_text(dom.query("#backend-note"),
    "frontend: olang (wasm) · backend: olang · in-memory sqlite")
reload()
