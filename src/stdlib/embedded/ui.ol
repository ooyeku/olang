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
        let tag = node.tag
        let mut out = "<" + tag
        for (name, value) in entries(node.attrs) {
        out = out + " " + name + "=\"" + esc(value) + "\""
        }
        if is_void(tag) => out + " />"
        else => {
            let mut inner = ""
            for child in node.children {
            inner = inner + html(child)
            }
            out + ">" + inner + "</" + tag + ">"
        }
    }
}

// ── controls a host binds ──────────────────────────────────────────────
// A control is a node like any other — `html` renders a real HTML
// control — that also says its kind (`control`) and its spec. Each
// `ui.slider` and its kin validates, then answers `runtime.bind(node,
// value)`: the value itself with no host, and in a notebook's kernel the
// value an editor set for the cell's control, while it still fits.

fn is_num(v) = typeof(v) == "Int" || typeof(v) == "Float"

fn label_of(v) = if typeof(v) == "String" => v else => show(v)

share fn control(kind, value, spec) = {
    let node = if kind == "slider" => {
        let mut attrs = #{ "type": "range", "min": spec.min, "max": spec.max, "value": value }
        if map_has_key(spec, "step") && spec.step != () => { attrs = map_set(attrs, "step", spec.step) }
        h("input", attrs, [])
    } else if kind == "select" => h("select", #{}, map(spec.options, (o) =>
        h("option", if o == value => #{ "value": label_of(o), "selected": "selected" }
            else => #{ "value": label_of(o) }, [label_of(o)])))
    else if kind == "date" => h("input", #{ "type": "date", "value": value }, [])
    else if kind == "text" => h("input", #{ "type": "text", "value": value }, [])
    else if kind == "toggle" => h("input",
        if value == true => #{ "type": "checkbox", "checked": "checked" }
        else => #{ "type": "checkbox" }, [])
    else if kind == "brush" => h("div",
        if value == () => #{ "class": "brush" }
        else => #{ "class": "brush", "data-x0": value[0], "data-x1": value[1] }, [])
    else => raise("ui.control: no control is a " + label_of(kind) +
        " (slider, select, date, text, toggle, brush)")
    map_set(map_set(node, "control", kind), "spec", spec)
}

/// A number chosen between `min` and `max` (by `step`, when given): an
/// Int slider when the value and both bounds are Ints.
share fn slider(value, min, max, step = ()) = {
    if !is_num(value) || !is_num(min) || !is_num(max) =>
        raise("ui.slider: the value, min and max are numbers")
    if min > max => raise("ui.slider: min " + show(min) + " is above max " + show(max))
    if value < min || value > max =>
        raise("ui.slider: " + show(value) + " is outside [" + show(min) + ", " + show(max) + "]")
    if step != () && (!is_num(step) || step <= 0) => raise("ui.slider: step is a positive number")
    runtime.bind(control("slider", value, #{ "min": min, "max": max, "step": step }), value)
}

/// One of `options` (a non-empty list).
share fn select(value, options) = {
    if typeof(options) != "List" || len(options) == 0 => raise("ui.select: options is a non-empty list")
    if !contains(options, value) =>
        raise("ui.select: " + show(value) + " is not one of the options")
    runtime.bind(control("select", value, #{ "options": options }), value)
}

fn date_like(s) = typeof(s) == "String" && len(s) == 10 &&
    str.substring(s, 4, 5) == "-" && str.substring(s, 7, 8) == "-" &&
    is_ok(str.parse_int(str.substring(s, 0, 4))) && is_ok(str.parse_int(str.substring(s, 5, 7))) &&
    is_ok(str.parse_int(str.substring(s, 8, 10)))

/// A day, as a "YYYY-MM-DD" String.
share fn date(value) = {
    if !date_like(value) => raise("ui.date: the value is a \"YYYY-MM-DD\" String, got " + show(value))
    runtime.bind(control("date", value, #{}), value)
}

/// A line of text.
share fn text(value) = {
    if typeof(value) != "String" => raise("ui.text: the value is a String, got " + typeof(value))
    runtime.bind(control("text", value, #{}), value)
}

/// On or off.
share fn toggle(value) = {
    if typeof(value) != "Bool" => raise("ui.toggle: the value is a Bool, got " + typeof(value))
    runtime.bind(control("toggle", value, #{}), value)
}

/// A range of a chart's x (`chart` a plot chart's SVG): `()` for none,
/// or `[x0, x1]`.
share fn brush(chart, value = ()) = {
    if typeof(chart) != "String" || is_err(plot.describe(chart)) =>
        raise("ui.brush: the chart is a plot chart's SVG (one that carries its description)")
    if value != () && !(typeof(value) == "List" && len(value) == 2 && is_num(value[0]) &&
        is_num(value[1]) && value[0] <= value[1]) =>
        raise("ui.brush: the value is () or [x0, x1], x0 <= x1")
    let range = if value == () => () else => [to_float(value[0]), to_float(value[1])]
    runtime.bind(control("brush", range, #{ "chart": chart }), range)
}

// ── mounting with keyed reconciliation ─────────────────────────────────

fn wrapper_id(mount, key) = "ui-" + mount + "-" + key

fn child_key(child, index) = {
    if typeof(child) == "String" => "i" + show(index)
    else => {
        let k = child.key
        if k == "" => "i" + show(index) else => k
    }
}

// Render a keyed list of children into `el` (which must carry an id).
// Keys should be short id-safe tokens; unkeyed children key by index.
share fn render(el, children) = {
    let mount = dom.get_attr(el, "id")
    let prev = unwrap_or(json.parse(dom.get_attr(el, "data-ui")),
        #{ "order": [], "html": #{} })
    let prev_order = prev.order
    let prev_html = prev.html

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
