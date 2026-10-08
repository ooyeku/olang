// tools/window_churn.ol — open, drive and close real windows, again and
// again, the way an editor does: accessibility nodes with actions, a title
// changed, the document marked edited, a frame drawn, the window closed;
// then the program waits a moment and ends. Run it many times to catch a
// crash in a window's teardown (tools/window_churn.sh counts them).
//
//   olang tools/window_churn.ol [windows] [linger_ms]

let args = skip(os.args(), 1)
let n = if len(args) > 0 => to_int(args[0]) else => 3
let linger = if len(args) > 1 => to_int(args[1]) else => 0
let ev = gui.events()
for i in 0..n {
    let w = unwrap(gui.open(#{ "title": "churn " + to_string(i), "size": (360, 220) }))
    gui.apply(w, [
        #{ "key": "root", "box": (0, 0, 360, 220), "style": #{ "bg": "#ffffff" } },
        #{ "key": "card", "parent": "root", "role": "button", "box": (10, 10, 140, 40), "text": "Card",
           "focusable": true, "actions": ["Move left", "Move right"], "description": "a card" },
        #{ "key": "list", "parent": "root", "role": "list", "box": (170, 10, 140, 120), "name": "a list",
           "focusable": true, "active": "row1", "actions": ["Sort"] },
        #{ "key": "row0", "parent": "list", "role": "listitem", "box": (0, 0, 140, 40), "text": "Row zero" },
        #{ "key": "row1", "parent": "list", "role": "listitem", "box": (0, 40, 140, 40), "text": "Row one" }
    ])
    gui.set(w, #{ "title": "churn " + to_string(i) + " — edited", "edited": true })
    let t0 = time.monotonic()
    while time.monotonic() - t0 < 250.0 {
        let _e = chan.recv_timeout(ev, 50)
    }
    gui.set(w, #{ "edited": false })
    gui.close(w)
}
if linger > 0 => { let _s = time.sleep(linger) }
println("churn: " + to_string(n) + " windows opened and closed")
