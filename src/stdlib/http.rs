use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Error types for HTTP operations
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("Request failed: {message}")]
    RequestFailed { message: String },
    #[error("Invalid URL: {url}")]
    InvalidUrl { url: String },
    #[error("Connection timeout: {url}")]
    Timeout { url: String },
    #[error("Server error: {message}")]
    ServerError { message: String },
    #[error("JSON parsing error: {message}")]
    JsonError { message: String },
}

impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            HttpError::Timeout {
                url: err.url().map(|u| u.to_string()).unwrap_or_default(),
            }
        } else {
            HttpError::RequestFailed {
                message: err.to_string(),
            }
        }
    }
}

impl From<url::ParseError> for HttpError {
    fn from(err: url::ParseError) -> Self {
        HttpError::InvalidUrl {
            url: err.to_string(),
        }
    }
}

/// Creates the http module with all HTTP functions
pub fn create_http_module() -> Value {
    let mut module = HashMap::new();

    // HTTP Client operations
    module.insert("get".to_string(), create_builtin_function("get", 1));
    module.insert("post".to_string(), create_builtin_function("post", 2));
    module.insert("put".to_string(), create_builtin_function("put", 2));
    module.insert("delete".to_string(), create_builtin_function("delete", 1));
    module.insert("request".to_string(), create_builtin_function("request", 3));

    // HTTP Server operations
    module.insert("serve".to_string(), create_builtin_function("serve", 2));
    module.insert(
        "response".to_string(),
        create_builtin_function("response", 2),
    );
    module.insert(
        "response_with_headers".to_string(),
        create_builtin_function("response_with_headers", 3),
    );

    // Utility functions
    module.insert(
        "parse_url".to_string(),
        create_builtin_function("parse_url", 1),
    );
    module.insert(
        "encode_query".to_string(),
        create_builtin_function("encode_query", 1),
    );
    module.insert(
        "decode_query".to_string(),
        create_builtin_function("decode_query", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("http.{}", name),
        arity,
    })
}

/// Main dispatcher for HTTP function calls
pub fn call_http_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "get" => http_get(args),
        "post" => http_post(args),
        "put" => http_put(args),
        "delete" => http_delete(args),
        "request" => http_request(args),
        "serve" => http_serve(args),
        "response" => http_response(args),
        "response_with_headers" => http_response_with_headers(args),
        "parse_url" => parse_url(args),
        "encode_query" => encode_query(args),
        "decode_query" => decode_query(args),
        _ => Err(format!("Unknown http function: {}", name).into()),
    }
}

/// Request options, parsed from the optional trailing map every client
/// verb accepts: `#{ "headers": #{...}, "timeout_ms": Int,
/// "bearer": String, "basic": (user, pass) }`. An unknown key or a
/// wrong-typed value raises (a typo'd option silently ignored would be a
/// request that quietly lacked its auth header).
#[derive(Default)]
struct RequestOpts {
    headers: Vec<(String, String)>,
    timeout_ms: Option<u64>,
    bearer: Option<String>,
    basic: Option<(String, String)>,
}

fn parse_opts(v: &Value, fname: &str) -> Result<RequestOpts, Box<dyn std::error::Error>> {
    let entries = match v {
        Value::Map(entries) => entries,
        other => {
            return Err(format!(
                "{fname}: options must be a map (#{{ \"headers\": ..., \"timeout_ms\": ..., \"bearer\": ..., \"basic\": ... }}), got {}",
                other.type_name()
            )
            .into());
        }
    };
    let mut opts = RequestOpts::default();
    for (key, value) in entries.iter() {
        match key.as_str() {
            "headers" => match value {
                Value::Map(hs) => {
                    for (name, val) in hs.iter() {
                        match val {
                            Value::String(sv) => {
                                opts.headers.push((name.clone(), sv.as_ref().clone()))
                            }
                            other => {
                                return Err(format!(
                                    "{fname}: header '{name}' must be a string, got {}",
                                    other.type_name()
                                )
                                .into());
                            }
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "{fname}: \"headers\" must be a map of header name to string, got {}",
                        other.type_name()
                    )
                    .into());
                }
            },
            "timeout_ms" => match value {
                Value::Integer(ms) if *ms > 0 => opts.timeout_ms = Some(*ms as u64),
                other => {
                    return Err(format!(
                        "{fname}: \"timeout_ms\" must be a positive integer, got {other}"
                    )
                    .into());
                }
            },
            "bearer" => match value {
                Value::String(tok) => opts.bearer = Some(tok.as_ref().clone()),
                other => {
                    return Err(format!(
                        "{fname}: \"bearer\" must be a string token, got {}",
                        other.type_name()
                    )
                    .into());
                }
            },
            "basic" => match value {
                Value::Tuple(parts) | Value::List(parts) if parts.len() == 2 => {
                    match (&parts[0], &parts[1]) {
                        (Value::String(u), Value::String(pw)) => {
                            opts.basic = Some((u.as_ref().clone(), pw.as_ref().clone()))
                        }
                        _ => {
                            return Err(format!(
                                "{fname}: \"basic\" must be a (user, password) pair of strings"
                            )
                            .into());
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "{fname}: \"basic\" must be a (user, password) pair of strings"
                    )
                    .into());
                }
            },
            other => {
                return Err(format!(
                    "{fname}: unknown option \"{other}\" — the options are \"headers\", \"timeout_ms\", \"bearer\", and \"basic\""
                )
                .into());
            }
        }
    }
    Ok(opts)
}

/// One request core for every client verb. Network failure is a value
/// (`Err(message)`); a misused call raises through the dispatcher.
fn perform(
    method: &str,
    url: &str,
    body: Option<&str>,
    opts: RequestOpts,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut client = reqwest::blocking::Client::builder();
    if let Some(ms) = opts.timeout_ms {
        client = client.timeout(std::time::Duration::from_millis(ms));
    }
    let client = client
        .build()
        .map_err(|e| format!("{method}: could not build HTTP client: {e}"))?;

    let upper = method.to_uppercase();
    let mut builder = match upper.as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        "HEAD" => client.head(url),
        _ => {
            return Err(format!("Unsupported HTTP method: {}", method).into());
        }
    };

    for (name, value) in &opts.headers {
        builder = builder.header(name, value);
    }
    if let Some(token) = &opts.bearer {
        builder = builder.bearer_auth(token);
    }
    if let Some((user, pass)) = &opts.basic {
        builder = builder.basic_auth(user, Some(pass));
    }
    if let Some(body) = body
        && !body.is_empty()
        && upper != "GET"
        && upper != "HEAD"
    {
        builder = builder.body(body.to_string());
    }

    match builder.send() {
        Ok(response) => {
            let status = response.status().as_u16() as i64;
            // Response headers, lowercased for predictable lookup; the
            // first value wins for a repeated header.
            let mut headers = HashMap::new();
            for (name, value) in response.headers() {
                let key = name.as_str().to_lowercase();
                if let Ok(v) = value.to_str() {
                    headers
                        .entry(key)
                        .or_insert_with(|| Value::String(Arc::new(v.to_string())));
                }
            }
            match response.text() {
                Ok(response_body) => {
                    let mut response_map = HashMap::new();
                    response_map.insert("status".to_string(), Value::Integer(status));
                    response_map.insert("body".to_string(), Value::String(Arc::new(response_body)));
                    response_map.insert("headers".to_string(), Value::Map(Arc::new(headers)));
                    response_map.insert(
                        "success".to_string(),
                        Value::Boolean((200..300).contains(&status)),
                    );
                    Ok(Value::Ok(Box::new(Value::Struct {
                        type_name: "HttpResponse".to_string(),
                        fields: std::sync::Arc::new(response_map),
                    })))
                }
                Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "Failed to read response body: {}",
                    e
                )))))),
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "{} request failed: {}",
            upper, e
        )))))),
    }
}

