//! The idioms of docs/idioms.md that a machine can see, as `olang check`
//! rules over the flattened AST. Each finding names the rule and the
//! form to write instead. A line that means it says so with
//! `// allow: rule_<name>`.
//!
//! Every rule here is one pass over the nodes: a project of 50,000 lines
//! is about a million nodes.
//!
//! The existing code has thousands of these, so the rules are run through
//! tools/idioms/ratchet.ol, which holds each file to its count. They are
//! not meant for `[check] rules` until a project is at zero.

let IDENT_RE = "^[A-Za-z_][A-Za-z0-9_]*$"

fn is_name_key(k) = k.kind == "str" && unwrap_or(re.is_match(IDENT_RE, k.value), false)

// `#{ "NO_COLOR": "1" }`: environment names are data, and `os.exec`'s
// `env` takes a map, so a literal whose keys are all capitals stays one.
fn is_env_key(k) = unwrap_or(re.is_match("^[A-Z][A-Z0-9_]*$", k.value), false)

fn is_map_set(n) = n.kind == "call" && n.target == "map_set" && len(n.args) == 3

fn key_of(call) = call.args[1]

/// `map_set(map_set(r, "a", x), "b", y)` with literal name keys: write
/// one spread. Reported once, at the outermost call of a chain.
share fn rule_map_set_chain(nodes) = {
    let mut inner = #{}
    for n in nodes {
        if is_map_set(n) && is_map_set(n.args[0]) && is_name_key(key_of(n)) && is_name_key(key_of(n.args[0])) => {
            inner = map_set(inner, to_string(n.args[0].start), true)
        }
    }
    let mut out = []
    for n in nodes {
        if is_map_set(n) && is_map_set(n.args[0]) && is_name_key(key_of(n)) && is_name_key(key_of(n.args[0]))
            && inner[to_string(n.start)] != true => {
            out = out + [{
                line: n.line,
                message: "nested map_set with fixed keys: write one spread, `#{ ...m, \"a\": x, \"b\": y }` over a map or `{ ...r, a: x, b: y }` over a record",
            }]
        }
    }
    out
}

/// `#{ "kind": "x", "n": 1 }` where every key is a literal name: a record
/// `{ kind: "x", n: 1 }`. A map whose keys are data, or that a hot loop
/// grows with `m = map_set(m, k, v)`, stays a map (allow it); one whose
/// keys are all capitals (an environment) is not reported.
share fn rule_record_as_map(nodes) = {
    // a spread's own entries (`#{ ...m, "k": v }`) stay as they are: the
    // spread keeps the kind of what it copies (docs/idioms.md §1)
    let mut spread_at = #{}
    for n in nodes {
        if n.kind == "spread_literal" => { spread_at = map_set(spread_at, to_string(n.start), true) }
    }
    let mut out = []
    for n in nodes {
        if n.kind == "map" && n.entry_count > 0 && spread_at[to_string(n.start)] != true
            && all(n.entries, (e) => is_name_key(e.key)) && !all(n.entries, (e) => is_env_key(e.key)) => {
            out = out + [{ line: n.line, message: "a map literal with fixed name keys: write a record `{ k: v }`" }]
        }
    }
    out
}

// The subject an `if` condition tests against a string: `p.kind == "a"`
// gives "p.kind", anything else ().
fn tested(c) = {
    if c == () || c.kind != "binop" || c.op != "==" => return ()
    let (l, r) = (c.left, c.right)
    let side = if r.kind == "str" => l else if l.kind == "str" => r else => ()
    if side == () || side.kind != "field" || side.object.kind != "ident" => return ()
    side.object.name + "." + side.field
}

/// `if p.kind == "a" => … else if p.kind == "b" => … else if p.kind == "c"`:
/// three or more tests of one field against strings. Write a `match`
/// (record patterns or an enum). Reported once, at the head.
share fn rule_kind_chain(nodes) = {
    let mut tails = #{}
    let mut out = []
    for n in nodes {
        if n.kind == "if" && tails[to_string(n.start)] != true => {
            let subject = tested(n.condition)
            if subject != () => {
                let mut count = 1
                let mut cur = n.else
                while cur != () && cur.kind == "if" && tested(cur.condition) == subject {
                    tails = map_set(tails, to_string(cur.start), true)
                    count = count + 1
                    cur = cur.else
                }
                if count >= 3 => {
                    out = out + [{ line: n.line, message: `${count} tests of ${subject} in an if chain: write a match` }]
                }
            }
        }
    }
    out
}

let LITERAL_KINDS = ["str", "int", "float", "bool", "unit", "list", "map"]

// A literal all the way down: `#{ "error": show(e) }` uses the error.
fn all_literal(v) = if typeof(v) == "List" => all(v, all_literal)
    else if typeof(v) != "Map" => true
    else => (v.kind == () || contains(LITERAL_KINDS, v.kind)) && all(map_values(v), all_literal)

fn is_literal(b) = contains(LITERAL_KINDS, b.kind) && all_literal(b)

/// `match r { Ok(v) => v, Err(e) => "" }`: the error dropped for a
/// default. Write `unwrap_or(r, "")`, or let it rise with `?`.
share fn rule_swallowed_err(nodes) = {
    let mut out = []
    for n in nodes {
        if n.kind == "match" && n.arm_count == 2 => {
            let (a, b) = (n.arms[0], n.arms[1])
            let ok_id = a.pattern.form == "ok" && a.pattern.inner.form == "ident" && a.body.kind == "ident" && a.body.name == a.pattern.inner.name
            if ok_id && b.pattern.form == "err" && is_literal(b.body) => {
                out = out + [{ line: n.line, message: "match that drops the Err for a default: write unwrap_or(r, default), or propagate with ?" }]
            }
        }
    }
    out
}

let FN_MAX_LINES = 80

/// A function longer than 80 lines: split it into named steps (an update
/// function routes to handlers).
share fn rule_long_fn(nodes) = {
    let k = len(nodes)
    let mut out = []
    for (i, n) in enumerate(nodes) {
        if n.kind == "fn" => {
            let end = n.body.end ?? n.end ?? n.start
            let mut last_line = n.line
            let mut j = i + 1
            // a pattern node has no span of its own: it is inside whatever
            // came before it, so the walk only stops at a node that starts
            // past the function
            while j < k && (nodes[j].start ?? n.start) < end {
                last_line = max(last_line, nodes[j].line ?? last_line)
                j = j + 1
            }
            let size = last_line - n.line + 1
            if size > FN_MAX_LINES => {
                out = out + [{ line: n.line, message: `${n.name} is ${size} lines: split it into named functions (at most ${FN_MAX_LINES})` }]
            }
        }
    }
    out
}
