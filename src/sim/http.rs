//! `http` inside a world (S2): the same calls, the same values, carried
//! by the world's network instead of sockets. Nothing here opens one.
//!
//! - **The client** (`http.get/post/put/delete/request`) resolves the
//!   URL's host to a node (`localhost` is its own), sends the request as
//!   a message, and waits on its own mailbox for the answer until its
//!   `timeout_ms` (30 s unless given) of *virtual* time. A timeout, a
//!   refused connection or an unknown host is an `Err` value, as a real
//!   network failure is; the response is the `HttpResponse` the real
//!   client answers.
//! - **The server** (`http.serve`) listens on (node, port) and runs its
//!   workers (4 unless `workers` says) as tasks of the node, each with
//!   its own interpreter clone, as the real pool does. A worker takes the
//!   next request, calls the handler, and sends what it returns back as
//!   a message. A handler that raises is a 500. `http.shutdown()` stops
//!   the node's servers and `serve` returns `Ok(())`.
//! - **Deferred answers** (`http.defer/respond/hold/notify/notify_all/
//!   held`) park the request in the world rather than a socket, per node.

use super::wire::{Body, Req, Resp};
use super::{Ctx, misuse, spawn_task};
use crate::ast::{Value, ValueMap};
use crate::interpreter::{Interpreter, InterpreterError};
use std::cell::Cell;
use std::sync::Arc;

/// How long a request waits for its answer unless told (reqwest's
/// default), in virtual ms.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Workers per server unless `workers` says: a fixed number, so a world
/// runs the same on every machine.
pub const DEFAULT_WORKERS: usize = 4;

thread_local! {
    /// The request the handler on this worker answers (its ticket).
    static CURRENT: Cell<u64> = const { Cell::new(0) };
}

fn s(text: impl Into<String>) -> Value {
    Value::String(Arc::new(text.into()))
}

fn err(text: impl Into<String>) -> Value {
    Value::Err(Box::new(s(text)))
}