fn str_arg<'a>(v: &'a Value, what: &str) -> Result<&'a str, Box<dyn std::error::Error>> {
    match v {
        Value::String(s) => Ok(s.as_ref()),
        _ => Err(format!("{what} must be a string").into()),
    }
}

/// Make a GET request. An optional trailing options map carries headers,
/// timeout, and auth — see `parse_opts`.
/// Usage: http.get(url) / http.get(url, #{ "bearer": token })
fn http_get(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.is_empty() || args.len() > 2 {
        return Err(format!("get expects 1 or 2 arguments, got {}", args.len()).into());
    }
    let url = str_arg(&args[0], "get: URL")?;
    let opts = match args.get(1) {
        Some(v) => parse_opts(v, "get")?,
        None => RequestOpts::default(),
    };
    perform("GET", url, None, opts)
}

/// Make a POST request.
/// Usage: http.post(url, body) / http.post(url, body, #{ "headers": #{ "content-type": "application/json" } })
fn http_post(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!("post expects 2 or 3 arguments, got {}", args.len()).into());
    }
    let url = str_arg(&args[0], "post: URL")?;
    let body = str_arg(&args[1], "post: body")?;
    let opts = match args.get(2) {
        Some(v) => parse_opts(v, "post")?,
        None => RequestOpts::default(),
    };
    perform("POST", url, Some(body), opts)
}

/// Make a PUT request.
/// Usage: http.put(url, body) / http.put(url, body, opts)
fn http_put(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!("put expects 2 or 3 arguments, got {}", args.len()).into());
    }
    let url = str_arg(&args[0], "put: URL")?;
    let body = str_arg(&args[1], "put: body")?;
    let opts = match args.get(2) {
        Some(v) => parse_opts(v, "put")?,
        None => RequestOpts::default(),
    };
    perform("PUT", url, Some(body), opts)
}

/// Make a DELETE request.
/// Usage: http.delete(url) / http.delete(url, opts)
fn http_delete(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.is_empty() || args.len() > 2 {
        return Err(format!("delete expects 1 or 2 arguments, got {}", args.len()).into());
    }
    let url = str_arg(&args[0], "delete: URL")?;
    let opts = match args.get(1) {
        Some(v) => parse_opts(v, "delete")?,
        None => RequestOpts::default(),
    };
    perform("DELETE", url, None, opts)
}

/// Make a request with an explicit method.
/// Usage: http.request("PATCH", url, body) / http.request(method, url, body, opts)
fn http_request(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() < 3 || args.len() > 4 {
        return Err(format!("request expects 3 or 4 arguments, got {}", args.len()).into());
    }
    let method = str_arg(&args[0], "request: method")?;
    let url = str_arg(&args[1], "request: URL")?;
    let body = str_arg(&args[2], "request: body")?;
    let opts = match args.get(3) {
        Some(v) => parse_opts(v, "request")?,
        None => RequestOpts::default(),
    };
    perform(method, url, Some(body), opts)
}

/// Start an HTTP server (simplified implementation for basic use)
/// Usage: http.serve(8080, handler_function) -> Result<Unit, Error>
fn http_serve(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    // The real server needs the interpreter (to call the olang handler), so
    // `http.serve` is dispatched to `serve_blocking` in the builtin layer.
    // Reaching this interpreter-less path is a routing bug.
    Err("serve must be dispatched with the interpreter (internal routing error)".into())
}

/// A parsed incoming request, before conversion to an olang value.
struct ParsedRequest {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: String,
}

/// Read and parse one HTTP/1.1 request from the stream. Minimal but honest:
/// request line, headers (keys lowercased), and a Content-Length body.
/// Errors carry the status they deserve (400/408/413/431), so abuse is
/// answered precisely and the connection closed.
fn read_request(
    stream: &mut std::net::TcpStream,
    limits: &ServeConfig,
) -> Result<Option<ParsedRequest>, (i64, String)> {
    use std::io::Read;

    let max_head = limits.max_header_bytes;
    let max_body = limits.max_body_bytes;
    // The whole-request deadline. Per-read timeouts bound each read;
    // this bounds their sum, which is what stops a client dribbling one
    // byte per second from holding a worker forever.
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(limits.request_timeout_ms);
    let overdue = |buf: &Vec<u8>| -> Option<(i64, String)> {
        (std::time::Instant::now() >= deadline && !buf.is_empty())
            .then(|| (408, "request took too long to arrive".to_string()))
    };

    // Read until the blank line that ends the header block.
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > max_head {
            return Err((431, "request header block too large".to_string()));
        }
        if let Some(e) = overdue(&buf) {
            return Err(e);
        }
        match stream.read(&mut chunk) {
            // A close (or idle timeout) before any bytes is the clean end of
            // a kept-alive connection, not an error.
            Ok(0) if buf.is_empty() => return Ok(None),
            Ok(0) => return Err((400, "connection closed mid-request".to_string())),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if buf.is_empty()
                    && matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
            {
                return Ok(None);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // Mid-request idle: the per-read timeout fired. Let the
                // deadline decide whether to keep waiting.
                if let Some(e) = overdue(&buf) {
                    return Err(e);
                }
            }
            Err(e) => return Err((400, e.to_string())),
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");

    // Request line: METHOD SP target SP version
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| (400, "malformed request line".to_string()))?
        .to_uppercase();
    let target = parts
        .next()
        .ok_or_else(|| (400, "malformed request line".to_string()))?;

    let (path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.to_string(), String::new()),
    };
    let query: Vec<(String, String)> = url::form_urlencoded::parse(raw_query.as_bytes())
        .into_owned()
        .collect();

    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_lowercase(), v.trim().to_string()));
        }
    }

    // Body: exactly Content-Length bytes (what's already buffered plus
    // more). A Content-Length that does not parse is a malformed
    // request, not an empty body — treating it as 0 would desynchronize
    // the kept-alive stream into the next request.
    let content_length = match headers.iter().find(|(k, _)| k == "content-length") {
        None => 0,
        Some((_, v)) => v
            .parse::<usize>()
            .map_err(|_| (400, format!("invalid Content-Length: {:?}", v)))?,
    };
    if content_length > max_body {
        return Err((413, "request body too large".to_string()));
    }
    let mut body_bytes: Vec<u8> = buf[head_end + 4..].to_vec();
    while body_bytes.len() < content_length {
        if let Some(e) = overdue(&buf) {
            return Err(e);
        }
        let n = match stream.read(&mut chunk) {
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e) => return Err((400, e.to_string())),
        };
        if n == 0 {
            break;
        }
        body_bytes.extend_from_slice(&chunk[..n]);
    }
    body_bytes.truncate(content_length);

    Ok(Some(ParsedRequest {
        method,
        path,
        query,
        headers,
        body: String::from_utf8_lossy(&body_bytes).to_string(),
    }))
}

