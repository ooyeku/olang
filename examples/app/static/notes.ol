// notes — a small SPA proving the stage-3 surface end to end:
// the ui module's keyed reconciliation, push_state/on_route navigation,
// and localStorage persistence. State is the storage itself.

use ui { h, hk, render }

let list = dom.query("#list")
let input = dom.query("#new-note")

fn load() = unwrap_or(json.parse(dom.storage_get("notes-v1")), [])
fn save(notes) = dom.storage_set("notes-v1", unwrap(json.stringify(notes)))

fn selected() = {
    let q = map_get(dom.location(), "query")
    if map_has_key(q, "sel") => map_get(q, "sel") else => ""
}

fn note_row(n, sel) = {
    let id = map_get(n, "id")
    let classes = if id == sel => "note sel" else => "note"
    hk(id, "div", #{ "class": classes, "id": "note-" + id }, [
        h("span", #{ "id": "t-" + id }, [map_get(n, "title")]),
        h("button", #{ "id": "del-" + id }, ["x"])
    ])
}

fn draw() = {
    let sel = selected()
    let mut rows = []
    for n in load() {
        rows = rows + [note_row(n, sel)]
    }
    render(list, rows)
}

fn add_note() = {
    let title = dom.value(input)
    if str.trim(title) != "" => {
        let id = "n" + show(random.randint(10000, 99999))
        save(load() + [#{ "id": id, "title": title }])
        dom.set_value(input, "")
        draw()
    }
}

fn without(notes, id) = filter(notes, (n) => map_get(n, "id") != id)

fn on_list_click(e) = {
    let tid = map_get(e, "id")
    if starts_with(tid, "del-") => {
        save(without(load(), str.substring(tid, 4, len(tid))))
        draw()
    } else => { if starts_with(tid, "t-") || starts_with(tid, "note-") => {
        let id = str.substring(tid, len(split(tid, "-")[0]) + 1, len(tid))
        dom.push_state("/notes.html?sel=" + id)
        draw()
    }}
}

dom.on(list, "click", (e) => { on_list_click(e) })
dom.on(input, "enter", (e) => { add_note() })
dom.on(dom.query("#add"), "click", (e) => { add_note() })
dom.on_route((r) => { draw() })

if len(load()) == 0 => {
    save([#{ "id": "n1", "title": "ship stage 3" }, #{ "id": "n2", "title": "try the back button" }])
}
draw()
