// l0.ol — Loom's engine (`gui`) on its own, before Loom: a window with a
// text field, a button, and a list, laid out by hand.
//
//   olang examples/gui/l0.ol                 a window (a display needed)
//   olang examples/gui/l0.ol --headless out.png
//                                            no display: draw the same
//                                            view and write its pixels
//   olang examples/gui/l0.ol --quit-after 3000
//
// Type a name and press Enter (or click Add); the list takes it. Tab moves
// between the field, the button, and the list; up and down move in the
// list. Everything is reachable by keyboard and named for a screen reader.

let args = os.args()
fn arg_after(flag) = {
    let i = index_of(args, flag)
    if i >= 0 && i + 1 < len(args) => args[i + 1] else => ""
}
fn index_of(xs, x) = fold(range(0, len(xs)), -1, (found, i) => if found < 0 && xs[i] == x => i else => found)

let headless_png = arg_after("--headless")
let quit_after = if arg_after("--quit-after") == "" => -1 else => to_int(arg_after("--quit-after"))

let opts = #{ "title": "Loom L0", "size": (520, 380), "min": (360, 300) }
let w = if headless_png != "" => gui.headless(#{ "size": (520, 380), "scale": 2 })
        else => match gui.open(opts) {
            Ok(win) => win,
            Err(e) => { println("no window: " + e + " (try --headless out.png)"); os.exit(1) }
        }
let events = gui.events()

// ── the model ───────────────────────────────────────────────────────
// One map, changed only by `update`, drawn only by `view` — Loom's
// shape, written by hand.
let initial = #{
    "name": "", "rev": 0, "at": 0, "width": 520.0, "height": 380.0,
    "status": "Type a name and press Enter.",
    "people": ["Ada Lovelace", "Grace Hopper", "Alan Turing", "Katherine Johnson",
               "Edsger Dijkstra", "Barbara Liskov", "Frances Allen", "Margaret Hamilton"]
}

// ── the view: positioned nodes ──────────────────────────────────────
let ROW = 32
let INK = "#18181b"
let MUTED = "#71717a"
let LINE = "#e4e4e7"
let ACCENT = "#2563eb"

