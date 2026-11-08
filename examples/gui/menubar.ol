// menubar.ol — a menu bar item with a live readout, a panel under it
// with a sparkline, and a notification (macOS).
//
//   olang examples/gui/menubar.ol                    the item, until Quit
//   olang examples/gui/menubar.ol --quit-after 5000  gone after 5 seconds
//
// The item shows this process's memory, sampled every second. Click it
// for the panel (the last minute as a sparkline); a click outside the
// panel or Escape puts it away. Right-click the item for a notification.
// The application is an accessory: no Dock icon and no main window.

let args = os.args()
fn arg_after(flag) = {
    let i = fold(range(0, len(args)), -1, (found, i) => if found < 0 && args[i] == flag => i else => found)
    if i >= 0 && i + 1 < len(args) => args[i + 1] else => ""
}
let quit_after = if arg_after("--quit-after") == "" => -1 else => to_int(arg_after("--quit-after"))

if !gui.platform().status_item => {
    println("menu bar items need macOS (gui.platform().status_item is false)")
    os.exit(0)
}

let _a = gui.accessory(true)
let events = gui.events()
let item = unwrap(gui.status_item(#{ "icon": "symbol:gauge.with.dots.needle.33percent", "title": "…", "tooltip": "olang's menu bar example" }))

// This process's resident memory, in MB (ps answers kilobytes).
fn rss_mb() = match os.exec(["/bin/ps", "-o", "rss=", "-p", to_string(os.pid())]) {
    Ok(r) => to_float(unwrap_or(str.parse_int(str.trim(r.stdout)), 0)) / 1024.0,
    Err(e) => 0.0
}

fn one_place(x) = to_string(to_float(to_int(x * 10.0)) / 10.0)

let W = 280
let H = 168

// The panel's nodes: a title, the last value, and the sparkline.
fn panel_view(samples) = {
    let top = fold(samples, 1.0, (a, v) => max(a, v)) * 1.15
    let n = len(samples)
    let step = if n <= 1 => 0.0 else => to_float(W - 32) / to_float(n - 1)
    let points = map(range(0, n), (i) => (to_float(16) + step * to_float(i), to_float(H - 20) - samples[i] / top * to_float(H - 84)))
    let last = if n == 0 => 0.0 else => samples[n - 1]
    [#{ "key": "root", "role": "group", "box": (0, 0, W, H), "style": #{ "bg": "#ffffff" } },
     #{ "key": "title", "parent": "root", "role": "heading", "level": 1, "box": (16, 14, W - 32, 18),
        "text": "Memory", "style": #{ "size": 13, "weight": 600, "color": "#18181b" } },
     #{ "key": "now", "parent": "root", "role": "text", "box": (16, 34, W - 32, 26),
        "text": one_place(last) + " MB", "style": #{ "size": 20, "weight": 600, "color": "#18181b" } },
     #{ "key": "spark", "parent": "root", "role": "figure", "name": "Memory over the last minute", "box": (0, 0, W, H),
        "draw": [#{ "op": "line", "x1": 16, "y1": H - 20, "x2": W - 16, "y2": H - 20, "color": "#e4e4e7" },
                 #{ "op": "path", "points": points, "stroke": "#2563eb", "width": 1.5 }] }]
}

let mut samples = []
let mut panel = ()
let mut running = true
let started = time.monotonic()
let mut next = 0

while running {
    let now = time.monotonic() - started
    if now >= next => {
        let more = samples + [rss_mb()]
        samples = if len(more) > 60 => skip(more, len(more) - 60) else => more
        let _t = gui.status_set_title(item, one_place(samples[len(samples) - 1]) + " MB")
        if panel != () => { let _p = gui.apply(panel, panel_view(samples)) }
        next = next + 1000
    }
    match chan.recv_timeout(events, #{ "timeout_ms": max(1, next - now) }) {
        Ok(e) => {
            if e.kind == "status" && e.button == "right" => {
                let _n = gui.notify(#{ "title": "olang", "body": "This process uses " + one_place(samples[len(samples) - 1]) + " MB.", "action": "show" })
            } else if e.kind == "status" && panel == () && !e.was_open => {
                panel = unwrap(gui.open(#{ "panel": item, "size": (W, H), "renderer": "software" }))
                let _p = gui.apply(panel, panel_view(samples))
            } else if e.kind == "status" && panel != () => {
                gui.close(panel)
                panel = ()
            } else if e.kind == "dismiss" && panel != () => {
                // put away: closed, so the panel costs nothing while it is
                gui.close(panel)
                panel = ()
            } else if e.kind == "notification" => {
                println("the notification was clicked: " + show(e.action))
            }
        },
        _ => ()
    }
    if quit_after >= 0 && time.monotonic() - started > quit_after => { running = false }
}
let _r = gui.status_remove(item)
