//! Rewrites to the idioms of docs/idioms.md, exactly, file by file:
//!
//!     olang tools/idioms/fix.ol records|unwrap [--check] <file or folder>...
//!
//! `records`: `#{ "kind": "x", "n": 1 }` becomes `{ kind: "x", n: 1 }`
//! where every key is a literal name. Left as maps:
//! - an environment (all keys in capitals), because `os.exec` takes a map;
//! - a spread's own entries (`#{ ...m, "k": v }`), because a spread keeps
//!   the kind of what it copies;
//! - the value of a `let mut` and the start value of a `fold`, because a
//!   map grown by `m = map_set(m, k, v)` is updated in place and a record
//!   is not (docs/idioms.md §1, performance);
//! - a line saying `// allow: rule_record_as_map`.
//!
//! `unwrap`: `match r { Ok(v) => v, Err(e) => <literal> }` becomes
//! `unwrap_or(r, <literal>)`. The default is a literal, so evaluating it
//! eagerly changes nothing.
//!
//! A file is written only if the rewrite parses. `--check` writes nothing
//! and lists what would change. Exit 1 when `--check` finds something.
//! A record is not a map to `typeof` (`"Object"`, not `"Map"`) and prints
//! as `{ k: v }`: code that tests `typeof(x) == "Map"` or compares printed
//! text has to be made record-aware first, and the tests say where.

let IDENT_RE = "^[A-Za-z_][A-Za-z0-9_]*$"
let ENV_RE = "^[A-Z][A-Z0-9_]*$"
// language.md's reserved words: a record's key may be one only quoted.
let RESERVED = ["fn", "let", "if", "else", "match", "for", "while", "loop", "break", "continue", "return",
    "true", "false", "struct", "enum", "await", "yield", "async", "defer", "const", "pub"]
let SKIP = ["/fixtures/", "/.build/", "/target/", "/dist/", "/shelf/", "/node_modules/"]

fn matches(pat, s) = unwrap_or(re.is_match(pat, s), false)

fn is_node(v) = typeof(v) == "Map" && v.kind != ()

/// Every node under `v`, depth first, with the node that holds it:
/// `[(node, parent)]`. A parent is () at the top.
fn walk(v, parent, out) = {
    let mut acc = out
    if is_node(v) => { acc = acc + [(v, parent)] }
    let here = if is_node(v) => v else => parent
    let kids = if typeof(v) == "Map" => map_values(v) else if typeof(v) == "List" => v else => []
    for k in kids {
        if typeof(k) == "Map" || typeof(k) == "List" => { acc = walk(k, here, acc) }
    }
    acc
}

fn line_at(src, byte) = len(filter(str.line_starts(src), (s) => s <= byte))

fn allowed(lines, src, byte, rule) = {
    let l = line_at(src, byte)
    l >= 1 && l <= len(lines) && str.contains(lines[l - 1], "allow: " + rule)
}

// ── records ───────────────────────────────────────────────────────────

fn name_key(e) = e.key.kind == "str" && matches(IDENT_RE, e.key.value)

fn env_key(e) = matches(ENV_RE, e.key.value)

// A map literal that grows in place, or that is a spread's own part.
fn held_as_map(node, parent) = {
    if parent == () => return false
    if parent.kind == "spread_literal" => return true
    if parent.kind == "let" && parent.mutable == true => return true
    if parent.kind == "call" && parent.target == "fold" && len(parent.args) >= 2 && parent.args[1].start == node.start => return true
    false
}

fn record_edits(src, nodes) = {
    let lines = str.lines(src)
    let mut edits = []
    for (n, parent) in nodes {
        if n.kind == "map" && n.entry_count > 0 && all(n.entries, name_key) && !all(n.entries, env_key)
            && !held_as_map(n, parent) && !allowed(lines, src, n.start, "rule_record_as_map")
            && str.slice_bytes(src, n.start, n.start + 2) == "#{" => {
            edits = edits + [(n.start, n.start + 2, "{")]
            // a reserved word stays quoted: `{ "fn": f, tag: t }`
            for e in filter(n.entries, (e) => !contains(RESERVED, e.key.value)) {
                edits = edits + [(e.key.start, e.key.end, e.key.value)]
            }
        }
    }
    edits
}