/// Dispatch `http.<verb>` inside a world. `None` runs the real builtin
/// (the pure ones: `response`, `parse_url`, ...).
pub(super) fn call(
    ctx: &Ctx,
    verb: &str,
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Option<Result<Value, InterpreterError>> {
    Some(match verb {
        "get" | "delete" => match args {
            [url] => client(ctx, verb, url, None, None),
            [url, opts] => client(ctx, verb, url, None, Some(opts)),
            _ => Err(misuse(format!(
                "{verb} expects 1 or 2 arguments, got {}",
                args.len()
            ))),
        },
        "post" | "put" => match args {
            [url, body] => client(ctx, verb, url, Some(body), None),
            [url, body, opts] => client(ctx, verb, url, Some(body), Some(opts)),
            _ => Err(misuse(format!(
                "{verb} expects 2 or 3 arguments, got {}",
                args.len()
            ))),
        },
        "request" => match args {
            [Value::String(method), url, body] => client(ctx, method, url, Some(body), None),
            [Value::String(method), url, body, opts] => {
                client(ctx, method, url, Some(body), Some(opts))
            }
            _ => Err(misuse(
                "request expects (method, url, body[, opts]), the method a String",
            )),
        },
        "serve" => serve(ctx, args, interpreter),
        "shutdown" => {
            ctx.world.shutdown(ctx.task, ctx.node);
            Ok(Value::Unit)
        }
        "defer" => defer(),
        "hold" => match args {
            [Value::String(topic)] => defer().inspect(|ticket| {
                if let Some(id) = ticket_of(ticket) {
                    ctx.world.hold(ctx.node, topic.to_string(), id);
                }
            }),
            _ => Err(misuse("http.hold expects the topic as a String")),
        },
        "respond" => match args {
            [ticket, response] => match ticket_of(ticket) {
                Some(id) => respond(ctx, id, response),
                None => Err(misuse(
                    "http.respond expects the ticket http.defer answered and a response",
                )),
            },
            _ => Err(misuse("http.respond expects the ticket and a response")),
        },
        "notify" => match args {
            [Value::String(topic), response] => {
                let ids = ctx.world.take_held(ctx.node, Some(topic));
                answer_all(ctx, ids, response)
            }
            _ => Err(misuse("http.notify expects the topic and a response")),
        },
        "notify_all" => match args {
            [response] => {
                let ids = ctx.world.take_held(ctx.node, None);
                answer_all(ctx, ids, response)
            }
            _ => Err(misuse("http.notify_all expects a response")),
        },
        "held" => match args {
            [Value::String(topic)] => {
                Ok(Value::Integer(ctx.world.held_count(ctx.node, topic) as i64))
            }
            _ => Err(misuse("http.held expects the topic as a String")),
        },
        _ => return None,
    })
}

// ─── the client ──────────────────────────────────────────────────────

fn client(
    ctx: &Ctx,
    method: &str,
    url: &Value,
    body: Option<&Value>,
    opts: Option<&Value>,
) -> Result<Value, InterpreterError> {
    let upper = method.to_uppercase();
    let lower = method.to_lowercase();
    let Value::String(url) = url else {
        return Err(misuse(format!("{lower}: URL must be a string")));
    };
    let body = match body {
        None => Vec::new(),
        Some(Value::String(b)) => b.as_bytes().to_vec(),
        Some(_) => return Err(misuse(format!("{lower}: body must be a string"))),
    };
    let opts = match opts {
        Some(v) => crate::stdlib::http::parse_opts(v, &lower).map_err(|e| misuse(e.to_string()))?,
        None => crate::stdlib::http::RequestOpts::default(),
    };
    if !matches!(
        upper.as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD"
    ) {
        return Err(misuse(format!("Unsupported HTTP method: {method}")));
    }
    let failed = |why: String| Ok(err(format!("{upper} request failed: {why}")));
    let parsed = match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u,
        Ok(u) => return failed(format!("the scheme {} is not http", u.scheme())),
        Err(e) => return failed(format!("invalid URL `{url}`: {e}")),
    };
    let host = parsed.host_str().unwrap_or("").to_string();
    let w = &ctx.world;
    let to = if matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1") {
        ctx.node
    } else {
        match w.node_named(&host) {
            Some(n) => n,
            None => {
                return failed(format!(
                    "could not resolve host `{host}` (no node of that name in the world)"
                ));
            }
        }
    };
    let port = parsed.port_or_known_default().unwrap_or(80);
    let mut headers: Vec<(String, String)> = vec![("host".into(), format!("{host}:{port}"))];
    for (k, v) in &opts.headers {
        headers.push((k.to_lowercase(), v.clone()));
    }
    if let Some(token) = &opts.bearer {
        headers.push(("authorization".into(), format!("Bearer {token}")));
    }
    if let Some((user, pass)) = &opts.basic {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        headers.push(("authorization".into(), format!("Basic {encoded}")));
    }
    let body = if matches!(upper.as_str(), "GET" | "HEAD") {
        Vec::new()
    } else {
        body
    };
    if !body.is_empty() {
        headers.push(("content-length".into(), body.len().to_string()));
    }
    let reply = w.open_box();
    let req = Req {
        id: 0,
        from: ctx.node,
        port,
        method: upper.clone(),
        path: parsed.path().to_string(),
        query: parsed.query_pairs().into_owned().collect(),
        headers,
        body,
        reply,
        remote: String::new(),
    };
    let timeout = opts.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let result = (|| {
        let id = w.send_request(ctx.task, ctx.node, to, req, url)?;
        let until = w.now() + timeout;
        let site = format!("{upper} {url} (waiting for the answer)");
        let got = w.await_box(ctx.task, reply, Some(until), &site)?;
        Ok::<_, super::Ended>((id, got))
    })();
    // Whatever happened, nobody waits here any more: a late answer drops.
    w.close_box(reply);
    let (id, got) = result?;
    match got {
        None => {
            w.settle_request(
                ctx.task,
                id,
                "timeout",
                None,
                None,
                format!("after {timeout} ms"),
            );
            failed(format!(
                "timed out after {timeout} ms (no answer from {host}:{port})"
            ))
        }
        Some(Body::Refused { why, .. }) => {
            w.settle_request(ctx.task, id, "refused", None, None, why.clone());
            failed(why)
        }
        Some(Body::Response { resp, .. }) => {
            w.settle_request(
                ctx.task,
                id,
                "ok",
                Some(resp.status),
                Some(&resp.body),
                format!("{} {}", resp.status, super::wire::preview(&resp.body)),
            );
            Ok(Value::Ok(Box::new(response_value(resp, opts.bytes))))
        }
        Some(Body::Request(_)) => failed("a request arrived where an answer was due".into()),
    }
}

