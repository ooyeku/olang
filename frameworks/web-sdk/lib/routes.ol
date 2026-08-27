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
share fn find(routes, method, path) = {
    // HEAD rides GET routes — probes and load balancers send it, and a
    // 405 there reads as "down".
    let m = if method == "HEAD" => "GET" else => method
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