fn view(m) = {
    let width = map_get(m, "width")
    let height = map_get(m, "height")
    let name = map_get(m, "name")
    let people = map_get(m, "people")
    let at = map_get(m, "at")
    let pad = 24
    let inner = width - 2 * pad
    let list_top = 136
    let list_h = height - list_top - 56
    let mut ops = [
        #{ "key": "root", "role": "group", "box": (0, 0, width, height), "style": #{ "bg": "#ffffff" } },
        #{ "key": "title", "parent": "root", "role": "heading", "level": 1, "box": (pad, 20, inner, 28),
           "text": "People", "style": #{ "size": 20, "weight": 650, "color": INK } },
        #{ "key": "label", "parent": "root", "role": "text", "box": (pad, 60, inner, 18),
           "text": "Name", "style": #{ "size": 13, "weight": 500, "color": MUTED } },
        #{ "key": "name", "parent": "root", "role": "input", "name": "Name", "box": (pad, 82, inner - 92, 36),
           "edit": #{ "value": name, "rev": map_get(m, "rev"), "placeholder": "e.g. Dennis Ritchie" },
           "style": #{ "bg": "#ffffff", "border": "#d4d4d8", "border_width": 1, "radius": 8,
                       "pad": (0, 10, 0, 10), "size": 15, "color": INK,
                       "focus": #{ "border": ACCENT, "border_width": 2 } } },
        #{ "key": "add", "parent": "root", "role": "button", "box": (width - pad - 80, 82, 80, 36),
           "text": "Add", "disabled": name == "",
           "style": #{ "bg": if name == "" => "#a1a1aa" else => ACCENT, "color": "#ffffff", "radius": 8,
                       "size": 15, "weight": 600, "align": "center", "valign": "center",
                       "hover": #{ "bg": "#1d4ed8" }, "pressed": #{ "bg": "#1e40af" } } },
        #{ "key": "people", "parent": "root", "role": "list", "name": "People", "scroll": true,
           "box": (pad, list_top, inner, list_h),
           "style": #{ "border": LINE, "border_width": 1, "radius": 8, "clip": true } }
    ]
    for i in range(0, len(people)) {
        let selected = i == at
        ops = ops + [#{ "key": "p" + to_string(i), "parent": "people", "role": "listitem",
            "box": (1, 1 + i * ROW, inner - 2, ROW), "text": people[i], "selected": selected,
            "focusable": false,
            "style": #{ "bg": if selected => "#eff6ff" else => "#ffffff",
                        "color": if selected => "#1e3a8a" else => INK, "size": 14,
                        "valign": "center", "pad": (0, 12, 0, 12),
                        "hover": #{ "bg": if selected => "#dbeafe" else => "#f4f4f5" } } }]
    }
    ops + [#{ "key": "status", "parent": "root", "role": "status", "box": (pad, height - 40, inner, 20),
              "text": map_get(m, "status"), "style": #{ "size": 13, "color": MUTED } }]
}

fn select(m, i) = {
    let people = map_get(m, "people")
    let m2 = map_set(m, "at", i)
    map_set(m2, "status", people[i] + " selected.")
}

fn add_person(m) = {
    let name = map_get(m, "name")
    if name == "" => m
    else => {
        let people = map_get(m, "people") + [name]
        let m2 = map_set(map_set(m, "people", people), "at", len(people) - 1)
        let m3 = map_set(map_set(m2, "status", "Added " + name + "."), "name", "")
        map_set(m3, "rev", map_get(m3, "rev") + 1)
    }
}

/// One event in, the next model out.
fn update(m, e) = {
    let kind = map_get(e, "kind")
    if kind == "resize" => map_set(map_set(m, "width", map_get(e, "width")), "height", map_get(e, "height"))
    else if kind == "changed" && map_get(e, "key") == "name" =>
        map_set(map_set(m, "name", map_get(e, "value")), "rev", map_get(e, "rev"))
    else if kind == "submit" => add_person(m)
    else if kind == "activate" && map_get(e, "key") == "add" => add_person(m)
    else if kind == "activate" && str.starts_with(map_get(e, "key"), "p") =>
        select(m, to_int(str.slice(map_get(e, "key"), 1, str.length(map_get(e, "key")))))
    else if kind == "key" && map_get(e, "target") == "people" && map_get(e, "key") == "down" =>
        select(m, min(map_get(m, "at") + 1, len(map_get(m, "people")) - 1))
    else if kind == "key" && map_get(e, "target") == "people" && map_get(e, "key") == "up" =>
        select(m, max(map_get(m, "at") - 1, 0))
    else => m
}

fn drain() = {
    let mut out = []
    let mut more = true
    while more {
        match chan.try_recv(events) {
            Ok(e) => { out = out + [e] },
            _ => { more = false }
        }
    }
    out
}

let mut m = initial
gui.apply(w, view(m))
gui.apply(w, [#{ "op": "focus", "key": "name" }])

if headless_png != "" => {
    gui.input(w, #{ "kind": "text", "text": "Dennis Ritchie" })
    for e in drain() { m = update(m, e) }
    gui.apply(w, view(m))
    fs.write_bytes(headless_png, gui.read(w, "pixels"))
    println("wrote " + headless_png)
    os.exit(0)
}

let started = time.monotonic()
let mut running = true
while running {
    match chan.recv_timeout(events, 100) {
        Ok(e) => {
            if map_get(e, "kind") == "close_requested" => { running = false }
            let before = map_get(m, "people")
            m = update(m, e)
            gui.apply(w, view(m))
            if len(map_get(m, "people")) != len(before) =>
                { gui.apply(w, [#{ "op": "scroll", "key": "people", "to": (0, map_get(m, "at") * ROW) }]) }
            ()
        },
        _ => ()
    }
    if quit_after >= 0 && time.monotonic() - started > quit_after => { running = false }
}
gui.close(w)