/// The `HttpResponse` the real client answers.
fn response_value(resp: Resp, bytes: bool) -> Value {
    let mut headers = resp.headers;
    if !headers.iter().any(|(k, _)| k == "content-length") {
        headers.push(("content-length".into(), resp.body.len().to_string()));
    }
    let headers = crate::stdlib::http::multi_map(headers);
    let body = if bytes {
        crate::stdlib::bytes::to_value(resp.body)
    } else {
        s(String::from_utf8_lossy(&resp.body).into_owned())
    };
    let mut m = ValueMap::default();
    m.insert("status".to_string(), Value::Integer(resp.status));
    m.insert("body".to_string(), body);
    m.insert("headers".to_string(), Value::Map(Arc::new(headers)));
    m.insert(
        "success".to_string(),
        Value::Boolean((200..300).contains(&resp.status)),
    );
    Value::Struct {
        type_name: "HttpResponse".into(),
        fields: Arc::new(m).into(),
    }
}

// ─── the server ──────────────────────────────────────────────────────

fn serve(
    ctx: &Ctx,
    args: &[Value],
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    if !(2..=3).contains(&args.len()) {
        return Err(misuse(format!(
            "http.serve expects 2 or 3 arguments (port, handler[, options]), got {}",
            args.len()
        )));
    }
    let port = match &args[0] {
        Value::Integer(p) if (0..=65535).contains(p) => *p as u16,
        _ => return Err(misuse("http.serve: port must be an integer in 0..=65535")),
    };
    let handler = args[1].clone();
    if !matches!(handler, Value::Function(_) | Value::Builtin(_)) {
        return Err(misuse("http.serve: handler must be a function"));
    }
    let config =
        crate::stdlib::http::serve_config(args.get(2)).map_err(|m| misuse(format!("http.{m}")))?;
    let given = |key: &str| {
        args.get(2).is_some_and(|o| match o {
            Value::Map(m) => m.contains_key(key),
            Value::Struct { fields, .. } => fields.contains_key(key),
            _ => false,
        })
    };
    let workers = if given("workers") {
        config.workers
    } else {
        DEFAULT_WORKERS
    };
    let capacity = if given("queue_capacity") {
        config.queue_capacity
    } else {
        workers.saturating_mul(64).max(64)
    };
    let w = &ctx.world;
    let (port, queue, ctl) =
        match w.listen(ctx.task, ctx.node, port, capacity, config.max_body_bytes)? {
            Ok(x) => x,
            Err(message) => return Ok(err(message)),
        };
    let node = ctx.node;
    let mut tasks = Vec::with_capacity(workers);
    for i in 0..workers {
        let mut worker = interpreter.thread_safe_clone();
        let handler = handler.clone();
        let world = w.clone();
        let label = format!("http worker {} of :{port}", i + 1);
        let (task, _) = spawn_task(w, ctx.task, node, None, &label, move || {
            let me = super::current().map(|c| c.task).unwrap_or(0);
            work(&world, me, node, port, queue, &mut worker, &handler)
        })?;
        tasks.push(task);
    }
    w.serve_wait(ctx.task, node, port, queue, ctl)?;
    // A drain waits up to `drain_ms` (the world's time) for the handlers
    // in flight, as a real server does: returning at once dropped a
    // request in flight at a stop that a real machine answers (OL-270).
    let until = w.now() + config.drain_ms;
    while let Some(&busy) = tasks.iter().find(|&&t| w.outcome(t).is_none()) {
        if w.now() >= until {
            w.note(ctx.task, "timeout", format!("http.serve drain on :{port}"));
            break;
        }
        w.block(
            ctx.task,
            super::sched::Wait::Task(busy),
            format!("http.serve draining :{port}"),
            Some(until),
        )?;
    }
    Ok(Value::Ok(Box::new(Value::Unit)))
}

/// One worker: take a request, call the handler, answer; until the
/// server shuts down (or the world ends, or the node crashes).
fn work(
    world: &Arc<super::World>,
    me: super::sched::TaskId,
    node: super::net::NodeId,
    port: u16,
    queue: u64,
    interpreter: &mut Interpreter,
    handler: &Value,
) -> Result<Value, String> {
    let stop = |e: super::Ended| InterpreterError::from(e).to_string();
    while let Some(req) = world.next_request(me, node, port, queue).map_err(stop)? {
        let value = crate::stdlib::http::request_value(
            &req.method,
            &req.path,
            &req.query,
            &req.headers,
            &req.body,
            &req.remote,
        );
        CURRENT.with(|c| c.set(req.id));
        let outcome = interpreter.call_function(handler.clone(), vec![value]);
        CURRENT.with(|c| c.set(0));
        // The world ended or the node crashed under the handler: stop.
        {
            let st = world.lock();
            super::World::stopped(&st, me).map_err(stop)?;
        }
        let resp = match &outcome {
            Ok(Value::Struct { type_name, fields })
                if type_name == "HttpDeferred"
                    && matches!(fields.get("ticket"), Some(Value::Integer(t)) if *t as u64 == req.id) =>
            {
                world.park(node, req);
                continue;
            }
            Ok(v) => render(v),
            Err(e) => {
                let message = super::error_text(e);
                world.note(me, "fail", format!("handler for #{}: {message}", req.id));
                plain(500, "internal server error")
            }
        };
        world.respond(me, node, &req, resp).map_err(stop)?;
    }
    Ok(Value::Unit)
}