// ── unwrap_or ─────────────────────────────────────────────────────────

let LITERALS = ["str", "int", "float", "bool", "unit", "list", "map", "object"]

fn literal(b) = contains(LITERALS, b.kind) && b.start != () && b.end != ()

fn unwrap_edits(src, nodes) = {
    let lines = str.lines(src)
    let mut edits = []
    for (n, parent) in nodes {
        if n.kind == "match" && n.arm_count == 2 && n.start != () && n.end != () => {
            let (a, b) = (n.arms[0], n.arms[1])
            let ok_id = a.pattern.form == "ok" && a.pattern.inner.form == "ident" && a.body.kind == "ident" && a.body.name == a.pattern.inner.name
            if ok_id && b.pattern.form == "err" && literal(b.body) && !allowed(lines, src, n.start, "rule_swallowed_err") => {
                let subject = str.slice_bytes(src, n.value.start, n.value.end)
                let default = str.slice_bytes(src, b.body.start, b.body.end)
                edits = edits + [(n.start, n.end, "unwrap_or(" + subject + ", " + default + ")")]
            }
        }
    }
    edits
}

// ── applying ──────────────────────────────────────────────────────────

// Edits as (start, end, text) over byte offsets, none overlapping.
fn apply(src, edits) = {
    let sorted = col.sort_by(edits, (e) => e[0])
    let mut out = ""
    let mut at = 0
    for (s, e, text) in sorted {
        if s < at => return Err("overlapping edits at byte " + to_string(s))
        out = out + str.slice_bytes(src, at, s) + text
        at = e
    }
    Ok(out + str.slice_bytes(src, at, str.byte_len(src)))
}

fn fix_text(pass, src) = {
    let tree = meta.parse(src)?
    let nodes = walk(tree, (), [])
    let edits = if pass == "records" => record_edits(src, nodes) else => unwrap_edits(src, nodes)
    if len(edits) == 0 => return Ok((src, 0))
    let out = apply(src, edits)?
    match meta.parse(out) {
        Ok(_) => Ok((out, if pass == "records" => len(filter(edits, (e) => e[2] == "{")) else => len(edits))),
        Err(e) => Err("the rewrite does not parse: " + show(e)),
    }
}

fn files_of(path) = {
    if !fs.is_dir(path) => return [path]
    unwrap_or(fs.glob(fs.join(path, "**/*.ol")), [])
        |> filter((p) => !any(SKIP, (s) => str.contains(p, s)))
        |> sort
}

fn run(pass, check, paths) = {
    let mut changed = 0
    let mut failed = 0
    for f in flatten(map(paths, files_of)) {
        let src = unwrap_or(fs.read_file(f), "")
        match fix_text(pass, src) {
            Err(e) => {
                println(`${f}: left as it was (${e})`)
                failed = failed + 1
            },
            Ok((out, n)) => if n > 0 => {
                changed = changed + 1
                println(`${f}: ${n} ${if check => "would change" else => "changed"}`)
                if !check => { unwrap(fs.write_file(f, out)) }
            },
        }
    }
    println(`${pass}: ${changed} files ${if check => "would change" else => "changed"}, ${failed} left`)
    if check && changed > 0 => 1 else if failed > 0 => 2 else => 0
}

fn main(args) = {
    let rest = skip(args, 1)
    if len(rest) < 2 || !contains(["records", "unwrap"], rest[0]) => {
        println("usage: olang tools/idioms/fix.ol records|unwrap [--check] <file or folder>...")
        return 2
    }
    let check = contains(rest, "--check")
    run(rest[0], check, filter(skip(rest, 1), (a) => a != "--check"))
}

os.exit(main(os.args()))
