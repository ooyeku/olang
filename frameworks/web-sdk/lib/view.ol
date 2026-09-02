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

/// A click on a form control is a focus/open gesture (a select opening
/// its dropdown, a caret landing in an input) — never an action. The
/// control's action fires on change (and Enter, via the keydown
/// delegation) instead. Without this, clicking a select fired its
/// action with the PRE-change values and the async repaint snapped the
/// open dropdown shut.
fn click_on_form_control(ev) =
    map_get(ev, "type") == "click"
        && contains(["input", "select", "textarea"], map_get(ev, "tag"))

// An input's action fires on Enter (the keydown delegation) and again
// on the `change` event the same Enter produces — two invocations for
// one intent, milliseconds apart, which doubled every non-idempotent
// handler (a create, an append). The Enter dispatch records the input;
// the change that follows within the same beat is the same intent and
// is skipped.
let last_enter = cell(#{ "id": "", "at": 0 })

fn is_repeat_of_enter(ev) = {
    if map_get(ev, "type") != "change" => false
    else => {
        let seen = cell.get(last_enter)
        map_get(seen, "id") != "" && map_get(seen, "id") == map_get(ev, "id")
            && time.monotonic_ms() - map_get(seen, "at") < 100
    }
}

// Two-step confirmation for destructive actions: a `confirm:<name>`
// action arms itself on the first click (the view repaints, and
// `confirm_armed(name)` lets `btn_confirm` change its label) and fires
// `<name>` on the second within a few seconds; anything else disarms
// it. The state lives here so every app does not reinvent the
// two-click button.
let armed_confirm = cell(#{ "name": "", "at": 0 })

/// Is this action awaiting its confirming second click?
share fn confirm_armed(name) = {
    let a = cell.get(armed_confirm)
    map_get(a, "name") == name && time.monotonic_ms() - map_get(a, "at") < 4000
}

fn resolve_confirm(name) =
    if str.starts_with(name, "confirm:") => {
        let target = str.substring(name, 8, str.length(name))
        if confirm_armed(target) => {
            cell.set(armed_confirm, #{ "name": "", "at": 0 })
            target
        } else => {
            cell.set(armed_confirm, #{ "name": target, "at": time.monotonic_ms() })
            rerender()
            ""
        }
    } else => name

fn dispatch_action(ev) = {
    let data = map_get(ev, "data")
    let raw_name = if data == () || click_on_form_control(ev) || is_repeat_of_enter(ev) => "" else => {
        let a = map_get(data, "action")
        if a == () => "" else => a
    }
    let name = if raw_name == "" => "" else => resolve_confirm(raw_name)
    if map_get(ev, "type") == "keydown" && map_get(ev, "id") != () && map_get(ev, "id") != "" => {
        cell.set(last_enter, #{ "id": map_get(ev, "id"), "at": time.monotonic_ms() })
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

/// Repaint one subtree in place: render `node` into the element with
/// this id, leaving the rest of the page — and its focused input —
/// untouched. For the toast, the counter, the chart that should not
/// cost a whole-page render on every store write. `apply` remains the
/// whole-view repaint.
share fn patch(id, node) =
    if dom.available() => dom.set_html(dom.query("#" + id), render(node)) else => ()

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
