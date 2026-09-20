//! view — where state becomes pixels (browser side).
//!
//! `mount(selector, view_fn, initial)` wires the loop: the view
//! function maps state to a `web.html` node tree; every `apply(f)`
//! (and every handled action) re-renders the tree into the mount
//! point. Events use delegation: one set of listeners on the root,
//! dispatching by each element's `data-action` — so re-rendered
//! markup never re-binds anything.
//!
//! A repaint reconciles: the node tree is handed to the host as data
//! (`dom.patch`) and diffed against the live DOM — text updated in
//! place, attributes diffed, children matched by `data-key` (else by
//! position and tag) — so the nodes that did not change are the nodes
//! the browser keeps, focus and caret and scroll included. Give list
//! rows a `"data-key"` attribute and reordering moves their elements
//! instead of rebuilding them; wrap a subtree in `memo` and it is not
//! even rebuilt while its inputs stand.

use lib.html { render, take_memo_tally, forget_memos }
use lib.state { init, current, update }

let mount_root = cell.new("")
let mount_view = cell.new(())
let mount_actions = cell.new(#{})
// How many repaints have been asked for. Action dispatch reads it
// across a handler: one whose own `apply` already repainted is not
// painted a second time.
let paints = cell.new(0)
// What the repaints cost, and the ones that were not needed (see
// `paint_stats`).
let paint_log = cell.new(#{
    "skipped": 0, "healed": 0, "view_ms": 0, "serialize_ms": 0.0, "patch_ms": 0.0,
    "nodes": 0, "memo_hits": 0, "memo_misses": 0, "last": #{}
})
// The top-level state keys a repaint depends on: `watched` when a view
// named them, everything but `unwatched` otherwise.
let watched = cell.new([])
let unwatched = cell.new([])

fn paint_once(f, root) = {
    let t0 = time.monotonic_ms()
    let tree = f(current())
    let view_ms = time.monotonic_ms() - t0
    let answer = dom.patch(dom.query(root), tree)
    let tally = take_memo_tally()
    // A runtime older than the answering `dom.patch` returns Unit.
    let told = contains(["Map", "JsonObject"], typeof(answer))
    let last = #{
        "view_ms": view_ms,
        "serialize_ms": if told => map_get(answer, "serialize_ms") else => 0.0,
        "patch_ms": if told => map_get(answer, "patch_ms") else => 0.0,
        "nodes": if told => map_get(answer, "nodes") else => 0,
        "memo_hits": map_get(tally, "hits"),
        "memo_misses": map_get(tally, "misses")
    }
    cell.update(paint_log, (log) => fold(map_keys(last), map_set(log, "last", last),
        (acc, k) => map_set(acc, k, map_get(acc, k) + map_get(last, k))))
    if told => map_get(answer, "missing") else => []
}

/// Repaint now: the view function over the current state, rendered
/// into the mount point. `apply` and action dispatch call this — a
/// manual call is only needed after out-of-band state changes.
///
/// A `keep` the page could not honor — a memo stamped while its subtree
/// was off the page — comes back from the patch; its stamp is dropped
/// and the view is painted once more, so a stale memo costs one extra
/// pass instead of a subtree that never appears.
share fn rerender() = {
    cell.set(paints, cell.get(paints) + 1)
    let f = cell.get(mount_view)
    let root = cell.get(mount_root)
    if f != () && root != "" && dom.available() => {
        let missing = paint_once(f, root)
        if len(missing) > 0 => {
            forget_memos(missing)
            cell.update(paint_log, (log) => map_set(log, "healed", map_get(log, "healed") + 1))
            paint_once(f, root)
            ()
        } else => ()
    }
}

/// The repaints asked for so far (a test's, or a profiler's, count).
share fn paint_count() = cell.get(paints)

/// What repainting has cost since the page loaded: `repaints`, the
/// `skipped` ones (a state change that moved no watched key), the
/// `healed` ones (a second pass after a `keep` the page could not
/// honor), and — summed over every pass, with the latest pass alone
/// under `last` — `view_ms` in the view function, `serialize_ms` and
/// `patch_ms` inside `dom.patch`, the `nodes` handed over, and
/// `memo_hits` / `memo_misses`.
share fn paint_stats() = map_set(cell.get(paint_log), "repaints", cell.get(paints))

/// Name the top-level state keys the view reads: `watch(["issues",
/// "filter", "route"])`. A state change that moves none of them does
/// not repaint — a heartbeat, a presence ping, a poll's bookkeeping.
/// Calls add up; with none, every key is watched except those
/// `unwatch` named.
share fn watch(keys) = cell.update(watched, (w) => w + filter(keys, (k) => !contains(w, k)))

/// Name top-level state keys no view reads: `unwatch(["live",
/// "inflight"])` — what a runtime over this SDK declares for its own
/// bookkeeping. A change confined to them does not repaint.
share fn unwatch(keys) = cell.update(unwatched, (w) => w + filter(keys, (k) => !contains(w, k)))

