// Walk the node tree against a context value, producing the output string.
// A context is any indexable value — a map, an anonymous object, or a parsed
// JSON object — and dotted paths (`user.name`) descend through them with the
// same `map_get` accessor, so JSON and native objects render identically.

use lib.ast { Node, NText, NVar, NEach, NIf }

// Can we look a key up inside this value?
share fn is_indexable(v) = {
    let t = typeof(v)
    (t == "Object") || (t == "JsonObject") || (t == "Map")
}

// Render a leaf value: strings stay bare (no quotes), a missing value is empty.
share fn show(v) = if typeof(v) == "String" => v
    else if typeof(v) == "Unit" => ""
    else => to_string(v)

// Is this value "truthy" for an {{#if}} section?
share fn truthy(v) = {
    let t = typeof(v)
    if t == "Bool" => v
    else if t == "List" => len(v) > 0
    else if t == "String" => str.length(v) > 0
    else if t == "Unit" => false
    else => true
}

// Descend a dotted path from `cur`, bailing to "" if a segment isn't indexable.
share fn walk(cur, segs, i) = if i >= len(segs) => cur
    else if is_indexable(cur) => walk(map_get(cur, segs[i]), segs, i + 1)
    else => ""

// Look up a path in the context. "." / "this" resolve to the context itself,
// which is how an {{#each}} body refers to the current scalar item.
share fn lookup(ctx, path) = if (path == ".") || (path == "this") => ctx
    else => walk(ctx, str.split(path, "."), 0)

share fn render_node(node, ctx) = match node {
    NText(s) => s,
    NVar(p) => show(lookup(ctx, p)),
    NEach(p, body) => {
        let items = lookup(ctx, p)
        if typeof(items) == "List" =>
            items |> fold("", (acc, item) => acc + render(body, item))
        else => ""
    },
    NIf(p, body) => if truthy(lookup(ctx, p)) => render(body, ctx) else => ""
}

share fn render(nodes, ctx) = {
    let mut out = ""
    for node in nodes {
        out = out + render_node(node, ctx)
    }
    out
}
