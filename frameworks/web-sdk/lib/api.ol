//! api — the client half of the route table (browser side).
//!
//! The server declared `rpc("todos.create", handler)`; the browser
//! calls it by the same name. Responses arrive in the SDK envelope,
//! and `call` unwraps it: the callback receives `Ok(data)` or
//! `Err(#{ "message", "details" })` — never a raw wire shape.
//!
//! `dom.request` is callback-shaped (the browser's event loop), so the
//! API is too: `call(name, payload, (result) => ...)`. It carries the
//! status code, so a response that is not the envelope — a proxy's
//! HTML, a bare "internal server error" — is an `Err` naming the
//! status, never a JSON parse failure in the handler.

/// Call a named endpoint: `call("todos.create", #{ "title": t },
/// (r) => match r { Ok(todo) => ..., Err(e) => ... })`.
share fn call(name, payload, k) =
    if dom.available() =>
        dom.request("POST", "/api/rpc/" + name,
            unwrap(json.stringify(payload)),
            (resp) => k(from_response(resp)))
    else => k(Err(#{ "message": "api.call(\"" + name
        + "\") needs the browser — this is a static preview", "details": [] }))

/// GET a REST route, envelope-unwrapped: `fetch("/api/todos", k)`.
share fn fetch(path, k) =
    if dom.available() =>
        dom.request("GET", path, "", (resp) => k(from_response(resp)))
    else => k(Err(#{ "message": "api.fetch(\"" + path
        + "\") needs the browser — this is a static preview", "details": [] }))

/// A `dom.request` response (`#{ "status", "headers", "body" }`) as the
/// api Result: a JSON body goes through the envelope; a network failure
/// (status 0) or a body that is not JSON is an `Err` carrying the
/// status and the text.
share fn from_response(resp) = {
    let status = if map_has_key(resp, "status") => map_get(resp, "status") else => 0
    let body = if map_has_key(resp, "body") => map_get(resp, "body") else => ""
    if status == 0 => Err(#{ "message": "request failed: " +
        (if map_has_key(resp, "error") => to_string(map_get(resp, "error")) else => "no response"),
        "details": [], "status": 0 })
    else => match json.parse(body) {
        Ok(v) => unwrap_envelope(v),
        Err(e) => Err(#{ "message": "HTTP " + to_string(status) + ": " + str.trim(body),
                         "details": [], "status": status })
    }
}

fn maplike(v) = {
    let t = typeof(v)
    t == "Map" || t == "JsonObject" || t == "Object"
}

/// The message inside an `api` Err — for callers that only want text.
share fn err_message(e) =
    if maplike(e) && map_has_key(e, "message") => map_get(e, "message")
    else if typeof(e) == "String" => e
    else => "request failed"

/// The validation details inside an `api` Err (possibly empty) — the
/// problem list a 422 envelope carried, ready for `form_fields`.
share fn err_details(e) =
    if maplike(e) && map_has_key(e, "details") => map_get(e, "details")
    else => []

/// The envelope, unwrapped: `{ data }` becomes `Ok(data)`; `{ error }`
/// becomes `Err(#{ "message", "details" })` — details is the
/// validation problem list when the server sent one, else empty — and
/// transport failures arrive the same way, because the dom layer
/// already delivers them in the error shape.
share fn unwrap_envelope(resp) = {
    if !maplike(resp) => Ok(resp)
    else if map_has_key(resp, "error") => {
        let e = map_get(resp, "error")
        if maplike(e) => {
            let message = if map_has_key(e, "message") => map_get(e, "message")
                else => "request failed"
            let details = if map_has_key(e, "details") => map_get(e, "details") else => []
            Err(#{ "message": message, "details": details })
        }
        else => Err(#{ "message": if typeof(e) == "String" => e else => "request failed",
                       "details": [] })
    }
    else if map_has_key(resp, "data") => Ok(map_get(resp, "data"))
    else => Ok(resp)
}

test "the envelope unwraps to Result" {
    assert_eq(unwrap_envelope(#{ "data": 42 }), Ok(42))
    assert_eq(unwrap_envelope(#{ "error": #{ "code": "x", "message": "boom" } }),
        Err(#{ "message": "boom", "details": [] }))
    assert_eq(unwrap_envelope(#{ "error": "flat" }),
        Err(#{ "message": "flat", "details": [] }))
    assert_eq(unwrap_envelope(#{ "error": #{ "message": "invalid",
        "details": ["title: required"] } }),
        Err(#{ "message": "invalid", "details": ["title: required"] }))
    assert_eq(unwrap_envelope(#{ "other": 1 }), Ok(#{ "other": 1 }))
}

test "from_response: the envelope when JSON, the status when not" {
    let ok = from_response(#{ "status": 200, "headers": #{}, "body": "{\"data\": [1, 2]}" })
    assert_eq(ok, Ok([1, 2]))
    let enveloped = from_response(#{ "status": 422, "headers": #{},
        "body": "{\"error\": {\"code\": \"invalid\", \"message\": \"validation failed\", \"details\": [\"title: required\"]}}" })
    assert_eq(err_details(match enveloped { Err(e) => e, Ok(v) => #{} }), ["title: required"])
    let plain = from_response(#{ "status": 500, "headers": #{}, "body": "internal server error" })
    let e = match plain { Err(x) => x, Ok(v) => #{} }
    assert_eq(map_get(e, "message"), "HTTP 500: internal server error")
    assert_eq(map_get(e, "status"), 500)
    let down = from_response(#{ "status": 0, "headers": #{}, "body": "", "error": "TypeError: Failed to fetch" })
    assert_eq(str.starts_with(err_message(match down { Err(x) => x, Ok(v) => #{} }), "request failed"), true)
}

test "err accessors read both halves" {
    let e = #{ "message": "validation failed", "details": ["title: required"] }
    assert_eq(err_message(e), "validation failed")
    assert_eq(err_details(e), ["title: required"])
    assert_eq(err_message("plain"), "plain")
    assert_eq(err_details("plain"), [])
}
