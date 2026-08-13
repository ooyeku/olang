// ui — declarative views for the dom module, written in olang and
// bundled into the binary as a builtin package (`use ui`).
//
// A view is a tree of plain values: `h(tag, attrs, children)` builds a
// node map, strings are text nodes (escaped on render), and `hk` gives
// a node a reconciliation key. `html(node)` renders a tree to an HTML
// string — pure, testable anywhere. `render(el, children)` mounts a
// keyed list of children into an element and reconciles against the
// previous render: unchanged children are NOT touched (input state and
// focus survive), changed ones re-render in place, added/removed keys
// insert and remove surgically, and reorders reposition without
// rebuilding. The memo travels in a `data-ui` attribute on the mount
// element — the same DOM-resident-state discipline as everything else
// in the browser story.

// ── building trees ─────────────────────────────────────────────────────

share fn h(tag, attrs, children) = #{
    "tag": tag, "attrs": attrs, "children": children, "key": ""
}

share fn hk(key, tag, attrs, children) = #{
    "tag": tag, "attrs": attrs, "children": children, "key": key
}

// ── rendering to HTML (pure) ───────────────────────────────────────────

share fn esc(s) = {
    let raw = if typeof(s) == "String" => s else => to_string(s)
    str.replace(str.replace(str.replace(str.replace(raw,
        "&", "&amp;"), "<", "&lt;"), ">", "&gt;"), "\"", "&quot;")
}

fn is_void(tag) =
    contains(["br", "hr", "img", "input", "meta", "link", "area", "col"], tag)

share fn html(node) = {
    if typeof(node) == "String" => esc(node)
    else => {
        let tag = map_get(node, "tag")
        let mut out = "<" + tag
        for (name, value) in entries(map_get(node, "attrs")) {
        out = out + " " + name + "=\"" + esc(value) + "\""
        }
        if is_void(tag) => out + " />"
        else => {
            let mut inner = ""
            for child in map_get(node, "children") {
            inner = inner + html(child)
            }
            out + ">" + inner + "</" + tag + ">"
        }
    }
}

// ── mounting with keyed reconciliation ─────────────────────────────────

fn wrapper_id(mount, key) = "ui-" + mount + "-" + key

fn child_key(child, index) = {
    if typeof(child) == "String" => "i" + show(index)
    else => {
        let k = map_get(child, "key")
        if k == "" => "i" + show(index) else => k
    }
}

// Render a keyed list of children into `el` (which must carry an id).
// Keys should be short id-safe tokens; unkeyed children key by index.
share fn render(el, children) = {
    let mount = dom.get_attr(el, "id")
    let prev = unwrap_or(json.parse(dom.get_attr(el, "data-ui")),
        #{ "order": [], "html": #{} })
    let prev_order = map_get(prev, "order")
    let prev_html = map_get(prev, "html")

    let mut order = []
    let mut html_map = #{}
    let mut index = 0
    for child in children {
        let k = child_key(child, index)
        order = order + [k]
        html_map = map_set(html_map, k, html(child))
        index = index + 1
    }

    // Removals first, so reordering only deals with survivors.
    for k in prev_order {
        if map_has_key(html_map, k) == false => {
            dom.remove(dom.query("#" + wrapper_id(mount, k)))
        }
    }

    // Updates in place; additions append (positioned below).
    for k in order {
        let target = map_get(html_map, k)
        if map_has_key(prev_html, k) => {
            if map_get(prev_html, k) != target => {
                dom.set_html(dom.query("#" + wrapper_id(mount, k)), target)
            }
        } else => {
            let w = dom.create("div")
            dom.set_attr(w, "id", wrapper_id(mount, k))
            dom.set_html(w, target)
            dom.append(el, w)
        }
    }

    // Reposition only when the surviving order actually changed — the
    // common re-render (same keys, edited content) moves nothing.
    let survivors = filter(prev_order, (k) => map_has_key(html_map, k))
    let added = filter(order, (k) => map_has_key(prev_html, k) == false)
    if join(concat(survivors, added), ",") != join(order, ",") => {
        let mut anchor = 0
        let mut i = len(order) - 1
        while i >= 0 {
            let w = dom.query("#" + wrapper_id(mount, order[i]))
            dom.insert_before(el, w, anchor)
            anchor = w
            i = i - 1
        }
    }

    dom.set_attr(el, "data-ui",
        unwrap(json.stringify(#{ "order": order, "html": html_map })))
}
