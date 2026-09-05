//! view — where state becomes pixels (browser side).
//!
//! `mount(selector, view_fn, initial)` wires the loop: the view
//! function maps state to a `web.html` node tree; every `apply(f)`
//! (and every handled action) re-renders the tree into the mount
//! point. Events use delegation: one set of listeners on the root,
//! dispatching by each element's `data-action` — so re-rendered
//! markup never re-binds anything.
//!
//! A repaint reconciles: the rendered tree is morphed into the mount
//! point (`dom.morph`) — text updated in place, attributes diffed,
//! children matched by `data-key` (else by position and tag) — so the
//! nodes that did not change are the nodes the browser keeps, focus and
//! caret and scroll included. Give list rows a `"data-key"` attribute
//! and reordering moves their elements instead of rebuilding them.

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
        dom.morph(dom.query(root), render(f(current())))
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

/// Is this action awaiting its confirming second click? Without a DOM
/// — a view rendered on the server, on a worker thread that must not
/// touch this module's cells — nothing is ever armed.
share fn confirm_armed(name) =
    if !dom.available() => false
    else => {
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
    if dom.available() => dom.morph(dom.query("#" + id), render(node)) else => ()

/// The argument of the event's `data-action` — how a prefix handler
/// reads it: `action_arg(ev)` on "toggle:7" is "7". A confirm-wrapped
/// action ("confirm:del:7", what `btn_confirm` renders) carries the
/// same argument as the action it confirms.
share fn action_arg(ev) = {
    let data = map_get(ev, "data")
    let name = if data == () => "" else => {
        let a = map_get(data, "action")
        if a == () => "" else => a
    }
    let bare = if str.starts_with(name, "confirm:") =>
        str.substring(name, 8, str.length(name)) else => name
    let colon = str.index_of(bare, ":")
    if colon == () => "" else => str.substring(bare, colon + 1, str.length(bare))
}

test "action_arg: the argument after the handler's prefix, confirm or not" {
    assert_eq(action_arg(#{ "data": #{ "action": "toggle:7" } }), "7")
    assert_eq(action_arg(#{ "data": #{ "action": "confirm:del:7" } }), "7")
    assert_eq(action_arg(#{ "data": #{ "action": "add" } }), "")
    assert_eq(action_arg(#{ "data": () }), "")
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
        let root = dom.query(selector)
        // A server-rendered first paint travels with the state it was
        // rendered from (the mount point names the element holding it).
        // The store starts there — the server's keys over the caller's
        // defaults — so what the page shows and what the handlers see
        // are one state. Keys the server did not set (a `url` or
        // `local` field restored by `hydrate`) keep the caller's value.
        let state_id = dom.get_attr(root, "data-olang-state")
        if state_id != "" => {
            match json.parse(dom.get_text(dom.query("#" + state_id))) {
                Ok(server_state) => init(merge_state(initial, server_state)),
                Err(e) => ()
            }
        } else => ()
        rerender()
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

/// The caller's initial state under the server's: every key the server
/// rendered from wins; the rest stay.
fn merge_state(base, server) =
    if contains(["Map", "JsonObject", "Object"], typeof(server)) && base != ()
        && contains(["Map", "JsonObject", "Object"], typeof(base)) =>
        fold(map_keys(server), base, (acc, k) => map_set(acc, k, map_get(server, k)))
    else => server

test "merge_state: the server's keys over the caller's defaults" {
    let merged = merge_state(#{ "notes": [], "errors": [], "theme": "dark" },
                             unwrap(json.parse("{\"notes\": [1, 2], \"errors\": []}")))
    assert_eq(len(map_get(merged, "notes")), 2)
    assert_eq(map_get(merged, "theme"), "dark")
    // A non-map state is replaced wholesale.
    assert_eq(merge_state(0, 5), 5)
}

/// The current value of the input with this id — how handlers read
/// fields: `input_value("title")`. Empty when there is no DOM.
share fn input_value(id) =
    if dom.available() => dom.value(dom.query("#" + id)) else => ""