fn plain(status: i64, text: &str) -> Resp {
    Resp {
        status,
        headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
        body: text.as_bytes().to_vec(),
    }
}

fn header_pairs(v: Option<&Value>) -> Vec<(String, String)> {
    let line = crate::stdlib::http::header_line_safe;
    let text = crate::stdlib::http::header_value_text;
    match v {
        Some(Value::Map(m)) => m.iter().map(|(k, v)| (line(k), line(&text(v)))).collect(),
        Some(Value::Struct { fields, .. }) => fields
            .iter()
            .map(|(k, v)| (line(k), line(&text(v))))
            .collect(),
        _ => Vec::new(),
    }
}

/// What a handler returned, as the response the real server would send.
fn render(v: &Value) -> Resp {
    match v {
        Value::String(text) => plain(200, text),
        Value::Struct { fields, .. } => {
            let status = match fields.get("status") {
                Some(Value::Integer(n)) => *n,
                _ => 200,
            };
            if fields.get("body_file").is_some() {
                return plain(
                    500,
                    "sim: body_file reads a file, and a world has no filesystem yet (S3)",
                );
            }
            let mut headers: Vec<(String, String)> = header_pairs(fields.get("headers"))
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect();
            let (body, kind) = match fields.get("body") {
                Some(raw @ Value::Native(_)) => match crate::stdlib::bytes::bytes_of(raw) {
                    Ok(b) => (b.to_vec(), "application/octet-stream"),
                    Err(_) => (raw.to_string().into_bytes(), "text/plain; charset=utf-8"),
                },
                Some(Value::String(t)) => (t.as_bytes().to_vec(), "text/plain; charset=utf-8"),
                Some(other) => (other.to_string().into_bytes(), "text/plain; charset=utf-8"),
                None => (Vec::new(), "text/plain; charset=utf-8"),
            };
            if !headers.iter().any(|(k, _)| k == "content-type") {
                headers.insert(0, ("content-type".into(), kind.into()));
            }
            Resp {
                status,
                headers,
                body,
            }
        }
        other => plain(
            500,
            &format!(
                "handler returned {}, expected a response or a string",
                other.type_name()
            ),
        ),
    }
}

// ─── deferred answers ────────────────────────────────────────────────

fn ticket_of(v: &Value) -> Option<u64> {
    match v {
        Value::Struct { type_name, fields } if type_name == "HttpDeferred" => {
            match fields.get("ticket") {
                Some(Value::Integer(id)) => Some(*id as u64),
                _ => None,
            }
        }
        Value::Integer(id) => Some(*id as u64),
        _ => None,
    }
}

fn defer() -> Result<Value, InterpreterError> {
    let id = CURRENT.with(|c| c.get());
    if id == 0 {
        return Err(misuse(
            "http.defer: no request is being handled on this thread",
        ));
    }
    let mut fields = ValueMap::default();
    fields.insert("ticket".to_string(), Value::Integer(id as i64));
    Ok(Value::Struct {
        type_name: "HttpDeferred".into(),
        fields: Arc::new(fields).into(),
    })
}

fn respond(ctx: &Ctx, ticket: u64, response: &Value) -> Result<Value, InterpreterError> {
    let Some(parked) = ctx.world.unpark(ticket) else {
        return Ok(err(
            "http.respond: no deferred request holds this ticket (answered already, or the client left)",
        ));
    };
    ctx.world
        .respond(ctx.task, parked.node, &parked.req, render(response))?;
    Ok(Value::Ok(Box::new(Value::Unit)))
}

fn answer_all(ctx: &Ctx, ids: Vec<u64>, response: &Value) -> Result<Value, InterpreterError> {
    let mut answered = 0;
    for id in ids {
        if let Value::Ok(_) = respond(ctx, id, response)? {
            answered += 1;
        }
    }
    Ok(Value::Integer(answered))
}
