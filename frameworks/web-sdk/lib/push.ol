//! push — server push over `http.defer`, without the bookkeeping.
//!
//! A handler that has nothing to say yet parks its connection under a
//! topic (`hold(topic)`) and returns the ticket; any thread later
//! answers every connection held under that topic (`notify(topic,
//! response)`), or all of them (`notify_all(response)`). The holds live
//! in the runtime's deferred-connection table (`http.hold`, `http.notify`),
//! so a held connection costs a socket, not a worker, and a hundred
//! tabs polling `issues.changed` are a hundred tickets in a map — one
//! table for the process, however many servers it runs.
//!
//!   route("GET", "/api/changes", (req, p) => hold("issues"))
//!   ...
//!   notify("issues", json_response(200, changed))
//!
//! A notified connection is answered once; a client that wants the next
//! change asks again. `held(topic)` counts what is parked.

/// Park the current request under `topic`. Returns the ticket the
/// handler must return; `serve` passes it through untouched.
share fn hold(topic) = http.hold(topic)

/// Answer every connection held under `topic` with `response`
/// (anything a handler may return: a string, `http.response(...)`, a
/// `{ status, body }` record). Returns how many were answered.
share fn notify(topic, response) = http.notify(topic, response)

/// Answer every held connection, whatever its topic.
share fn notify_all(response) = http.notify_all(response)

/// How many connections are held under `topic` right now.
share fn held(topic) = http.held(topic)

test "an empty topic answers nothing" {
    assert_eq(held("nobody"), 0)
    assert_eq(notify("nobody", "nothing"), 0)
}