fn is_map(v) = contains(["Map", "JsonObject", "Object"], typeof(v))

/// Whether going from `before` to `after` moved anything a view reads.
fn moved_watched(before, after) =
    if !is_map(before) || !is_map(after) => before != after
    else => {
        let named = cell.get(watched)
        let ignored = cell.get(unwatched)
        let keys = if len(named) > 0 => named else => {
            let all = map_keys(after)
            all + filter(map_keys(before), (k) => !contains(all, k))
        }
        fold(keys, false, (hit, k) => hit || (!contains(ignored, k) && map_get(before, k) != map_get(after, k)))
    }

// How many state changes have been judged for a repaint, and the state
// the last judgment saw: action dispatch judges only what a handler
// changed AFTER its own `apply` did.
let judged = cell.new(0)
let judged_state = cell.new(())

/// Repaint unless the change moved nothing a view reads.
fn repaint_for(before, after) = {
    cell.set(judged, cell.get(judged) + 1)
    cell.set(judged_state, after)
    if moved_watched(before, after) => rerender()
    else => cell.update(paint_log, (log) => map_set(log, "skipped", map_get(log, "skipped") + 1))
}

/// Update state and repaint — what an event handler calls:
/// `apply((s) => map_set(s, "todos", s.todos + [t]))`. A change that
/// moved no watched key (`watch`, `unwatch`) does not repaint.
share fn apply(f) = {
    let before = current()
    let next = update(f)
    repaint_for(before, current())
    next
}

/// Register a named action: `action("todo.add", (ev) => ...)`. Any
/// element carrying `data-action="todo.add"` fires it on click (or
/// change/submit for inputs); the handler receives the event map
/// (`id`, `value`, `key`, `data`, modifiers) and the view repaints
/// after it runs.
share fn action(name, handler) =
    cell.update(mount_actions, (m) => map_set(m, name, handler))

/// Register many actions at once: `actions(#{ "todo.add": add, "todo.toggle":
/// toggle })` updates the handler table in one step where an app the
/// size of a product registers a hundred and more at load — each
/// `action` call is a cell update of its own, and they added up to a
/// measurable slice of the boot.
share fn actions(table) =
    cell.update(mount_actions, (m) =>
        fold(map_keys(table), m, (acc, k) => map_set(acc, k, map_get(table, k))))

test "actions registers a table in one update" {
    actions(#{ "t.a": (ev) => 1, "t.b": (ev) => 2 })
    let m = cell.get(mount_actions)
    assert_eq(map_has_key(m, "t.a") && map_has_key(m, "t.b"), true)
}

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

/// The events an input's `data-action` fires on. A text-like box fires
/// on Enter and on a button, never on `change` — leaving the first of
/// two boxes must not submit the form with the second still empty; a
/// control whose value IS the argument (select, checkbox, radio, date,
/// number, range, color, file) fires on `change`. `data-on="change enter
/// click"` names the events explicitly and overrides the default.
fn event_fires(ev, data) = {
    let ty = map_get(ev, "type")
    let on = if data == () => () else => map_get(data, "on")
    if on != () => {
        let wanted = " " + on + " "
        let name = if ty == "keydown" => "enter" else => ty
        str.contains(wanted, " " + name + " ")
    }
    else if ty == "change" && map_get(ev, "tag") == "input" => {
        let kind = map_get(ev, "input_type")
        !contains(["", "text", "search", "email", "url", "tel", "password"], if kind == () => "" else => kind)
    }
    else => true
}

test "a text box's change does not fire its action; a select's does; data-on decides" {
    let text_change = #{ "type": "change", "tag": "input", "input_type": "text", "data": #{ "action": "a" } }
    assert_eq(event_fires(text_change, map_get(text_change, "data")), false)
    let select_change = #{ "type": "change", "tag": "select", "input_type": (), "data": #{ "action": "a" } }
    assert_eq(event_fires(select_change, map_get(select_change, "data")), true)
    let checkbox = #{ "type": "change", "tag": "input", "input_type": "checkbox", "data": #{ "action": "a" } }
    assert_eq(event_fires(checkbox, map_get(checkbox, "data")), true)
    let opted = #{ "type": "change", "tag": "input", "input_type": "text", "data": #{ "action": "a", "on": "change enter" } }
    assert_eq(event_fires(opted, map_get(opted, "data")), true)
    let enter = #{ "type": "keydown", "tag": "input", "input_type": "text", "data": #{ "action": "a", "on": "change enter" } }
    assert_eq(event_fires(enter, map_get(enter, "data")), true)
    let click_off = #{ "type": "click", "tag": "button", "input_type": (), "data": #{ "action": "a", "on": "enter" } }
    assert_eq(event_fires(click_off, map_get(click_off, "data")), false)
}

fn dispatch_action(ev) = {
    let data = map_get(ev, "data")
    let raw_name = if data == () || click_on_form_control(ev) || is_repeat_of_enter(ev)
        || !event_fires(ev, data) => "" else => {
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
            // Repaint only if the handler changed state and nothing has
            // painted it yet: a handler that went through `apply` (or a
            // runtime over it) has repainted already, and painting again
            // doubled the cost of every click — the whole view built and
            // diffed twice. An unconditional rerender here would also wipe
            // focus and in-progress typing on every click into an
            // action-carrying form control. The states are compared only
            // when no repaint ran: two large stores are not walked for a
            // handler that already painted.
            // A change `apply` already judged — painted, or skipped because
            // no view reads what moved — is not judged again; only what the
            // handler changed after that is.
            let painted = cell.get(paints)
            let judgments = cell.get(judged)
            let before = current()
            let r = h(ev)
            if cell.get(judged) != judgments => {
                let seen = cell.get(judged_state)
                if current() != seen => repaint_for(seen, current()) else => ()
            }
            else if cell.get(paints) != painted => ()
            else if current() != before => repaint_for(before, current())
            else => ()
        }
    }
}

