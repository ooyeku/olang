//! api — the client half of the route table (browser side).
//!
//! The server declared `rpc("todos.create", handler)`; the browser
//! calls it by the same name. Responses arrive in the SDK envelope,
//! and `call` unwraps it: the callback receives `Ok(data)` or
//! `Err(message)` — never a raw wire shape.
//!
//! `dom.fetch_json` is callback-shaped (the browser's event loop), so
//! the API is too: `call(name, payload, (result) => ...)`.

/// Call a named endpoint: `call("todos.create", #{ "title": t },
/// (r) => match r { Ok(todo) => ..., Err(msg) => ... })`.
share fn call(name, payload, k) =
    if dom.available() =>
        dom.fetch_json("POST", "/api/rpc/" + name,
            unwrap(json.stringify(payload)),
            (resp) => k(unwrap_envelope(resp)))
    else => k(Err("api.call(\"" + name + "\") needs the browser — this is a static preview"))

/// GET a REST route, envelope-unwrapped: `fetch("/api/todos", k)`.
share fn fetch(path, k) =
    if dom.available() =>
        dom.fetch_json("GET", path, "", (resp) => k(unwrap_envelope(resp)))
    else => k(Err("api.fetch(\"" + path + "\") needs the browser — this is a static preview"))

/// The envelope, unwrapped: `{ data }` becomes `Ok(data)`, `{ error }`
/// becomes `Err(message)` — including transport failures, which the
/// dom layer already delivers in the same error shape.
fn maplike(v) = {
    let t = typeof(v)
    t == "Map" || t == "JsonObject" || t == "Object"
}

share fn unwrap_envelope(resp) = {
    if !maplike(resp) => Ok(resp)
    else if map_has_key(resp, "error") => {
        let e = map_get(resp, "error")
        if typeof(e) == "Map" && map_has_key(e, "message") => Err(map_get(e, "message"))
        else => Err(if typeof(e) == "String" => e else => "request failed")
    }
    else if map_has_key(resp, "data") => Ok(map_get(resp, "data"))
    else => Ok(resp)
}

test "the envelope unwraps to Result" {
    assert_eq(unwrap_envelope(#{ "data": 42 }), Ok(42))
    assert_eq(unwrap_envelope(#{ "error": #{ "code": "x", "message": "boom" } }), Err("boom"))
    assert_eq(unwrap_envelope(#{ "error": "flat" }), Err("flat"))
    assert_eq(unwrap_envelope(#{ "other": 1 }), Ok(#{ "other": 1 }))
}
