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
///
/// The children are kept as given. A pass that flattened and wrapped
/// them ran for every node of every repaint (7,450 calls over five
/// interactions of one application); the two readers of a tree — the
/// browser's patcher and `render` — flatten as they walk instead.
share fn el(tag, attrs, children) =
    #{ "tag": tag, "attrs": attrs, "children": children }

/// An element with no children — `img`, `br`, `hr`, `input`, or any tag
/// written through `el` that carries none: `void_el("img", #{ "src": s })`.
share fn void_el(tag, attrs) = el(tag, attrs, [])

/// `@when(cond, node)`: the node when `cond` holds, nothing otherwise —
/// and the node is built only then. A function `when(cond, node)` would
/// evaluate its argument first, raising on exactly the case it exists
/// to skip (`when(t != (), span([map_get(t, "id")]))` with `t` Unit);
/// the macro expands to the `if`, so the false branch costs nothing.
meta fn when(cond, node) = `if ${cond} => ${node} else => text("")`

/// A text node — always escaped at render. Non-strings stringify
/// (numbers, booleans); strings pass through as themselves.
share fn text(s) = #{ "text": as_text(s) }

fn as_text(v) = if typeof(v) == "String" => v else => to_string(v)

/// Trusted, pre-rendered markup — the explicit escaping opt-out.
share fn raw(s) = #{ "raw": s }

