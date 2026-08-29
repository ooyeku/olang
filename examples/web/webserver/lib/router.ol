// A small HTTP router. A route pairs a method and a path pattern with a
// handler; patterns may contain `:name` segments, which bind the matching
// path segment into a params map the handler receives. Dispatch walks the
// table in order and falls through to a 404.
//
//   route("GET", "/notes/:id", (req, params) => ... map_get(params, "id") ...)

share fn route(method, pattern, handler) = {
    method: method, pattern: pattern, handler: handler
}

// Split a path into its segments; "/notes/5" -> ["notes", "5"].
fn segments(path) = str.split(path, "/") |> filter((s) => str.length(s) > 0)

// Match one pattern against a path. Returns { ok, params } — `params` is a
// map of the `:name` bindings when ok.
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

// Find the first route matching the request and call its handler with
// (request, params). No match: 404 with the method and path named.
share fn dispatch(routes, req) = {
    let hits = routes |> filter((r) =>
        (r.method == req.method) && match_path(r.pattern, req.path).ok)
    if len(hits) == 0 =>
        http.response(404, "no route for " + req.method + " " + req.path)
    else => {
        let r = hits[0]
        r.handler(req, match_path(r.pattern, req.path).params)
    }
}

// JSON response helper: stringify a value with the right content type.
share fn json_response(status, value) =
    http.response_with_headers(status, unwrap(json.stringify(value)),
        #{ "Content-Type": "application/json" })
