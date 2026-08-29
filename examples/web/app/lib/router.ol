// The HTTP layer: routing with `:name` parameters, method-aware 405s,
// a middleware pipeline (request id → auth → handler → access log),
// and one JSON error envelope used everywhere.
//
// A handler is (req, params) -> response. Handlers may also *return*
// Err("...") — dispatch converts that into a clean 500 envelope, so a
// data-layer failure surfaces as JSON, not a dropped connection.

share fn route(method, pattern, handler) = {
    method: method, pattern: pattern, handler: handler
}

fn segments(path) = str.split(path, "/") |> filter((s) => str.length(s) > 0)

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

// ── responses ────────────────────────────────────────────────────────

share fn json_response(status, value) =
    http.response_with_headers(status, unwrap(json.stringify(value)),
        #{ "Content-Type": "application/json" })

// The one error shape every failure wears:
//   { error: { code, message, details? } }
share fn error_response(status, code, message) =
    json_response(status, #{ "error": #{ "code": code, "message": message } })

share fn error_with_details(status, code, message, details) =
    json_response(status, #{ "error": #{ "code": code, "message": message, "details": details } })

// ── query-string helpers (req.query is a parsed map) ─────────────────

share fn q_str(req, key, fallback) =
    if map_has_key(req.query, key) => map_get(req.query, key) else => fallback

share fn q_int(req, key, fallback, lo, hi) = {
    if !map_has_key(req.query, key) => fallback
    else => match str.parse_int(map_get(req.query, key)) {
        Ok(n) => clamp(n, lo, hi),
        Err(e) => fallback
    }
}

// One of an allowed set, else the fallback — for sort columns and
// directions, so nothing from the wire reaches SQL as an identifier.
share fn q_enum(req, key, allowed, fallback) = {
    let v = q_str(req, key, fallback)
    if contains(allowed, v) => v else => fallback
}

// ── auth ─────────────────────────────────────────────────────────────
// When TRACKER_TOKEN is set in the environment, mutating requests must
// carry `Authorization: Bearer <token>`. Reads stay open.

fn bearer_token(req) = {
    let mut found = ""
    for k in map_keys(req.headers) {
        if str.to_lower(k) == "authorization" => { found = map_get(req.headers, k) }
    }
    if str.starts_with(found, "Bearer ") => str.substring(found, 7, str.length(found))
    else => ""
}

share fn authorized(req) = match os.get_env("TRACKER_TOKEN") {
    Err(e) => true,                       // no token configured: open
    Ok(want) => bearer_token(req) == want
}

fn is_mutation(method) = method == "POST" || method == "PATCH" || method == "PUT" || method == "DELETE"

// ── dispatch with middleware ─────────────────────────────────────────

fn find_match(routes, req) = {
    // HEAD rides GET routes — probes and load balancers send it, and a
    // 405 there reads as "down".
    let method = if req.method == "HEAD" => "GET" else => req.method
    let mut hit = { found: false, handler: 0, params: #{} }
    let mut allowed = []
    for r in routes {
        let m = match_path(r.pattern, req.path)
        if m.ok => {
            allowed = concat(allowed, [r.method])
            if !hit.found && r.method == method => {
                hit = { found: true, handler: r.handler, params: m.params }
            }
        }
    }
    { hit: hit, allowed: allowed }
}

share fn dispatch(routes, req) = {
    let req_id = crypto.random_hex(6)
    let started = time.monotonic_ms()

    let outcome = find_match(routes, req)
    let response = if outcome.hit.found => {
        if is_mutation(req.method) && !authorized(req) =>
            error_response(401, "unauthorized", "this endpoint requires Authorization: Bearer <token>")
        else => {
            let h = outcome.hit.handler
            let result = h(req, outcome.hit.params)
            // A handler returning Err(msg) becomes a clean 500 envelope.
            match result {
                Err(message) => error_response(500, "internal", show(message)),
                Ok(resp) => resp,
                resp => resp
            }
        }
    } else => if len(outcome.allowed) > 0 =>
        http.response_with_headers(405,
            unwrap(json.stringify(#{ "error": #{ "code": "method_not_allowed",
                "message": req.method + " not allowed here", "allow": outcome.allowed } })),
            #{ "Content-Type": "application/json", "Allow": join(outcome.allowed, ", ") })
    else =>
        error_response(404, "not_found", "no route for " + req.method + " " + req.path)

    let ms = time.monotonic_ms() - started
    println("[" + req_id + "] " + req.method + " " + req.path +
        " -> " + show(response.status) + " (" + show(ms) + "ms)")
    response
}