/// A subtree that is rebuilt only when its inputs change:
/// `memo("chart", [rows, width], () => chart(rows, width))`. The key
/// names the element (it becomes its `data-key`); the inputs are
/// compared by value against the last repaint's. While they stand, the
/// repaint hands the host a `keep` marker and the browser leaves that
/// subtree exactly as it is — the view function does not even run
/// `build`. Rendered to markup (the server, a static preview) it always
/// builds.
///
/// Whether the element is still on the page is the patcher's question,
/// not this function's: a `keep` it cannot honor comes back from
/// `dom.patch` as `missing`, `forget_memos` drops the stamp, and the
/// view paints once more. A memo stamped while its subtree was off the
/// page heals itself, and no repaint pays a selector scan per memo.
///
/// `inputs` of `()` means "always build": see `volatile`.
let memo_stamps = cell.new(#{})
// Which list a row's key belongs to, so forgetting a row forgets the
// list stamp that would otherwise answer `keeps` for it again.
let memo_row_lists = cell.new(#{})
let memo_tally = cell.new(#{ "hits": 0, "misses": 0 })

fn tally(which, by) =
    cell.update(memo_tally, (t) => map_set(t, which, map_get(t, which) + by))

/// Memo hits and misses since the last call, and reset — a repaint's
/// share, read by `view.rerender` into `paint_stats()`.
share fn take_memo_tally() = {
    let t = cell.get(memo_tally)
    cell.set(memo_tally, #{ "hits": 0, "misses": 0 })
    t
}

fn keyed(node, key) =
    if contains(["Map", "JsonObject"], typeof(node)) && map_has_key(node, "tag") && map_get(node, "tag") != "" =>
        map_set(node, "attrs", map_set(map_get(node, "attrs"), "data-key", key))
    else => node

share fn memo(key, inputs, build) =
    if inputs == () => volatile(key, build)
    else => {
        let stamp = show(inputs)
        let stamps = cell.get(memo_stamps)
        if dom.available() && map_has_key(stamps, key) && map_get(stamps, key) == stamp => {
            tally("hits", 1)
            #{ "keep": key }
        } else => {
            tally("misses", 1)
            cell.set(memo_stamps, map_set(stamps, key, stamp))
            keyed(build(), key)
        }
    }

/// A keyed subtree that is rebuilt on every repaint, and leaves no stamp
/// a later `memo` of the same key could match: "rebuild while this
/// holds" as a call — `if editing => volatile("row", build) else =>
/// memo("row", inputs, build)` — instead of a clock smuggled into the
/// inputs.
share fn volatile(key, build) = {
    tally("misses", 1)
    cell.update(memo_stamps, (m) => map_remove(m, key))
    keyed(build(), key)
}

/// A memoized list: `memo_list("issues", rows, (r) => r.id, (r) => [r,
/// selected == r.id], (r) => issue_row(r))` answers the row nodes, to
/// splice into any container. `key_of` names a row, `inputs_of` is what
/// the row is built from. One stamp covers the whole list — every row's
/// key and inputs — so a repaint that moved nothing costs one comparison
/// and hands the host one marker for all the rows; when something did
/// move, only the rows whose own inputs changed are rebuilt, and the
/// rest are kept where they stand (or moved, when the order changed).
share fn memo_list(key, items, key_of, inputs_of, row) = {
    let row_keys = map(items, (it) => key + ":" + as_text(key_of(it)))
    if !dom.available() =>
        map(range(0, len(items)), (i) => keyed(row(items[i]), row_keys[i]))
    else => {
        let row_stamps = map(items, (it) => show(inputs_of(it)))
        let stamp = show([row_keys, row_stamps])
        let stamps = cell.get(memo_stamps)
        if map_has_key(stamps, key) && map_get(stamps, key) == stamp => {
            tally("hits", len(items))
            [#{ "keeps": row_keys }]
        } else => {
            // Rows that left the list take their stamps with them.
            let before = map_get(stamps, key + "#rows")
            let gone = if before == () => [] else => filter(before, (k) => !contains(row_keys, k))
            let pruned = fold(gone, stamps, (m, k) => map_remove(m, k))
            let mut next = map_set(map_set(pruned, key, stamp), key + "#rows", row_keys)
            let mut lists = fold(gone, cell.get(memo_row_lists), (m, k) => map_remove(m, k))
            let mut out = []
            for i in range(0, len(items)) {
                let rk = row_keys[i]
                if map_has_key(stamps, rk) && map_get(stamps, rk) == row_stamps[i] => {
                    tally("hits", 1)
                    out = out + [#{ "keep": rk }]
                } else => {
                    tally("misses", 1)
                    next = map_set(next, rk, row_stamps[i])
                    lists = map_set(lists, rk, key)
                    out = out + [keyed(row(items[i]), rk)]
                }
            }
            cell.set(memo_stamps, next)
            cell.set(memo_row_lists, lists)
            out
        }
    }
}

/// Drop the stamps of these keys — what `dom.patch` answered as
/// `missing` — so the next repaint builds them. A row's key also drops
/// its list's stamp.
share fn forget_memos(keys) = {
    let lists = cell.get(memo_row_lists)
    cell.update(memo_stamps, (m) => fold(keys, m, (acc, k) => {
        let without = map_remove(acc, k)
        if map_has_key(lists, k) => map_remove(without, map_get(lists, k)) else => without
    }))
}

/// A fragment: children rendered with no wrapping element.
share fn fragment(children) =
    #{ "tag": "", "attrs": #{}, "children": children }

/// A node's children as a flat list of nodes: nested lists spliced,
/// strings wrapped as text, fragments opened. For code that inspects a
/// tree; `render` and the browser's patcher do this as they walk.
share fn children_of(node) = flatten_children(map_get(node, "children"))

fn flatten_children(children) = {
    let mut out = []
    for c in children {
        if typeof(c) == "List" => { out = out + flatten_children(c) }
        else if typeof(c) == "String" => { out = out + [text(c)] }
        else if contains(["Map", "JsonObject"], typeof(c)) && map_has_key(c, "tag") && map_get(c, "tag") == "" =>
            { out = out + flatten_children(map_get(c, "children")) }
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
    else if typeof(node) == "List" => node |> map((c) => render(c)) |> join("")
    else if map_has_key(node, "text") => escape(map_get(node, "text"))
    else if map_has_key(node, "raw") => map_get(node, "raw")
    // A memo's keep marker only exists for the browser's patch; markup
    // has nothing to keep.
    else if map_has_key(node, "keep") || map_has_key(node, "keeps") => ""
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

test "void_el renders an element with no children; @when builds its node lazily" {
    assert_eq(render(void_el("img", #{ "src": "a.png" })), "<img src=\"a.png\">")
    let t = ()
    assert_eq(render(@when(t != (), span(#{}, [map_get(t, "id")]))), "")
    let u = #{ "id": "OT-1" }
    assert_eq(render(@when(u != (), span(#{}, [map_get(u, "id")]))), "<span>OT-1</span>")
}

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

test "memo builds where there is no dom, and stamps its key on the element" {
    let n = memo("panel", [1, "a"], () => div(#{ "class": "p" }, ["x"]))
    assert_eq(render(n), "<div class=\"p\" data-key=\"panel\">x</div>")
    // Same inputs again: still built here (no page to keep it on).
    assert_eq(render(memo("panel", [1, "a"], () => div(#{}, ["y"]))), "<div data-key=\"panel\">y</div>")
    assert_eq(render(#{ "keep": "panel" }), "")
}

test "children stay nested and render the same; children_of flattens for a reader" {
    let node = ul(#{}, [[li(#{}, ["a"]), [li(#{}, ["b"])]], fragment(["c"])])
    assert_eq(render(node), "<ul><li>a</li><li>b</li>c</ul>")
    assert_eq(len(map_get(node, "children")), 2)
    assert_eq(len(children_of(node)), 3)
}

test "volatile always builds and leaves no stamp; memo with () inputs is volatile" {
    let a = volatile("v", () => div(#{}, ["1"]))
    assert_eq(render(a), "<div data-key=\"v\">1</div>")
    assert_eq(map_has_key(cell.get(memo_stamps), "v"), false)
    assert_eq(render(memo("v", (), () => div(#{}, ["2"]))), "<div data-key=\"v\">2</div>")
    assert_eq(map_has_key(cell.get(memo_stamps), "v"), false)
}

test "memo_list keys its rows and builds every one where there is no dom" {
    let rows = [#{ "id": 1, "t": "a" }, #{ "id": 2, "t": "b" }]
    let nodes = memo_list("rows", rows, (r) => map_get(r, "id"), (r) => r, (r) => li(#{}, [map_get(r, "t")]))
    assert_eq(render(ul(#{}, nodes)), "<ul><li data-key=\"rows:1\">a</li><li data-key=\"rows:2\">b</li></ul>")
}

test "forget_memos drops a stamp, and a row's key drops its list's" {
    cell.set(memo_stamps, #{ "panel": "s", "rows": "l", "rows:1": "r" })
    cell.set(memo_row_lists, #{ "rows:1": "rows" })
    forget_memos(["panel", "rows:1"])
    assert_eq(map_keys(cell.get(memo_stamps)), [])
}

test "attribute order is deterministic" {
    assert_eq(render(div(#{ "b": "2", "a": "1" }, [])), "<div a=\"1\" b=\"2\"></div>")
}
