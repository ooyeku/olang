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

fn row_html(issue) = {
    let id = show(map_get(issue, "id"))
    let status = map_get(issue, "status")
    "<tr class=\"status-" + status + "\">"
        + "<td class=\"id\">" + id + "</td>"
        + "<td>" + esc(map_get(issue, "title")) + "</td>"
        + "<td><button class=\"status\" id=\"adv-" + id + "\">" + status + "</button></td>"
        + "<td>" + esc(map_get(issue, "priority")) + "</td>"
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

fn reload() = {
    dom.fetch("GET", "/api/issues", "", (resp) => {
        render(map_get(unwrap(json.parse(resp)), "items"))
    })
}

fn add_issue() = {
    let title_input = dom.query("#new-title")
    let title = str.trim(dom.value(title_input))
    if title != "" => {
        dom.fetch("POST", "/api/issues", "{\"title\": \"" + esc(title) + "\"}", (resp) => {
            dom.set_value(dom.query("#new-title"), "")
            reload()
        })
    }
}

dom.on(dom.query("#add-btn"), "click", () => { add_issue() })
dom.set_text(dom.query("#backend-note"),
    "frontend: olang (wasm) · backend: olang · in-memory sqlite")
reload()
