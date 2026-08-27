//! html — views as plain data, rendered to markup.
//!
//! An HTML tree is maps and lists, nothing more: an element is
//! `#{ "tag", "attrs", "children" }`, text is `#{ "text": s }`, and
//! trusted markup is `#{ "raw": s }`. Because a view is a value, a
//! framework built on this SDK can transform, inspect, or diff it —
//! the Open AST philosophy applied to markup.
//!
//! Rendering escapes by default: text nodes and attribute values are
//! HTML-escaped, so untrusted input renders as text. `raw` is the one
//! explicit opt-out.

/// An element node: `el("div", #{ "class": "card" }, [text("hi")])`.
/// Children may be nodes, strings (auto-wrapped as text), or lists
/// (flattened) — so `map(...)` results splice in directly.
share fn el(tag, attrs, children) =
    #{ "tag": tag, "attrs": attrs, "children": flatten_children(children) }

/// A text node — always escaped at render. Non-strings stringify
/// (numbers, booleans); strings pass through as themselves.
share fn text(s) = #{ "text": as_text(s) }

fn as_text(v) = if typeof(v) == "String" => v else => to_string(v)

/// Trusted, pre-rendered markup — the explicit escaping opt-out.
share fn raw(s) = #{ "raw": s }

/// A fragment: children rendered with no wrapping element.
share fn fragment(children) =
    #{ "tag": "", "attrs": #{}, "children": flatten_children(children) }

fn flatten_children(children) = {
    let mut out = []
    for c in children {
        if typeof(c) == "List" => { out = out + flatten_children(c) }
        else if typeof(c) == "String" => { out = out + [text(c)] }
        else => { out = out + [c] }
    }
    out
}

// ── the common tags, so views read as structure, not strings ─────────

share fn div(attrs, children) = el("div", attrs, children)
share fn span(attrs, children) = el("span", attrs, children)
share fn p(attrs, children) = el("p", attrs, children)
share fn h1(attrs, children) = el("h1", attrs, children)
share fn h2(attrs, children) = el("h2", attrs, children)
share fn h3(attrs, children) = el("h3", attrs, children)
share fn ul(attrs, children) = el("ul", attrs, children)
share fn ol_(attrs, children) = el("ol", attrs, children)
share fn li(attrs, children) = el("li", attrs, children)
share fn table(attrs, children) = el("table", attrs, children)
share fn thead(attrs, children) = el("thead", attrs, children)
share fn tbody(attrs, children) = el("tbody", attrs, children)
share fn tr(attrs, children) = el("tr", attrs, children)
share fn th(attrs, children) = el("th", attrs, children)
share fn td(attrs, children) = el("td", attrs, children)
share fn form(attrs, children) = el("form", attrs, children)
share fn label(attrs, children) = el("label", attrs, children)
share fn button(attrs, children) = el("button", attrs, children)
share fn a(attrs, children) = el("a", attrs, children)
share fn section(attrs, children) = el("section", attrs, children)
share fn header(attrs, children) = el("header", attrs, children)
share fn footer(attrs, children) = el("footer", attrs, children)
share fn main_(attrs, children) = el("main", attrs, children)
share fn nav(attrs, children) = el("nav", attrs, children)
share fn pre(attrs, children) = el("pre", attrs, children)
share fn code(attrs, children) = el("code", attrs, children)
share fn strong(attrs, children) = el("strong", attrs, children)
share fn em(attrs, children) = el("em", attrs, children)

/// A void element — no children by construction: `input(#{ ... })`.
share fn input(attrs) = el("input", attrs, [])
share fn img(attrs) = el("img", attrs, [])
share fn br() = el("br", #{}, [])
share fn hr() = el("hr", #{}, [])
share fn select(attrs, children) = el("select", attrs, children)
share fn option(attrs, children) = el("option", attrs, children)
share fn textarea(attrs, children) = el("textarea", attrs, children)

// ── rendering ────────────────────────────────────────────────────────

/// Escape text for HTML element content.
share fn escape(s) =
    str.replace(str.replace(str.replace(as_text(s), "&", "&amp;"), "<", "&lt;"), ">", "&gt;")

/// Escape a value for a double-quoted attribute.
share fn escape_attr(s) = str.replace(escape(s), "\"", "&quot;")

fn is_void(tag) =
    contains(["input", "img", "br", "hr", "meta", "link", "source", "area", "base", "col", "embed", "track", "wbr"], tag)

fn render_attrs(attrs) = {
    let mut out = ""
    for k in sort(map_keys(attrs)) {
        let v = map_get(attrs, k)
        // `#{ "disabled": true }` renders as a bare attribute; false
        // and Unit render as absent.
        let t = typeof(v)
        if t == "Bool" => { if v => { out = out + " " + k } }
        else if t != "Unit" => {
            out = out + " " + k + "=\"" + escape_attr(v) + "\""
        }
    }
    out
}

/// A node tree as markup — the isomorphic half: the same view data
/// renders on the server (pages, emails, static sites) and in the
/// browser (through web.view's mount).
share fn render(node) = {
    if typeof(node) == "String" => escape(node)
    else if map_has_key(node, "text") => escape(map_get(node, "text"))
    else if map_has_key(node, "raw") => map_get(node, "raw")
    else => {
        let tag = map_get(node, "tag")
        let inner = map_get(node, "children") |> map((c) => render(c)) |> join("")
        if tag == "" => inner
        else if is_void(tag) => "<" + tag + render_attrs(map_get(node, "attrs")) + ">"
        else => "<" + tag + render_attrs(map_get(node, "attrs")) + ">" + inner + "</" + tag + ">"
    }
}

/// A complete document: `page(title, head_extra, body_node)` — the
/// shell `web.app` serves, and a one-call static-site generator.
share fn page(title, head_extra, body) =
    "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">"
    + "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
    + "<title>" + escape(title) + "</title>" + head_extra
    + "</head><body>" + render(body) + "</body></html>"

test "elements render with escaped text and attrs" {
    assert_eq(render(div(#{ "class": "card" }, ["hi"])), "<div class=\"card\">hi</div>")
    assert_eq(render(text("<b>&")), "&lt;b&gt;&amp;")
    assert_eq(render(div(#{ "title": "a\"b" }, [])), "<div title=\"a&quot;b\"></div>")
    assert_eq(render(raw("<b>bold</b>")), "<b>bold</b>")
}

test "children flatten, strings wrap, lists splice" {
    let items = map([1, 2], (n) => li(#{}, [to_string(n)]))
    assert_eq(render(ul(#{}, [items])), "<ul><li>1</li><li>2</li></ul>")
    assert_eq(render(fragment(["a", span(#{}, ["b"])])), "a<span>b</span>")
}

test "void elements, boolean and absent attributes" {
    assert_eq(render(input(#{ "type": "text", "disabled": true })),
        "<input disabled type=\"text\">")
    assert_eq(render(input(#{ "value": () })), "<input>")
    assert_eq(render(br()), "<br>")
}

test "attribute order is deterministic" {
    assert_eq(render(div(#{ "b": "2", "a": "1" }, [])), "<div a=\"1\" b=\"2\"></div>")
}
