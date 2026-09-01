//! store — a declarative, persistence-aware application store.
//!
//! The whole store is ONE declaration: a prefix and a field table,
//! validated and normalized at load time by the `@store` macro. Each
//! field names where it lives:
//!
//!   let SPEC = @store("app", [
//!       ["mode",  "url",   "list"],          // survives reload, deep-
//!       ["query", "url",   "state: open"],   //   linkable, back/forward
//!       ["theme", "local", "system"],        // survives across sessions
//!       ["items", "mem",   []]               // per-page-load
//!   ])
//!
//! Three persistence classes:
//!
//!   mem     ordinary state — any literal default, reset on reload
//!   url     reflected into the querystring (`/?mode=board`); reloads
//!           land exactly where the user was, links are shareable, and
//!           the browser's back/forward buttons walk app history.
//!           Values are strings by contract (parse at the use site).
//!   local   persisted in localStorage under "<prefix>-<name>";
//!           strings by contract. Defaults are stored as ABSENT, so a
//!           user who never chose keeps following new defaults.
//!
//! The lifecycle is four calls:
//!
//!   mount("#app", view, hydrate(SPEC))       // defaults ⊕ url ⊕ local
//!   persist(SPEC, current())                 // after a navigation-ish
//!                                            //   change: url + storage
//!   on_restore(SPEC, (fields) => ...)        // back/forward: url fields
//!                                            //   decoded, handed back
//!   default_of(SPEC, "query")                // the spec is the one
//!                                            //   source of defaults
//!
//! Why a macro: the field table is checked while the program LOADS —
//! duplicate names, unknown classes, non-string url/local defaults,
//! and non-identifier names are load-time errors naming the `@store`
//! site, and the normalized spec (name list, class buckets, default
//! map) is baked as a literal, so the runtime never re-parses the
//! table. Everything here runs natively too (persistence degrades to
//! the defaults), which is what keeps it testable without a browser.

// ── the macro ────────────────────────────────────────────────────────

meta fn store(prefix, fields) = {
    let p = unwrap(meta.eval(prefix))
    let fs = unwrap(meta.eval(fields))
    let lower = "abcdefghijklmnopqrstuvwxyz"
    let tail_ok = lower + "0123456789_"
    let ident_ok = (s) => {
        let n = str.length(s)
        let mut ok = n > 0 && str.contains(lower, str.substring(s, 0, 1))
        let mut i = 1
        while ok && i < n {
            ok = str.contains(tail_ok, str.substring(s, i, i + 1))
            i = i + 1
        }
        ok
    }
    if typeof(p) != "String" || !ident_ok(p) =>
        unwrap(Err("@store: prefix must be a lowercase identifier, got " + show(p)))
    else => ()
    let mut names = []
    let mut defaults = #{}
    let mut url_fields = []
    let mut local_fields = []
    for f in fs {
        if typeof(f) != "List" || len(f) != 3 =>
            unwrap(Err("@store: each field is [name, class, default], got " + show(f)))
        else => ()
        let name = f[0]
        let class = f[1]
        let default = f[2]
        if typeof(name) != "String" || !ident_ok(name) =>
            unwrap(Err("@store: field name must be a lowercase identifier, got "
                + show(name)))
        else => ()
        if contains(names, name) =>
            unwrap(Err("@store: duplicate field '" + name + "'"))
        else => ()
        if !contains(["mem", "url", "local"], class) =>
            unwrap(Err("@store: field '" + name + "' has unknown class '"
                + show(class) + "' (mem | url | local)"))
        else => ()
        if class != "mem" && typeof(default) != "String" =>
            unwrap(Err("@store: field '" + name + "' is '" + class
                + "'-persisted, so its default must be a String"))
        else => ()
        names = names + [name]
        defaults = map_set(defaults, name, default)
        if class == "url" => { url_fields = url_fields + [name] }
        else if class == "local" => { local_fields = local_fields + [name] }
    }
    meta.lit(#{ "prefix": p, "names": names, "defaults": defaults,
        "url": url_fields, "local": local_fields })
}

// ── hydration ────────────────────────────────────────────────────────

share fn default_of(spec, name) = map_get(map_get(spec, "defaults"), name)

// The last querystring this store wrote or restored — so persist only
// pushes real changes, and the first persist after boot is a no-op.
let last_qs = cell.new("")