/// The request value handed to the olang handler: a struct with dot access,
/// with `query` and `headers` as maps so `map_get` reads them.
fn request_to_value(req: &ParsedRequest, remote_addr: &str) -> Value {
    let query_map: HashMap<String, Value> = req
        .query
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(Arc::new(v.clone()))))
        .collect();
    let header_map: HashMap<String, Value> = req
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(Arc::new(v.clone()))))
        .collect();

    let mut fields = HashMap::new();
    fields.insert(
        "method".to_string(),
        Value::String(Arc::new(req.method.clone())),
    );
    fields.insert(
        "path".to_string(),
        Value::String(Arc::new(req.path.clone())),
    );
    fields.insert("query".to_string(), Value::Map(Arc::new(query_map)));
    fields.insert("headers".to_string(), Value::Map(Arc::new(header_map)));
    fields.insert(
        "body".to_string(),
        Value::String(Arc::new(req.body.clone())),
    );
    fields.insert(
        "remote_addr".to_string(),
        Value::String(Arc::new(remote_addr.to_string())),
    );

    Value::Struct {
        type_name: "HttpRequest".to_string(),
        fields: std::sync::Arc::new(fields),
    }
}

fn reason_phrase(status: i64) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// Serialize a status/body/header set into HTTP/1.1 response bytes.
fn response_raw_bytes(
    status: i64,
    body: &[u8],
    extra_headers: &[(String, String)],
    keep_alive: bool,
) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {} {}\r\n", status, reason_phrase(status));
    let has_content_type = extra_headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    if !has_content_type {
        out.push_str("Content-Type: application/octet-stream\r\n");
    }
    for (k, v) in extra_headers {
        out.push_str(&format!(
            "{}: {}\r\n",
            header_line_safe(k),
            header_line_safe(v)
        ));
    }
    out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    out.push_str(if keep_alive {
        "Connection: keep-alive\r\n\r\n"
    } else {
        "Connection: close\r\n\r\n"
    });
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// A header key or value is one line: strip CR and LF so a handler
/// interpolating attacker-controlled text into a header cannot split
/// the response or inject headers of its own.
fn header_line_safe(s: &str) -> String {
    if s.contains(['\r', '\n']) {
        s.replace(['\r', '\n'], " ")
    } else {
        s.to_string()
    }
}

fn response_bytes(
    status: i64,
    body: &str,
    extra_headers: &[(String, String)],
    keep_alive: bool,
) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {} {}\r\n", status, reason_phrase(status));
    let has_content_type = extra_headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    if !has_content_type {
        out.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    }
    for (k, v) in extra_headers {
        out.push_str(&format!(
            "{}: {}\r\n",
            header_line_safe(k),
            header_line_safe(v)
        ));
    }
    out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    if keep_alive {
        out.push_str("Connection: keep-alive\r\n\r\n");
    } else {
        out.push_str("Connection: close\r\n\r\n");
    }
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// Turn whatever the handler returned into response bytes. A response struct
/// (from `http.response`/`response_with_headers`, or any struct-like value
/// with `status`/`body`/`headers` fields) is honored; a bare string is a 200.
fn render_handler_result(value: &Value, keep_alive: bool, fs: crate::caps::FsCap) -> Vec<u8> {
    match value {
        Value::String(s) => response_bytes(200, s, &[], keep_alive),
        Value::Struct { fields, .. } => {
            let status = match fields.get("status") {
                Some(Value::Integer(s)) => *s,
                _ => 200,
            };
            let body = match fields.get("body") {
                Some(Value::String(s)) => s.as_ref().clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            let mut headers = Vec::new();
            match fields.get("headers") {
                Some(Value::Struct { fields: hs, .. }) => {
                    for (k, v) in hs.iter() {
                        headers.push((k.clone(), header_value_text(v)));
                    }
                }
                Some(Value::Map(m)) => {
                    for (k, v) in m.iter() {
                        headers.push((k.clone(), header_value_text(v)));
                    }
                }
                _ => {}
            }
            // `body_file`: serve a file's raw bytes — the binary-safe path
            // (olang strings cannot carry arbitrary bytes; wasm artifacts
            // and images can). Takes precedence over `body` when present.
            if let Some(Value::String(path)) = fields.get("body_file") {
                // `body_file` reads a file: confine it under `fs`, so `net`
                // (which gates `http`) is not a latent file-read capability.
                if !fs.allows(crate::caps::FsCap::Read) {
                    return response_bytes(
                        403,
                        "body_file: capability 'fs' denied (this program cannot read files)",
                        &[],
                        keep_alive,
                    );
                }
                return match std::fs::read(path.as_str()) {
                    Ok(bytes) => response_raw_bytes(status, &bytes, &headers, keep_alive),
                    Err(e) => {
                        response_bytes(404, &format!("body_file {}: {}", path, e), &[], keep_alive)
                    }
                };
            }
            // A Bytes body (`fs.read_bytes`, `meta.encode`) is served raw:
            // the binary-safe path that needs no file on disk.
            if let Some(raw @ Value::Native(_)) = fields.get("body")
                && let Ok(bytes) = crate::stdlib::bytes::bytes_of(raw)
            {
                return response_raw_bytes(status, bytes, &headers, keep_alive);
            }
            response_bytes(status, &body, &headers, keep_alive)
        }
        other => response_bytes(
            500,
            &format!(
                "handler returned {}, expected a response or a string",
                other.type_name()
            ),
            &[],
            keep_alive,
        ),
    }
}

fn header_value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.as_ref().clone(),
        other => other.to_string(),
    }
}