/// Repaint one subtree in place: render `node` into the element with
/// this id, leaving the rest of the page — and its focused input —
/// untouched. For the toast, the counter, the chart that should not
/// cost a whole-page render on every store write. `apply` remains the
/// whole-view repaint.
share fn patch(id, node) =
    if dom.available() => dom.patch(dom.query("#" + id), node) else => ()

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

test "one repaint per handled action: a handler that applies is not painted again, one that only updates is painted once, one that changes nothing is not painted" {
    let s0 = init(#{ "n": 0 })
    action("t.apply", (ev) => apply((s) => map_set(s, "n", map_get(s, "n") + 1)))
    action("t.update", (ev) => update((s) => map_set(s, "n", map_get(s, "n") + 1)))
    action("t.idle", (ev) => ())
    let click = (name) => #{ "type": "click", "tag": "button", "data": #{ "action": name } }
    let p0 = paint_count()
    dispatch_action(click("t.apply"))
    assert_eq((paint_count() - p0, map_get(current(), "n")), (1, 1))
    dispatch_action(click("t.update"))
    assert_eq((paint_count() - p0, map_get(current(), "n")), (2, 2))
    dispatch_action(click("t.idle"))
    assert_eq(paint_count() - p0, 2)
}

test "a change confined to unwatched keys does not repaint; watch narrows to the named keys" {
    let s0 = init(#{ "n": 0, "live": 0, "other": 0 })
    cell.set(watched, [])
    cell.set(unwatched, [])
    unwatch(["live"])
    let p0 = paint_count()
    let k0 = map_get(paint_stats(), "skipped")
    apply((s) => map_set(s, "live", 1))
    assert_eq((paint_count() - p0, map_get(paint_stats(), "skipped") - k0), (0, 1))
    apply((s) => map_set(s, "other", 1))
    assert_eq(paint_count() - p0, 1)
    watch(["n"])
    apply((s) => map_set(s, "other", 2))
    assert_eq(paint_count() - p0, 1)
    apply((s) => map_set(s, "n", 1))
    assert_eq(paint_count() - p0, 2)
    cell.set(watched, [])
    cell.set(unwatched, [])
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
        // A first paint that arrived with its state is adopted: the page
        // already shows exactly that state, so the first render would be
        // an interpreted pass whose diff changes nothing — 200 ms of a
        // large app's boot. When the caller's defaults add keys the
        // server did not render from, the render runs, since the page
        // may not show them.
        let adopted = if state_id != "" => {
            match json.parse(dom.get_text(dom.query("#" + state_id))) {
                Ok(server_state) => {
                    let merged = merge_state(initial, server_state)
                    init(merged)
                    adopts_first_paint(merged, server_state)
                },
                Err(e) => false
            }
        } else => false
        if adopted == false => rerender() else => ()
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

/// Whether the merged state is what the server rendered from — then the
/// first paint stands as it is. Compared key by key: the server's state
/// arrives as parsed JSON and the merged one is a map, and values of
/// different kinds never compare equal as wholes.
fn adopts_first_paint(merged, server_state) =
    contains(["Map", "JsonObject", "Object"], typeof(server_state))
        && contains(["Map", "JsonObject", "Object"], typeof(merged))
        && sort(map_keys(merged)) == sort(map_keys(server_state))
        && fold(map_keys(server_state), true,
                (ok, k) => ok && map_get(merged, k) == map_get(server_state, k))

test "the first paint is adopted only when the state is the server's" {
    let server = unwrap(json.parse("{\"notes\": [1], \"errors\": []}"))
    assert_eq(adopts_first_paint(merge_state(#{ "notes": [], "errors": [] }, server), server), true)
    assert_eq(adopts_first_paint(merge_state(#{ "notes": [], "theme": "dark" }, server), server), false)
}

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