/// The initial state: every declared default, overridden by the URL's
/// query parameters (url fields) and localStorage (local fields).
/// Natively there is no browser, so hydrate IS the defaults.
share fn hydrate(spec) = {
    let mut s = map_get(spec, "defaults")
    if dom.available() => {
        let qmap = map_get(dom.location(), "query")
        for name in map_get(spec, "url") {
            if map_has_key(qmap, name) => {
                s = map_set(s, name, map_get(qmap, name))
            }
        }
        for name in map_get(spec, "local") {
            let held = dom.storage_get(map_get(spec, "prefix") + "-" + name)
            if held != "" => { s = map_set(s, name, held) }
        }
        cell.set(last_qs, querystring(spec, s))
    }
    s
}

// ── persistence ──────────────────────────────────────────────────────

// Query values need only the characters that would break parsing
// encoded — the browser's URLSearchParams decodes them back.
fn urlencode(v) = v
    |> str.replace("%", "%25")
    |> str.replace("&", "%26")
    |> str.replace("=", "%3D")
    |> str.replace("#", "%23")
    |> str.replace("+", "%2B")
    |> str.replace("?", "%3F")
    |> str.replace(" ", "%20")

/// The querystring the url fields describe: `name=value` in
/// declaration order, defaults omitted — the home state is a clean
/// `/`. Pure, so it tests without a browser.
share fn querystring(spec, s) = {
    let mut parts = []
    for name in map_get(spec, "url") {
        let v = map_get(s, name)
        if v != default_of(spec, name) && typeof(v) == "String" && v != "" => {
            parts = parts + [name + "=" + urlencode(v)]
        }
    }
    join(parts, "&")
}

/// Reflect the state: url fields into browser history (one pushState
/// per actual change — back/forward walk these), local fields into
/// storage (defaults stored as absent). Call after any change to a
/// persisted field; extra calls are free.
share fn persist(spec, s) =
    if dom.available() => {
        let qs = querystring(spec, s)
        if qs != cell.get(last_qs) => {
            cell.set(last_qs, qs)
            dom.push_state(if qs == "" => "/" else => "/?" + qs)
        }
        else => ()
        for name in map_get(spec, "local") {
            let key = map_get(spec, "prefix") + "-" + name
            let v = map_get(s, name)
            if v == default_of(spec, name) => dom.storage_remove(key)
            else => dom.storage_set(key, v)
        }
    }
    else => ()

/// Back/forward: decode the url fields from the restored location
/// (absent → default) and hand them to the app, which applies them
/// and refetches whatever they imply.
share fn on_restore(spec, handler) =
    if dom.available() =>
        dom.on_route((r) => {
            let qmap = map_get(r, "query")
            let mut fields = #{}
            for name in map_get(spec, "url") {
                fields = map_set(fields, name,
                    if map_has_key(qmap, name) => map_get(qmap, name)
                    else => default_of(spec, name))
            }
            cell.set(last_qs, querystring(spec, fields))
            handler(fields)
        })
    else => ()

// ── tests (native: hydration is the defaults, querystring is pure) ──

let TEST_SPEC = @store("t", [
    ["mode", "url", "list"],
    ["query", "url", "state: open"],
    ["theme", "local", "system"],
    ["items", "mem", []],
    ["stats", "mem", ()]
])

test "the macro bakes a normalized spec with class buckets" {
    assert_eq(map_get(TEST_SPEC, "prefix"), "t")
    assert_eq(map_get(TEST_SPEC, "names"),
        ["mode", "query", "theme", "items", "stats"])
    assert_eq(map_get(TEST_SPEC, "url"), ["mode", "query"])
    assert_eq(map_get(TEST_SPEC, "local"), ["theme"])
    assert_eq(default_of(TEST_SPEC, "query"), "state: open")
    assert_eq(default_of(TEST_SPEC, "stats"), ())
}

test "hydrate without a browser is exactly the defaults" {
    let s = hydrate(TEST_SPEC)
    assert_eq(map_get(s, "mode"), "list")
    assert_eq(map_get(s, "items"), [])
    assert_eq(map_get(s, "theme"), "system")
}

test "querystring: declaration order, defaults omitted, encoding" {
    let s = hydrate(TEST_SPEC)
    assert_eq(querystring(TEST_SPEC, s), "")
    let moved = map_set(s, "mode", "board")
    assert_eq(querystring(TEST_SPEC, moved), "mode=board")
    let queried = map_set(moved, "query", "priority: urgent \"a & b\" top: 5")
    assert_eq(querystring(TEST_SPEC, queried),
        "mode=board&query=priority:%20urgent%20\"a%20%26%20b\"%20top:%205")
    // Back to defaults → clean home URL.
    assert_eq(querystring(TEST_SPEC, hydrate(TEST_SPEC)), "")
}

test "persist and on_restore are safe no-ops without a browser" {
    let s = hydrate(TEST_SPEC)
    assert_eq(persist(TEST_SPEC, s), ())
    assert_eq(on_restore(TEST_SPEC, (fields) => fields), ())
}