#[derive(Clone)]
struct ServeConfig {
    workers: usize,
    queue_capacity: usize,
    max_requests_per_connection: usize,
    idle_timeout_ms: u64,
    write_timeout_ms: u64,
    /// Ceiling for one request's header block (431 past it).
    max_header_bytes: usize,
    /// Ceiling for one request's body (413 past it).
    max_body_bytes: usize,
    /// Whole-request deadline, head plus body (408 past it) — the
    /// slow-client bound the per-read timeout alone cannot give.
    request_timeout_ms: u64,
    /// The address to listen on. 127.0.0.1 unless asked: a server that
    /// should be reachable from another machine says so.
    bind: String,
}

fn default_worker_count() -> usize {
    std::env::var("OLANG_HTTP_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|workers| *workers > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(4)
        })
}

fn option_usize(
    fields: &HashMap<String, Value>,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    match fields.get(name) {
        None => Ok(default),
        Some(Value::Integer(value)) if *value >= min as i64 && *value <= max as i64 => {
            Ok(*value as usize)
        }
        Some(Value::Integer(value)) => Err(format!(
            "serve: option '{}' must be in {}..={}, got {}",
            name, min, max, value
        )),
        Some(other) => Err(format!(
            "serve: option '{}' must be an integer, got {}",
            name,
            other.type_name()
        )),
    }
}

fn serve_config(options: Option<&Value>) -> Result<ServeConfig, String> {
    let workers = default_worker_count();
    let defaults = ServeConfig {
        workers,
        queue_capacity: workers.saturating_mul(64).max(64),
        // A finite keep-alive quantum prevents a small set of persistent
        // clients from monopolizing every worker while other connections sit
        // in the queue. Clients reconnect transparently after the cap.
        max_requests_per_connection: 100,
        idle_timeout_ms: 5_000,
        write_timeout_ms: 10_000,
        max_header_bytes: 64 * 1024,
        max_body_bytes: 10 * 1024 * 1024,
        request_timeout_ms: 30_000,
        bind: "127.0.0.1".to_string(),
    };
    let Some(options) = options else {
        return Ok(defaults);
    };
    let fields = match options {
        Value::Map(map) => map.as_ref(),
        Value::Struct { fields, .. } => fields,
        other => {
            return Err(format!(
                "serve: options must be a map or object, got {}",
                other.type_name()
            ));
        }
    };
    let known = [
        "workers",
        "queue_capacity",
        "max_requests_per_connection",
        "idle_timeout_ms",
        "write_timeout_ms",
        "max_header_bytes",
        "max_body_bytes",
        "request_timeout_ms",
        "bind",
    ];
    if let Some(unknown) = fields.keys().find(|key| !known.contains(&key.as_str())) {
        return Err(format!("serve: unknown option '{}'", unknown));
    }

    let workers = option_usize(fields, "workers", defaults.workers, 1, 256)?;
    let bind = match fields.get("bind") {
        None => defaults.bind.clone(),
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(other) => {
            return Err(format!(
                "serve: bind must be an address string such as \"0.0.0.0\", got {}",
                other.type_name()
            ));
        }
    };
    Ok(ServeConfig {
        bind,
        workers,
        queue_capacity: option_usize(
            fields,
            "queue_capacity",
            workers.saturating_mul(64).max(64),
            1,
            1_000_000,
        )?,
        max_requests_per_connection: option_usize(
            fields,
            "max_requests_per_connection",
            defaults.max_requests_per_connection,
            1,
            1_000_000,
        )?,
        idle_timeout_ms: option_usize(
            fields,
            "idle_timeout_ms",
            defaults.idle_timeout_ms as usize,
            1,
            3_600_000,
        )? as u64,
        write_timeout_ms: option_usize(
            fields,
            "write_timeout_ms",
            defaults.write_timeout_ms as usize,
            1,
            3_600_000,
        )? as u64,
        max_header_bytes: option_usize(
            fields,
            "max_header_bytes",
            defaults.max_header_bytes,
            1024,
            1024 * 1024,
        )?,
        max_body_bytes: option_usize(
            fields,
            "max_body_bytes",
            defaults.max_body_bytes,
            1024,
            1024 * 1024 * 1024,
        )?,
        request_timeout_ms: option_usize(
            fields,
            "request_timeout_ms",
            defaults.request_timeout_ms as usize,
            100,
            3_600_000,
        )? as u64,
    })
}

fn serve_connection(
    mut stream: std::net::TcpStream,
    interpreter: &mut crate::interpreter::Interpreter,
    handler: &Value,
    config: &ServeConfig,
) {
    use std::io::Write;

    let remote_addr = stream
        .peer_addr()
        .map(|address| address.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(
        config.idle_timeout_ms,
    )));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(
        config.write_timeout_ms,
    )));

    for served in 0..config.max_requests_per_connection {
        match read_request(&mut stream, config) {
            Ok(None) => break,
            Ok(Some(req)) => {
                let client_wants_close = req
                    .headers
                    .iter()
                    .any(|(key, value)| key == "connection" && value.eq_ignore_ascii_case("close"));
                let keep_alive =
                    !client_wants_close && served + 1 < config.max_requests_per_connection;
                let request_value = request_to_value(&req, &remote_addr);
                let fs_grant = interpreter.effective_fs();
                let bytes = match interpreter.call_function(handler.clone(), vec![request_value]) {
                    Ok(result) => render_handler_result(&result, keep_alive, fs_grant),
                    Err(error) => {
                        crate::log::get_logger().error(
                            "http",
                            &format!(
                                "handler failed remote_addr={} method={} path={}: {}",
                                remote_addr, req.method, req.path, error
                            ),
                        );
                        response_bytes(500, "internal server error", &[], keep_alive)
                    }
                };
                if stream.write_all(&bytes).is_err() || stream.flush().is_err() {
                    break;
                }
                if !keep_alive {
                    break;
                }
            }
            Err((status, message)) => {
                let bytes = response_bytes(status, &message, &[], false);
                let _ = stream.write_all(&bytes);
                let _ = stream.flush();
                break;
            }
        }
    }
}

