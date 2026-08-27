//! api — the client half of the route table (browser side).
//!
//! The server declared `rpc("todos.create", handler)`; the browser
//! calls it by the same name. Responses arrive in the SDK envelope,
//! and `call` unwraps it: the callback receives `Ok(data)` or
//! `Err(#{ "message", "details" })` — never a raw wire shape.
//!
//! `dom.fetch_json` is callback-shaped (the browser's event loop), so
//! the API is too: `call(name, payload, (result) => ...)`.

/// Call a named endpoint: `call("todos.create", #{ "title": t },
/// (r) => match r { Ok(todo) => ..., Err(e) => ... })`.
share fn call(name, payload, k) =
    if dom.available() =>
        dom.fetch_json("POST", "/api/rpc/" + name,
            unwrap(json.stringify(payload)),
            (resp) => k(unwrap_envelope(resp)))
    else => k(Err(#{ "message": "api.call(\"" + name
        + "\") needs the browser — this is a static preview", "details": [] }))

/// GET a REST route, envelope-unwrapped: `fetch("/api/todos", k)`.
share fn fetch(path, k) =
    if dom.available() =>
        dom.fetch_json("GET", path, "", (resp) => k(unwrap_envelope(resp)))
    else => k(Err(#{ "message": "api.fetch(\"" + path
        + "\") needs the browser — this is a static preview", "details": [] }))

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

test "err accessors read both halves" {
    let e = #{ "message": "validation failed", "details": ["title: required"] }
    assert_eq(err_message(e), "validation failed")
    assert_eq(err_details(e), ["title: required"])
    assert_eq(err_message("plain"), "plain")
    assert_eq(err_details("plain"), [])
}
