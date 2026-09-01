//! view — where state becomes pixels (browser side).
//!
//! `mount(selector, view_fn, initial)` wires the loop: the view
//! function maps state to a `web.html` node tree; every `apply(f)`
//! (and every handled action) re-renders the tree into the mount
//! point. Events use delegation: one set of listeners on the root,
//! dispatching by each element's `data-action` — so re-rendered
//! markup never re-binds anything.
//!
//! v1 renders by `set_html` of the whole tree, the pattern the example
//! apps proved out; because views are plain data, a diffing engine can
//! replace this without changing a single caller.

use lib.html { render }
use lib.state { init, current, update }

let mount_root = cell.new("")
let mount_view = cell.new(())
let mount_actions = cell.new(#{})

/// Repaint now: the view function over the current state, rendered
/// into the mount point. `apply` and action dispatch call this — a
/// manual call is only needed after out-of-band state changes.
share fn rerender() = {
    let f = cell.get(mount_view)
    let root = cell.get(mount_root)
    if f != () && root != "" && dom.available() => {
        dom.set_html(dom.query(root), render(f(current())))
    }
}

/// Update state and repaint — what an event handler calls:
/// `apply((s) => map_set(s, "todos", s.todos + [t]))`.
share fn apply(f) = {
    let next = update(f)
    rerender()
    next
}

/// Register a named action: `action("todo.add", (ev) => ...)`. Any
/// element carrying `data-action="todo.add"` fires it on click (or
/// change/submit for inputs); the handler receives the event map
/// (`id`, `value`, `key`, `data`, modifiers) and the view repaints
/// after it runs.
share fn action(name, handler) =
    cell.update(mount_actions, (m) => map_set(m, name, handler))

fn dispatch_action(ev) = {
    let data = map_get(ev, "data")
    let name = if data == () => "" else => {
        let a = map_get(data, "action")
        if a == () => "" else => a
    }
    if name != "" => {
        let handlers = cell.get(mount_actions)
        // Exact name first; then the prefix before ":" — so one
        // registered "toggle" serves "toggle:7", the handler reading
        // the full name from the event's data.
        let h = {
            let exact = map_get(handlers, name)
            if exact != () => exact
            else => {
                let colon = str.index_of(name, ":")
                if colon == () => ()
                else => map_get(handlers, str.substring(name, 0, colon))
            }
        }
        if h != () => {
            // Repaint only if the handler changed state: `apply` already
            // repaints, and an unconditional rerender here would wipe
            // focus and in-progress typing on every click into an
            // action-carrying form control.
            let before = current()
            let r = h(ev)
            if current() != before => rerender() else => ()
        }
    }
}

/// The full `data-action` string from an event — how a prefix handler
/// reads its argument: `action_arg(ev)` on "toggle:7" is "7".
share fn action_arg(ev) = {
    let data = map_get(ev, "data")
    let name = if data == () => "" else => {
        let a = map_get(data, "action")
        if a == () => "" else => a
    }
    let colon = str.index_of(name, ":")
    if colon == () => "" else => str.substring(name, colon + 1, str.length(name))
}

/// Wire the app: `mount("#app", view_fn, initial_state)`. In the
/// browser this installs the state, paints the first frame, and binds
/// the delegated listeners. Run natively — where there is no DOM —
/// it degrades to what isomorphic views make possible: the initial
/// frame rendered to stdout as HTML, a static preview of the app.
share fn mount(selector, view_fn, initial) = {
    cell.set(mount_root, selector)
    cell.set(mount_view, view_fn)
    init(initial)
    if dom.available() => {
        rerender()
        let root = dom.query(selector)
        dom.on(root, "click", (ev) => dispatch_action(ev))
        dom.on(root, "change", (ev) => dispatch_action(ev))
        dom.on(root, "submit", (ev) => dispatch_action(ev))
        dom.on(root, "keydown", (ev) => {
            if map_get(ev, "key") == "Enter" => dispatch_action(ev) else => ()
        })
    }
    else => {
        println(render(view_fn(initial)))
        println("")
        println("── static preview: this is a browser program. Serve it —")
        println("── `olang run server.ol`, then open the printed address.")
    }
}

/// The current value of the input with this id — how handlers read
/// fields: `input_value("title")`. Empty when there is no DOM.
share fn input_value(id) =
    if dom.available() => dom.value(dom.query("#" + id)) else => ""