/// The real `http.serve`: a bounded worker-pool HTTP/1.1 server on
/// 127.0.0.1. Each worker owns an interpreter clone, so independent
/// connections execute concurrently without sharing mutable interpreter
/// state. Stateful native handles (such as SQLite connections) provide their
/// own synchronization. The optional third argument configures the pool and
/// connection limits.
pub fn serve_blocking(
    args: Vec<Value>,
    interpreter: &mut crate::interpreter::Interpreter,
) -> Result<Value, crate::interpreter::InterpreterError> {
    use std::io::Write;

    let err_val = |msg: String| Ok(Value::Err(Box::new(Value::String(Arc::new(msg)))));

    if !(2..=3).contains(&args.len()) {
        return err_val(format!(
            "serve expects 2 or 3 arguments (port, handler[, options]), got {}",
            args.len()
        ));
    }
    let port = match &args[0] {
        Value::Integer(p) if (0..=65535).contains(p) => *p as u16,
        _ => return err_val("serve: port must be an integer in 0..=65535".to_string()),
    };
    let handler = args[1].clone();
    if !matches!(handler, Value::Function(_) | Value::Builtin(_)) {
        return err_val("serve: handler must be a function".to_string());
    }
    let config = match serve_config(args.get(2)) {
        Ok(config) => config,
        Err(message) => return err_val(message),
    };

    let listener = match std::net::TcpListener::bind((config.bind.as_str(), port)) {
        Ok(l) => l,
        Err(e) => {
            return err_val(format!(
                "serve: could not bind {}:{}: {}",
                config.bind, port, e
            ));
        }
    };
    // The OS assigns the port when 0 was requested; report the real one.
    let local = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    // These lines are informational; the socket is the server's real
    // interface. A closed stdout (a harness that read the port line and
    // moved on, a dead pipe consumer) must never kill the server — and
    // println! would panic the main thread on exactly that. This race is
    // real: under load a parent reading only the first line closed the
    // pipe before the second write, and the whole server died at boot.
    {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "listening on http://{}:{}", config.bind, local);
        let _ = writeln!(
            out,
            "http workers={} queue_capacity={}",
            config.workers, config.queue_capacity
        );
        let _ = out.flush();
    }

    let (sender, receiver) = std::sync::mpsc::sync_channel(config.queue_capacity);
    let receiver = Arc::new(std::sync::Mutex::new(receiver));
    let mut worker_handles = Vec::with_capacity(config.workers);
    for worker_id in 0..config.workers {
        let receiver = Arc::clone(&receiver);
        let handler = handler.clone();
        let worker_config = config.clone();
        let mut worker_interpreter = interpreter.thread_safe_clone();
        let handle = match std::thread::Builder::new()
            .name(format!("olang-http-{}", worker_id + 1))
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                // In the census while serving: an idle worker counts as
                // live, which correctly disables the stall abort for a
                // program that may still receive external requests.
                let _live = crate::stdlib::chan::live_guard();
                loop {
                    let stream = {
                        let receiver = match receiver.lock() {
                            Ok(receiver) => receiver,
                            Err(_) => return,
                        };
                        match receiver.recv() {
                            Ok(stream) => stream,
                            Err(_) => return,
                        }
                    };
                    serve_connection(stream, &mut worker_interpreter, &handler, &worker_config);
                }
            }) {
            Ok(handle) => handle,
            Err(error) => {
                drop(sender);
                return err_val(format!("serve: could not start worker pool: {}", error));
            }
        };
        worker_handles.push(handle);
    }
    // Only workers should own receiving handles. If every worker exits, the
    // next send then reports Disconnected instead of filling an orphaned
    // queue forever.
    drop(receiver);

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(error) => {
                crate::log::get_logger().warn("http", &format!("accept failed: {}", error));
                continue;
            }
        };
        match sender.try_send(stream) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(mut stream)) => {
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(
                    config.write_timeout_ms,
                )));
                let bytes = response_bytes(503, "server overloaded", &[], false);
                let _ = stream.write_all(&bytes);
                let _ = stream.flush();
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                return Err(crate::interpreter::InterpreterError::RuntimeError {
                    message: "HTTP worker pool stopped unexpectedly".to_string(),
                });
            }
        }
    }

    drop(sender);
    for handle in worker_handles {
        let _ = handle.join();
    }
    Ok(Value::Unit)
}

/// Create an HTTP response struct
/// Usage: http.response(200, "Hello World") -> HttpResponse
fn http_response(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("response expects 2 arguments, got {}", args.len()).into());
    }

    let status = match &args[0] {
        Value::Integer(s) => *s,
        _ => return Err("response: status must be an integer".into()),
    };

    let body = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("response: body must be a string".into()),
    };

    let mut response_map = HashMap::new();
    response_map.insert("status".to_string(), Value::Integer(status));
    response_map.insert(
        "body".to_string(),
        Value::String(Arc::new(body.to_string())),
    );
    response_map.insert(
        "headers".to_string(),
        Value::Struct {
            type_name: "Headers".to_string(),
            fields: std::sync::Arc::new(HashMap::new()),
        },
    );

    Ok(Value::Struct {
        type_name: "HttpResponse".to_string(),
        fields: std::sync::Arc::new(response_map),
    })
}

/// Create an HTTP response with headers
/// Usage: http.response_with_headers(200, "Hello", headers) -> HttpResponse
fn http_response_with_headers(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!(
            "response_with_headers expects 3 arguments, got {}",
            args.len()
        )
        .into());
    }

    let status = match &args[0] {
        Value::Integer(s) => *s,
        _ => return Err("response_with_headers: status must be an integer".into()),
    };

    let body = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("response_with_headers: body must be a string".into()),
    };

    // Headers may be a map literal (`#{ "Content-Type": "..." }`) — the only
    // way to write keys containing `-` — or an anonymous object.
    let headers = match &args[2] {
        Value::Struct { fields, .. } => fields.as_ref().clone(),
        Value::Map(m) => m.as_ref().clone(),
        _ => return Err("response_with_headers: headers must be a map or object".into()),
    };

    let mut response_map = HashMap::new();
    response_map.insert("status".to_string(), Value::Integer(status));
    response_map.insert(
        "body".to_string(),
        Value::String(Arc::new(body.to_string())),
    );
    response_map.insert(
        "headers".to_string(),
        Value::Struct {
            type_name: "Headers".to_string(),
            fields: std::sync::Arc::new(headers),
        },
    );

    Ok(Value::Struct {
        type_name: "HttpResponse".to_string(),
        fields: std::sync::Arc::new(response_map),
    })
}

/// Parse a URL into components
/// Usage: http.parse_url("https://example.com/path?query=value") -> Result<UrlInfo, Error>
fn parse_url(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("parse_url expects 1 argument, got {}", args.len()).into());
    }

    let url_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("parse_url: URL must be a string".to_string().into());
        }
    };

    match url::Url::parse(url_str) {
        Ok(url) => {
            let mut url_map = HashMap::new();
            url_map.insert(
                "scheme".to_string(),
                Value::String(Arc::new(url.scheme().to_string())),
            );
            url_map.insert(
                "host".to_string(),
                Value::String(Arc::new(url.host_str().unwrap_or("").to_string())),
            );
            url_map.insert(
                "port".to_string(),
                Value::Integer(url.port().unwrap_or(0) as i64),
            );
            url_map.insert(
                "path".to_string(),
                Value::String(Arc::new(url.path().to_string())),
            );
            url_map.insert(
                "query".to_string(),
                Value::String(Arc::new(url.query().unwrap_or("").to_string())),
            );
            url_map.insert(
                "fragment".to_string(),
                Value::String(Arc::new(url.fragment().unwrap_or("").to_string())),
            );

            Ok(Value::Ok(Box::new(Value::Struct {
                type_name: "UrlInfo".to_string(),
                fields: std::sync::Arc::new(url_map),
            })))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to parse URL: {}",
            e
        )))))),
    }
}

