//! routes — one table for the whole stack.
//!
//! A route is data: `#{ "method", "pattern", "name", "handler" }`.
//! The same table drives the server's dispatch (web.server) and the
//! client's named calls (web.api): declare an endpoint once with
//! `rpc("todos.create", handler)` and the browser calls it as
//! `api.call("todos.create", payload)` — the table is the single
//! source of truth, which is what makes the full stack feel like one
//! program.

/// A REST route: `route("GET", "/todos/:id", (req, params) => ...)`.
/// The handler returns a response (web.server's json/error helpers),
/// a plain value (wrapped as `{ data }` with 200), or `Err(message)`
/// (a clean 500 envelope).
share fn route(method, pattern, handler) =
    #{ "method": method, "pattern": pattern, "name": "", "handler": handler }

/// A named endpoint: `rpc("todos.create", (req) => ...)`. Mounted at
/// `POST /api/rpc/<name>`; the client half is `api.call(name, payload)`.
/// The handler receives the request (body already parsed when JSON)
/// and returns a value, a response, or `Err(message)`.
share fn rpc(name, handler) =
    #{ "method": "POST", "pattern": "/api/rpc/" + name, "name": name, "handler": handler }

fn segments(path) = str.split(path, "/") |> filter((s) => str.length(s) > 0)

/// Match a `:param` pattern against a concrete path:
/// `{ ok, params }` with each `:name` segment captured.
share fn match_path(pattern, path) = {
    let want = segments(pattern)
    let got = segments(path)
    if len(want) != len(got) => { ok: false, params: #{} }
    else => {
        let mut params = #{}
        let mut ok = true
        let mut i = 0
        while ok && (i < len(want)) {
            let w = want[i]
            if str.starts_with(w, ":") => {
                params = map_set(params, str.substring(w, 1, str.length(w)), got[i])
            }
            else if w != got[i] => { ok = false }
            i = i + 1
        }
        { ok: ok, params: params }
    }
}

/// The routes matching a path, whatever their method — dispatch uses
/// this for 405-with-Allow; `{ hit, allowed }`.
/// A route table indexed once: exact patterns (no `:param`) keyed by
/// "METHOD /path" for one lookup, the parameterized ones kept for the
/// walk. `find` takes either the list or the index; `serve` builds the
/// index at start, so a hundred rpc routes cost one map lookup per
/// request instead of a hundred pattern matches.
share fn index_routes(routes) = {
    let is_exact = (r) => !str.contains(map_get(r, "pattern"), ":")
    let exact = fold(filter(routes, is_exact), #{}, (acc, r) =>
        map_set(acc, map_get(r, "method") + " " + map_get(r, "pattern"), r))
    #{ "exact": exact, "dynamic": filter(routes, (r) => !is_exact(r)), "routes": routes }
}

fn is_index(routes) = contains(["Map", "JsonObject"], typeof(routes)) && map_has_key(routes, "exact")

share fn find(routes, method, path) = {
    // HEAD rides GET routes — probes and load balancers send it, and a
    // 405 there reads as "down".
    let m = if method == "HEAD" => "GET" else => method
    if is_index(routes) => {
        let exact = map_get(routes, "exact")
        let key = m + " " + path
        if map_has_key(exact, key) => {
            let r = map_get(exact, key)
            #{ "hit": #{ "found": true, "handler": map_get(r, "handler"), "params": #{}, "name": map_get(r, "name") },
               "allowed": [map_get(r, "method")] }
        }
        // A miss on the exact table: the parameterized routes may match,
        // and a 405 needs every method the path answers to, so the walk
        // runs over the whole list as before.
        else => find(map_get(routes, "routes"), method, path)
    }
    else => find_by_walk(routes, m)(path)
}

fn find_by_walk(routes, m) = (path) => {
    let mut hit = #{ "found": false, "handler": 0, "params": #{}, "name": "" }
    let mut allowed = []
    for r in routes {
        let matched = match_path(map_get(r, "pattern"), path)
        if matched.ok => {
            allowed = concat(allowed, [map_get(r, "method")])
            if !map_get(hit, "found") && map_get(r, "method") == m => {
                hit = #{ "found": true, "handler": map_get(r, "handler"),
                         "params": matched.params, "name": map_get(r, "name") }
            }
        }
    }
    #{ "hit": hit, "allowed": allowed }
}

test "an indexed table answers exact paths in one lookup and walks the rest" {
    let table = index_routes([
        route("GET", "/a", (req, p) => "a"),
        route("POST", "/a", (req, p) => "posted"),
        route("GET", "/t/:id", (req, p) => map_get(p, "id"))
    ])
    let hit = map_get(find(table, "GET", "/a"), "hit")
    assert_eq(map_get(hit, "found"), true)
    assert_eq(map_get(hit, "handler")((), #{}), "a")
    let dyn = find(table, "GET", "/t/9")
    assert_eq(map_get(map_get(dyn, "hit"), "params"), #{ "id": "9" })
    // A method miss on an exact path still reports every allowed method.
    let miss = find(table, "DELETE", "/a")
    assert_eq(map_get(map_get(miss, "hit"), "found"), false)
    assert_eq(sort(map_get(miss, "allowed")), ["GET", "POST"])
    assert_eq(map_get(map_get(find(table, "HEAD", "/a"), "hit"), "found"), true)
}

test "patterns capture :params and reject shape mismatches" {
    let m = match_path("/todos/:id/comments/:cid", "/todos/7/comments/9")
    assert_eq(m.ok, true)
    assert_eq(map_get(m.params, "id"), "7")
    assert_eq(map_get(m.params, "cid"), "9")
    assert_eq(match_path("/todos/:id", "/todos").ok, false)
    assert_eq(match_path("/todos", "/todos/7").ok, false)
    assert_eq(match_path("/", "/").ok, true)
}

test "find honors method, HEAD rides GET, allowed collects" {
    let table = [
        route("GET", "/t", (req, p) => "get"),
        route("POST", "/t", (req, p) => "post")
    ]
    let f = find(table, "POST", "/t")
    assert_eq(map_get(map_get(f, "hit"), "found"), true)
    assert_eq(map_get(find(table, "HEAD", "/t"), "hit") |> map_get("found"), true)
    assert_eq(map_get(find(table, "DELETE", "/t"), "hit") |> map_get("found"), false)
    assert_eq(len(map_get(find(table, "DELETE", "/t"), "allowed")), 2)
}

test "rpc routes mount under /api/rpc by name" {
    let r = rpc("todos.create", (req) => 1)
    assert_eq(map_get(r, "pattern"), "/api/rpc/todos.create")
    let f = find([r], "POST", "/api/rpc/todos.create")
    assert_eq(map_get(map_get(f, "hit"), "name"), "todos.create")
}