/// Encode query parameters from a struct
/// Usage: http.encode_query(params_struct) -> Result<String, Error>
fn encode_query(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("encode_query expects 1 argument, got {}", args.len()).into());
    }

    match &args[0] {
        Value::Struct { fields, .. } => {
            let mut query_pairs = Vec::new();
            for (key, value) in fields.iter() {
                let value_str = match value {
                    Value::String(s) => s.as_ref().clone(),
                    Value::Integer(n) => n.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    _ => continue, // Skip non-primitive values
                };
                query_pairs.push(format!(
                    "{}={}",
                    urlencoding::encode(key),
                    urlencoding::encode(&value_str)
                ));
            }
            Ok(Value::Ok(Box::new(Value::String(Arc::new(
                query_pairs.join("&"),
            )))))
        }
        _ => Ok(Value::Err(Box::new(Value::String(Arc::new(
            "encode_query: argument must be a struct".to_string(),
        ))))),
    }
}

/// Decode query string into a struct  
/// Usage: http.decode_query("key1=value1&key2=value2") -> Result<Struct, Error>
fn decode_query(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("decode_query expects 1 argument, got {}", args.len()).into());
    }

    let query_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("decode_query: query string must be a string"
                .to_string()
                .into());
        }
    };

    let mut params = HashMap::new();

    for pair in query_str.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            let decoded_key = urlencoding::decode(key).unwrap_or_default();
            let decoded_value = urlencoding::decode(value).unwrap_or_default();
            params.insert(
                decoded_key.to_string(),
                Value::String(Arc::new(decoded_value.to_string())),
            );
        }
    }

    Ok(Value::Struct {
        type_name: "QueryParams".to_string(),
        fields: std::sync::Arc::new(params),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Value;
    use std::collections::HashMap;
    use std::sync::Arc;

    // Helper functions to create test values
    fn string_val(s: &str) -> Value {
        Value::String(Arc::new(s.to_string()))
    }

    fn int_val(n: i64) -> Value {
        Value::Integer(n)
    }

    fn bool_val(b: bool) -> Value {
        Value::Boolean(b)
    }

    fn struct_val(type_name: &str, fields: HashMap<String, Value>) -> Value {
        Value::Struct {
            type_name: type_name.to_string(),
            fields: std::sync::Arc::new(fields),
        }
    }

    // Helper to assert Result<T, E> success
    fn assert_ok(result: &Value) -> &Value {
        match result {
            Value::Ok(inner) => inner,
            Value::Err(e) => panic!("Expected Ok result, got Err: {:?}", e),
            // 0.64: infallible functions return the value directly.
            bare => bare,
        }
    }

    // Helper to assert Result<T, E> error

    /// Assert a call reports failure, either way it can now.
    ///
    /// 0.64 split the two: misuse (bad arity or type) **raises**, which is
    /// a Rust `Err`; environmental failure still returns `Ok(Value::Err)`.
    /// Tests that only care *that* the call failed use this; tests that
    /// care *which* assert on the specific shape.
    #[allow(dead_code)]
    fn assert_fails(result: Result<Value, Box<dyn std::error::Error>>) {
        match result {
            Err(_) => {}
            Ok(Value::Err(_)) => {}
            Ok(other) => panic!("expected a failure, got: {:?}", other),
        }
    }

    #[allow(dead_code)]
    fn assert_err(result: &Value) -> &Value {
        match result {
            Value::Err(inner) => inner,
            _ => {
                panic!("Expected Err result, got: {:?}", result)
            }
        }
    }

    #[test]
    fn test_create_http_module() {
        let module = create_http_module();
        if let Value::Struct { fields, .. } = module {
            // Test that all expected functions are present
            let expected_functions = vec![
                "get",
                "post",
                "put",
                "delete",
                "request",
                "serve",
                "response",
                "response_with_headers",
                "parse_url",
                "encode_query",
                "decode_query",
            ];

            for func_name in expected_functions {
                assert!(
                    fields.contains_key(func_name),
                    "Missing function: {}",
                    func_name
                );
                if let Value::Builtin(builtin) = &fields[func_name] {
                    assert_eq!(builtin.name, format!("http.{}", func_name));
                } else {
                    panic!(
                        "Expected builtin function for {}, got: {:?}",
                        func_name, fields[func_name]
                    );
                }
            }

            assert_eq!(fields.len(), 11, "Expected 11 functions in http module");
        } else {
            panic!("Expected struct for http module, got: {:?}", module);
        }
    }

    #[test]
    fn test_http_response_creation() {
        // Test basic response creation
        let result = http_response(vec![int_val(200), string_val("Hello World")]).unwrap();

        if let Value::Struct { type_name, fields } = result {
            assert_eq!(type_name, "HttpResponse");
            assert_eq!(fields["status"], int_val(200));
            assert_eq!(fields["body"], string_val("Hello World"));
            assert!(matches!(fields["headers"], Value::Struct { .. }));
        } else {
            panic!("Expected HttpResponse struct, got: {:?}", result);
        }
    }

    #[test]
    fn test_http_response_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), string_val("application/json"));
        headers.insert("X-Custom-Header".to_string(), string_val("test-value"));
        let headers_struct = struct_val("Headers", headers.clone());

        let result =
            http_response_with_headers(vec![int_val(201), string_val("Created"), headers_struct])
                .unwrap();

        if let Value::Struct { type_name, fields } = result {
            assert_eq!(type_name, "HttpResponse");
            assert_eq!(fields["status"], int_val(201));
            assert_eq!(fields["body"], string_val("Created"));

            if let Value::Struct {
                fields: header_fields,
                ..
            } = &fields["headers"]
            {
                assert_eq!(
                    header_fields["Content-Type"],
                    string_val("application/json")
                );
                assert_eq!(header_fields["X-Custom-Header"], string_val("test-value"));
            } else {
                panic!("Expected headers struct, got: {:?}", fields["headers"]);
            }
        } else {
            panic!("Expected HttpResponse struct, got: {:?}", result);
        }
    }

    #[test]
    fn test_parse_url_success() {
        let test_cases = vec![
            (
                "https://example.com:8080/path/to/resource?query=value&foo=bar#section",
                (
                    "https",
                    "example.com",
                    8080,
                    "/path/to/resource",
                    "query=value&foo=bar",
                    "section",
                ),
            ),
            (
                "http://localhost/api",
                ("http", "localhost", 0, "/api", "", ""),
            ),
            (
                "https://api.github.com/repos/owner/repo",
                ("https", "api.github.com", 0, "/repos/owner/repo", "", ""),
            ),
        ];

        for (url, expected) in test_cases {
            let result = parse_url(vec![string_val(url)]).unwrap();
            let parsed = assert_ok(&result);

            if let Value::Struct { fields, .. } = parsed {
                assert_eq!(fields["scheme"], string_val(expected.0));
                assert_eq!(fields["host"], string_val(expected.1));
                assert_eq!(fields["port"], int_val(expected.2));
                assert_eq!(fields["path"], string_val(expected.3));
                assert_eq!(fields["query"], string_val(expected.4));
                assert_eq!(fields["fragment"], string_val(expected.5));
            } else {
                panic!(
                    "Expected UrlInfo struct for URL: {}, got: {:?}",
                    url, result
                );
            }
        }
    }

    #[test]
    fn test_parse_url_invalid() {
        let invalid_urls = vec![
            "not-a-url",
            "ftp://", // incomplete
            "://missing-scheme",
        ];

        for invalid_url in invalid_urls {
            assert_fails(parse_url(vec![string_val(invalid_url)])); // Should be an error
        }
    }

    #[test]
    fn test_encode_query() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), string_val("John Doe"));
        params.insert("age".to_string(), int_val(30));
        params.insert("active".to_string(), bool_val(true));
        params.insert("score".to_string(), Value::Float(85.5));

        let params_struct = struct_val("QueryParams", params);
        let result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&result);

        if let Value::String(query_string) = encoded {
            let query = query_string.as_ref();
            // Query parameters can be in any order, so check individual parts
            assert!(query.contains("name=John%20Doe"));
            assert!(query.contains("age=30"));
            assert!(query.contains("active=true"));
            assert!(query.contains("score=85.5"));
            assert!(query.chars().filter(|&c| c == '&').count() == 3); // 4 params = 3 separators
        } else {
            panic!(
                "Expected string result from encode_query, got: {:?}",
                result
            );
        }
    }

    #[test]
    fn test_decode_query() {
        let query_string = "name=John%20Doe&age=30&city=New%20York&active=true";
        let result = decode_query(vec![string_val(query_string)]).unwrap();
        let decoded = assert_ok(&result);

        if let Value::Struct { fields, .. } = decoded {
            assert_eq!(fields["name"], string_val("John Doe"));
            assert_eq!(fields["age"], string_val("30"));
            assert_eq!(fields["city"], string_val("New York"));
            assert_eq!(fields["active"], string_val("true"));
        } else {
            panic!(
                "Expected QueryParams struct from decode_query, got: {:?}",
                result
            );
        }
    }

    #[test]
    fn test_decode_query_empty() {
        let result = decode_query(vec![string_val("")]).unwrap();
        let decoded = assert_ok(&result);

        if let Value::Struct { fields, .. } = decoded {
            assert!(fields.is_empty());
        } else {
            panic!("Expected empty QueryParams struct, got: {:?}", result);
        }
    }

    #[test]
    fn test_http_serve_requires_interpreter_dispatch() {
        // The interpreter-less path must refuse: real serving happens in
        // serve_blocking, dispatched with the interpreter in builtin.rs.
        let result = http_serve(vec![int_val(8080), string_val("handler")]);
        assert!(result.is_err());
    }

    #[test]
    fn test_response_bytes_shape() {
        let bytes = response_bytes(404, "missing", &[], false);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(text.contains("Content-Length: 7\r\n"));
        assert!(text.contains("Content-Type: text/plain"));
        assert!(text.ends_with("\r\n\r\nmissing"));
    }

    #[test]
    fn test_response_bytes_honors_custom_content_type() {
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        let text = String::from_utf8(response_bytes(200, "{}", &headers, false)).unwrap();
        assert!(text.contains("Content-Type: application/json\r\n"));
        // The default must not ALSO be emitted.
        assert!(!text.contains("text/plain"));
    }

    #[test]
    fn test_render_handler_result_bare_string_is_200() {
        let text = String::from_utf8(render_handler_result(
            &string_val("hi"),
            false,
            crate::caps::FsCap::Full,
        ))
        .unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.ends_with("hi"));
    }

    #[test]
    fn test_render_handler_result_uses_response_struct() {
        let resp = http_response(vec![int_val(201), string_val("made")]).unwrap();
        let text = String::from_utf8(render_handler_result(
            &resp,
            false,
            crate::caps::FsCap::Full,
        ))
        .unwrap();
        assert!(text.starts_with("HTTP/1.1 201 Created\r\n"));
        assert!(text.ends_with("made"));
    }

    #[test]
    fn test_render_handler_result_bytes_body_is_raw() {
        let mut fields = HashMap::new();
        fields.insert("status".to_string(), int_val(200));
        fields.insert(
            "body".to_string(),
            crate::stdlib::bytes::to_value(vec![0, 159, 146, 150]),
        );
        let resp = Value::Struct {
            type_name: "Response".to_string(),
            fields: std::sync::Arc::new(fields),
        };
        let out = render_handler_result(&resp, false, crate::caps::FsCap::Full);
        assert!(
            out.ends_with(&[0, 159, 146, 150]),
            "body bytes must pass through untouched"
        );
        let head = String::from_utf8_lossy(&out);
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head.contains("Content-Length: 4\r\n"));
        assert!(head.contains("Content-Type: application/octet-stream\r\n"));
    }

    /// S3: `body_file` reads a file, so `net` (which gates `http`) must not
    /// be a latent file-read capability. Under `fs = None` the read is
    /// refused with a 403 instead of serving the file's bytes.
    #[test]
    fn test_body_file_denied_without_fs() {
        let mut fields = HashMap::new();
        fields.insert("status".to_string(), int_val(200));
        fields.insert("body_file".to_string(), string_val("/etc/hostname"));
        let resp = Value::Struct {
            type_name: "Response".to_string(),
            fields: std::sync::Arc::new(fields),
        };
        // fs = None: denied.
        let denied = String::from_utf8(render_handler_result(
            &resp,
            false,
            crate::caps::FsCap::None,
        ))
        .unwrap();
        assert!(
            denied.starts_with("HTTP/1.1 403"),
            "expected 403, got: {denied}"
        );
        assert!(denied.contains("capability 'fs' denied"));
        // fs = Read: the read is permitted (the file may or may not exist,
        // but the capability gate does not refuse it).
        let allowed = String::from_utf8(render_handler_result(
            &resp,
            false,
            crate::caps::FsCap::Read,
        ))
        .unwrap();
        assert!(!allowed.contains("capability 'fs' denied"));
    }

    #[test]
    fn test_argument_validation_errors() {
        // Test various argument validation errors

        // http_get - wrong number of arguments
        assert_fails(http_get(vec![]));

        assert_fails(http_get(vec![string_val("url"), string_val("extra")]));

        // http_get - wrong argument type
        assert_fails(http_get(vec![int_val(123)]));

        // http_post - wrong number of arguments
        assert_fails(http_post(vec![string_val("url")]));

        // http_post - wrong argument types
        assert_fails(http_post(vec![int_val(123), string_val("body")]));

        assert_fails(http_post(vec![string_val("url"), int_val(123)]));

        // http_request - wrong number of arguments
        assert_fails(http_request(vec![string_val("GET"), string_val("url")]));

        // http_request - unsupported method
        assert_fails(http_request(vec![
            string_val("INVALID"),
            string_val("http://example.com"),
            string_val(""),
        ]));

        // parse_url - wrong number of arguments
        assert_fails(parse_url(vec![]));

        // parse_url - wrong argument type
        assert_fails(parse_url(vec![int_val(123)]));

        // encode_query - wrong argument type
        assert_fails(encode_query(vec![string_val("not a struct")]));

        // decode_query - wrong argument type
        assert_fails(decode_query(vec![int_val(123)]));

        // http_serve — the interpreter-less path always refuses (real serving
        // is dispatched to serve_blocking with the interpreter)
        assert!(http_serve(vec![int_val(8080)]).is_err());
        assert!(http_serve(vec![string_val("not a port"), string_val("handler")]).is_err());
    }

    #[test]
    fn test_response_creation_errors() {
        // Test http_response with wrong arguments
        let result = http_response(vec![string_val("not an int"), string_val("body")]);
        assert!(result.is_err());

        let result = http_response(vec![int_val(200), int_val(123)]);
        assert!(result.is_err());

        let result = http_response(vec![int_val(200)]);
        assert!(result.is_err());

        // Test http_response_with_headers with wrong arguments
        let result = http_response_with_headers(vec![
            string_val("not an int"),
            string_val("body"),
            struct_val("Headers", HashMap::new()),
        ]);
        assert!(result.is_err());

        let result = http_response_with_headers(vec![
            int_val(200),
            int_val(123),
            struct_val("Headers", HashMap::new()),
        ]);
        assert!(result.is_err());

        let result = http_response_with_headers(vec![
            int_val(200),
            string_val("body"),
            string_val("not a struct"),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_call_http_function_dispatcher() {
        // Test that the dispatcher correctly routes to functions
        let result = call_http_function("response", vec![int_val(200), string_val("OK")]);
        assert!(result.is_ok());

        let result = call_http_function("parse_url", vec![string_val("http://example.com")]);
        assert!(result.is_ok());

        // Test unknown function
        let result = call_http_function("unknown_function", vec![]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown http function")
        );
    }

    #[test]
    fn test_http_request_method_handling() {
        let supported_methods = vec!["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"];

        for method in supported_methods {
            let result = http_request(vec![
                string_val(method),
                string_val("http://httpbin.org/anything"), // This would fail in actual request
                string_val(""),
            ])
            .unwrap();

            // We expect this to be an error due to network, but not due to unsupported method
            if let Value::Err(error) = result
                && let Value::String(error_msg) = error.as_ref()
            {
                // Should not contain "Unsupported HTTP method"
                assert!(!error_msg.contains("Unsupported HTTP method"));
            }
        }
    }

    #[test]
    fn test_encode_query_with_special_characters() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), string_val("John & Jane"));
        params.insert("email".to_string(), string_val("test@example.com"));
        params.insert("path".to_string(), string_val("/path/with spaces"));

        let params_struct = struct_val("QueryParams", params);
        let result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&result);

        if let Value::String(query_string) = encoded {
            let query = query_string.as_ref();
            assert!(query.contains("John%20%26%20Jane")); // URL-encoded "John & Jane"
            assert!(query.contains("test%40example.com")); // URL-encoded "test@example.com"
            assert!(query.contains("%2Fpath%2Fwith%20spaces")); // URL-encoded "/path/with spaces"
        } else {
            panic!(
                "Expected string result from encode_query, got: {:?}",
                encoded
            );
        }
    }

    #[test]
    fn test_decode_query_with_special_characters() {
        let query_string =
            "name=John%20%26%20Jane&email=test%40example.com&path=%2Fpath%2Fwith%20spaces";
        let result = decode_query(vec![string_val(query_string)]).unwrap();
        let decoded = assert_ok(&result);

        if let Value::Struct { fields, .. } = decoded {
            assert_eq!(fields["name"], string_val("John & Jane"));
            assert_eq!(fields["email"], string_val("test@example.com"));
            assert_eq!(fields["path"], string_val("/path/with spaces"));
        } else {
            panic!(
                "Expected QueryParams struct from decode_query, got: {:?}",
                decoded
            );
        }
    }

    #[test]
    fn test_query_encode_decode_roundtrip() {
        let mut original_params = HashMap::new();
        original_params.insert("name".to_string(), string_val("Test User"));
        original_params.insert("age".to_string(), int_val(25));
        original_params.insert("active".to_string(), bool_val(true));

        let params_struct = struct_val("QueryParams", original_params.clone());

        // Encode
        let encoded_result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&encoded_result);

        // Decode
        let decoded_result = decode_query(vec![encoded.clone()]).unwrap();
        let decoded = assert_ok(&decoded_result);

        if let Value::Struct { fields, .. } = decoded {
            assert_eq!(fields["name"], string_val("Test User"));
            assert_eq!(fields["age"], string_val("25")); // Numbers become strings in decode
            assert_eq!(fields["active"], string_val("true")); // Booleans become strings in decode
        } else {
            panic!(
                "Expected QueryParams struct from roundtrip, got: {:?}",
                decoded
            );
        }
    }

    #[test]
    fn test_encode_query_skips_complex_types() {
        let mut params = HashMap::new();
        params.insert("simple_string".to_string(), string_val("hello"));
        params.insert("simple_int".to_string(), int_val(42));
        params.insert(
            "complex_struct".to_string(),
            struct_val("ComplexType", HashMap::new()),
        );
        params.insert(
            "list".to_string(),
            Value::List(Arc::new(vec![string_val("item1"), string_val("item2")])),
        );

        let params_struct = struct_val("QueryParams", params);
        let result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&result);

        if let Value::String(query_string) = encoded {
            let query = query_string.as_ref();
            assert!(query.contains("simple_string=hello"));
            assert!(query.contains("simple_int=42"));
            // Complex types should be skipped
            assert!(!query.contains("complex_struct"));
            assert!(!query.contains("list"));
        } else {
            panic!(
                "Expected string result from encode_query, got: {:?}",
                encoded
            );
        }
    }
}
