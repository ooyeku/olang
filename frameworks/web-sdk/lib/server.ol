//! server — the whole backend in one call.
//!
//! `serve(config)` takes a route table and runs the full stack: JSON
//! envelopes, method-aware 405s, request logging, static assets, and
//! the wasm frontend — the HTML shell, the design system, the dom
//! shim, and the client program are all served by the same process.
//!
//! The client program is *bundled* on the way out: the browser loads
//! one source file, so `serve` splices the SDK's browser-side modules
//! (html, ui, forms, state, view, api) ahead of the app's client code
//! and strips the `use web` line — the app writes imports as if the
//! browser resolved them, and the server makes that true.
//!
//! Responses use one envelope everywhere: `{ "data": ... }` on
//! success, `{ "error": { "code", "message", "details?" } }` on
//! failure — because the browser's fetch surfaces no status codes,
//! the body must carry the whole truth.

use lib.routes { find }
use lib.html { page, raw }

// ── responses ────────────────────────────────────────────────────────

/// A JSON response at `status` — the raw form when an endpoint needs
/// full control.
share fn json_response(status, value) =
    http.response_with_headers(status, unwrap(json.stringify(value)),
        #{ "Content-Type": "application/json" })

/// Success: `{ "data": value }` at 200.
share fn ok_data(value) = json_response(200, #{ "data": value })

/// The one error shape every failure wears.
share fn error_response(status, code, message) =
    json_response(status, #{ "error": #{ "code": code, "message": message } })

/// Validation failures: 422 with the problem list from
/// `validate.check` in `details`.
share fn invalid(details) =
    json_response(422, #{ "error": #{ "code": "invalid", "message": "validation failed",
        "details": details } })

// ── request helpers ──────────────────────────────────────────────────

/// The request body as JSON: `Ok(value)` or `Err(response)` ready to
/// return — `match body_json(req) { Err(resp) => resp, Ok(v) => ... }`.
share fn body_json(req) = match json.parse(req.body) {
    Ok(v) => Ok(v),
    Err(e) => Err(error_response(400, "bad_json", "body must be JSON"))
}

/// A query value with a fallback.
share fn q_str(req, key, fallback) =
    if map_has_key(req.query, key) => map_get(req.query, key) else => fallback

/// A clamped integer query value.
share fn q_int(req, key, fallback, lo, hi) = {
    if !map_has_key(req.query, key) => fallback
    else => match str.parse_int(map_get(req.query, key)) {
        Ok(n) => clamp(n, lo, hi),
        Err(e) => fallback
    }
}

/// One of an allowed set, else the fallback — nothing from the wire
/// reaches SQL as an identifier.
share fn q_enum(req, key, allowed, fallback) = {
    let v = q_str(req, key, fallback)
    if contains(allowed, v) => v else => fallback
}

// ── dispatch ─────────────────────────────────────────────────────────

fn is_response(v) = {
    // A response is the struct http.response builds (or a map wearing
    // its shape); every plain data kind — parsed JSON included — wraps
    // in the envelope instead.
    let t = typeof(v)
    if t == "Map" || t == "JsonObject" => map_has_key(v, "status")
    else => !contains(["Int", "Float", "String", "Bool", "List", "Unit", "Tuple"], t)
}

/// Dispatch a request against a route table. A handler may return a
/// response, `Err(message)` (a clean 500 envelope), or any plain
/// value — wrapped as `{ "data": value }`, which is what makes rpc
/// handlers one-liners.
share fn dispatch(routes, req) = {
    let started = time.monotonic_ms()
    let outcome = find(routes, req.method, req.path)
    let hit = map_get(outcome, "hit")
    let response = if map_get(hit, "found") => {
        let h = map_get(hit, "handler")
        let result = h(req, map_get(hit, "params"))
        match result {
            Err(message) => error_response(500, "internal", show(message)),
            Ok(v) => if is_response(v) => v else => ok_data(v),
            v => if is_response(v) => v else => ok_data(v)
        }
    } else => if len(map_get(outcome, "allowed")) > 0 =>
        http.response_with_headers(405,
            unwrap(json.stringify(#{ "error": #{ "code": "method_not_allowed",
                "message": req.method + " not allowed here",
                "allow": map_get(outcome, "allowed") } })),
            #{ "Content-Type": "application/json",
               "Allow": join(map_get(outcome, "allowed"), ", ") })
    else =>
        error_response(404, "not_found", "no route for " + req.method + " " + req.path)
    let ms = time.monotonic_ms() - started
    println(req.method + " " + req.path + " -> " + show(response.status) + " (" + show(ms) + "ms)")
    response
}

// ── the SDK's own files (for bundling and assets) ────────────────────

/// Where the SDK lives on this machine: `WEB_SDK_DIR`, the shelf's
/// registration, or the in-repo path — the same search order a user's
/// mental model has.
share fn sdk_dir() = {
    match os.get_env("WEB_SDK_DIR") {
        Ok(dir) => dir,
        Err(e) => {
            let home = match os.get_env("HOME") { Ok(h) => h, Err(e2) => "" }
            let from_shelf = match fs.read_file(home + "/.olang/shelf.toml") {
                Err(e3) => (),
                Ok(text) => match toml.parse(text) {
                    Err(e4) => (),
                    Ok(t) => {
                        let libs = map_get(t, "libraries")
                        if libs != () && map_has_key(libs, "web") => map_get(libs, "web")
                        else => ()
                    }
                }
            }
            if from_shelf != () => from_shelf
            else if fs.exists("frameworks/web-sdk/olang.toml") => "frameworks/web-sdk"
            else if fs.exists("lib/html.ol") => "."
            else if fs.exists("../lib/html.ol") => ".."
            else => "."
        }
    }
}

fn sdk_file(rel) = unwrap(fs.read_file(sdk_dir() + "/" + rel))

// ── the client bundle ────────────────────────────────────────────────

/// The SDK's browser-side modules, in dependency order. `share ` and
/// intra-SDK `use lib.*` lines are stripped: the bundle is one flat
/// program, so the names resolve by concatenation.
fn browser_modules() = ["lib/html.ol", "lib/forms.ol", "lib/ui.ol",
                        "lib/state.ol", "lib/view.ol", "lib/api.ol"]

fn strip_module_lines(source) = {
    let mut out = []
    let mut in_use = false
    let mut in_test = false
    for line in str.lines(source) {
        let t = str.trim(line)
        if in_test => {
            // A test block runs as ordinary statements when its module
            // is spliced, so the bundle drops them: from a top-level
            // `test "..." {` through the column-0 closing brace (the
            // SDK's own brace style; client code follows the same
            // convention).
            if line == "}" => { in_test = false }
        }
        else if str.starts_with(line, "test ") && str.ends_with(t, "{") => {
            in_test = true
        }
        else if in_use => {
            // Inside a multi-line `use lib.x { ... }`: skip through the
            // closing brace.
            if str.contains(t, "}") => { in_use = false }
        }
        else if str.starts_with(t, "use lib.") || str.starts_with(t, "use web")
            || str.starts_with(t, "share use ") => {
            if str.contains(t, "{") && !str.contains(t, "}") => { in_use = true }
        }
        else if str.starts_with(t, "//!") => ()
        else => {
            let ls = str.trim_start(line)
            let kept = if str.starts_with(ls, "share fn ") =>
                str.replace(line, "share fn ", "fn ")
            else if str.starts_with(ls, "share let ") =>
                str.replace(line, "share let ", "let ")
            else => line
            out = out + [kept]
        }
    }
    join(out, "\n")
}

/// The client program the browser actually loads: the SDK's browser
/// modules spliced ahead of the app's client source, with its
/// `use web { ... }` line removed (the bundle makes it true). The
/// result is parse-checked — a broken bundle fails loudly at the
/// server, never as a blank page.
share fn bundle_client(client_source) = {
    let sdk = browser_modules()
        |> map((m) => strip_module_lines(sdk_file(m)))
        |> join("\n\n")
    let app_src = strip_module_lines(client_source)
    let bundled = sdk + "\n\n// ── application ──\n" + app_src
    match meta.parse(bundled) {
        Err(e) => unwrap(Err("client bundle does not parse: " + e)),
        Ok(tree) => bundled
    }
}

// ── the app ──────────────────────────────────────────────────────────

fn content_type_for(path_str) =
    if str.ends_with(path_str, ".css") => "text/css"
    else if str.ends_with(path_str, ".js") => "application/javascript"
    else if str.ends_with(path_str, ".ol") => "text/plain; charset=utf-8"
    else if str.ends_with(path_str, ".svg") => "image/svg+xml"
    else if str.ends_with(path_str, ".png") => "image/png"
    else => "application/octet-stream"

fn text_response(body, ctype) =
    http.response_with_headers(200, body, #{ "Content-Type": ctype })

/// The full stack, one call. Config keys:
///
///   title    the page title (default "olang app")
///   routes   the route table (route(...) and rpc(...) entries)
///   client   path of the app's client source (default "client.ol";
///            "" for an API-only server)
///   port     default 7500
///   head     extra HTML for <head> (default "")
///
/// Serves `/` (the shell), `/web.css`, `/olang-dom.js`, `/app.ol`
/// (the bundled client), `/olang.wasm`, and every route in the table.
/// Blocks serving; returns only on failure to bind.
share fn serve(config) = {
    let title = get_or(config, "title", "olang app")
    let user_routes = get_or(config, "routes", [])
    let client_path = get_or(config, "client", "client.ol")
    let port = get_or(config, "port", 7500)
    let head = get_or(config, "head", "")

    // Read once at boot: assets, the bundle, and the wasm's location.
    let css = sdk_file("static/web.css")
    let shim = sdk_file("static/olang-dom.js")
    let bundle = if client_path == "" => ""
        else => bundle_client(unwrap(fs.read_file(client_path)))
    let wasm_path = if fs.exists("static/olang_playground.wasm") =>
        "static/olang_playground.wasm"
    else => sdk_dir() + "/static/olang_playground.wasm"
    if client_path != "" && !fs.exists(wasm_path) => {
        println("WARNING: olang_playground.wasm not found — the frontend cannot boot.")
        println("Build and copy it:")
        println("  cargo build -p olang-playground --target wasm32-unknown-unknown --release")
        println("  cp target/wasm32-unknown-unknown/release/olang_playground.wasm static/")
    }

    let shell = page(title,
        "<link rel=\"stylesheet\" href=\"/web.css\">" + head,
        raw("<main id=\"app\"></main>"
            + "<script type=\"module\" src=\"/olang-dom.js\" data-src=\"/app.ol\"></script>"))

    let static_routes = [
        #{ "method": "GET", "pattern": "/", "name": "",
           "handler": (req, p) => text_response(shell, "text/html; charset=utf-8") },
        #{ "method": "GET", "pattern": "/web.css", "name": "",
           "handler": (req, p) => text_response(css, "text/css") },
        #{ "method": "GET", "pattern": "/olang-dom.js", "name": "",
           "handler": (req, p) => text_response(shim, "application/javascript") },
        #{ "method": "GET", "pattern": "/app.ol", "name": "",
           "handler": (req, p) => text_response(bundle, "text/plain; charset=utf-8") },
        #{ "method": "GET", "pattern": "/olang.wasm", "name": "",
           "handler": (req, p) => {
               status: 200,
               body_file: wasm_path,
               headers: #{ "Content-Type": "application/wasm" }
           } }
    ]
    let table = static_routes + user_routes

    println(title + " listening on http://127.0.0.1:" + to_string(port))
    match http.serve(port, (req) => dispatch(table, req)) {
        Err(e) => {
            println("could not start on port " + show(port) + ": " + show(e))
            Err(e)
        },
        Ok(v) => Ok(v)
    }
}

fn get_or(m, key, fallback) =
    if map_has_key(m, key) => map_get(m, key) else => fallback

fn fake_req(method, path) =
    { method: method, path: path, query: #{}, headers: #{}, body: "" }

fn fake_req_body(method, path, body) =
    { method: method, path: path, query: #{}, headers: #{}, body: body }

test "dispatch: hit, wrap, 404, 405 with Allow, Err becomes 500" {
    let table = [
        #{ "method": "GET", "pattern": "/plain", "name": "",
           "handler": (req, p) => 41 + 1 },
        #{ "method": "GET", "pattern": "/resp", "name": "",
           "handler": (req, p) => json_response(201, #{ "made": true }) },
        #{ "method": "POST", "pattern": "/only-post", "name": "",
           "handler": (req, p) => "posted" },
        #{ "method": "GET", "pattern": "/boom", "name": "",
           "handler": (req, p) => Err("db exploded") }
    ]
    // A plain value wraps as { data } at 200.
    let r1 = dispatch(table, fake_req("GET", "/plain"))
    assert_eq(r1.status, 200)
    assert_eq(str.contains(r1.body, "\"data\":42"), true)
    // A full response passes through untouched.
    assert_eq(dispatch(table, fake_req("GET", "/resp")).status, 201)
    // No route: 404 envelope.
    let r404 = dispatch(table, fake_req("GET", "/missing"))
    assert_eq(r404.status, 404)
    assert_eq(str.contains(r404.body, "not_found"), true)
    // Wrong method on a real path: 405 naming what IS allowed.
    let r405 = dispatch(table, fake_req("GET", "/only-post"))
    assert_eq(r405.status, 405)
    assert_eq(map_get(r405.headers, "Allow"), "POST")
    // A handler's Err is a clean 500 envelope, not a dropped socket.
    let r500 = dispatch(table, fake_req("GET", "/boom"))
    assert_eq(r500.status, 500)
    assert_eq(str.contains(r500.body, "db exploded"), true)
}

test "dispatch: HEAD rides GET; params reach the handler" {
    let table = [
        #{ "method": "GET", "pattern": "/t/:id", "name": "",
           "handler": (req, p) => "id=" + map_get(p, "id") }
    ]
    assert_eq(dispatch(table, fake_req("HEAD", "/t/9")).status, 200)
    let r = dispatch(table, fake_req("GET", "/t/9"))
    assert_eq(str.contains(r.body, "id=9"), true)
}

test "body_json: parsed or a ready 400" {
    let good = body_json(fake_req_body("POST", "/x", "{\"a\": 1}"))
    assert_eq(is_ok(good), true)
    assert_eq(map_get(unwrap(good), "a"), 1)
    let bad = body_json(fake_req_body("POST", "/x", "not json"))
    assert_eq(is_ok(bad), false)
    let status = match bad { Err(resp) => resp.status, Ok(v) => 0 }
    assert_eq(status, 400)
}

test "query helpers clamp, fall back, and gate enums" {
    let req = { method: "GET", path: "/", headers: #{}, body: "",
                query: #{ "n": "50", "bad": "x", "sort": "title" } }
    assert_eq(q_int(req, "n", 10, 0, 30), 30)
    assert_eq(q_int(req, "bad", 10, 0, 99), 10)
    assert_eq(q_int(req, "absent", 7, 0, 99), 7)
    assert_eq(q_enum(req, "sort", ["title", "id"], "id"), "title")
    assert_eq(q_enum(req, "sort2", ["title", "id"], "id"), "id")
    let evil = { method: "GET", path: "/", headers: #{}, body: "",
                 query: #{ "sort": "title; DROP TABLE" } }
    assert_eq(q_enum(evil, "sort", ["title", "id"], "id"), "id")
}

test "the envelope shapes" {
    let ok = ok_data(#{ "n": 1 })
    assert_eq(ok.status, 200)
    assert_eq(str.contains(ok.body, "\"data\""), true)
    let err = error_response(404, "not_found", "gone")
    assert_eq(err.status, 404)
    assert_eq(str.contains(err.body, "\"not_found\""), true)
    let inv = invalid(["title: required"])
    assert_eq(inv.status, 422)
    assert_eq(str.contains(inv.body, "details"), true)
}

test "strip_module_lines flattens a module for the bundle" {
    let src = "//! doc\nuse lib.html { div }\nshare fn f(x) = x\nfn g(y) = y\n"
    assert_eq(strip_module_lines(src), "fn f(x) = x\nfn g(y) = y")
    // A multi-line use block strips whole.
    let multi = "use lib.html {\n    div, span\n}\nshare fn h(x) = x"
    assert_eq(strip_module_lines(multi), "fn h(x) = x")
    // Test blocks strip: they would execute as statements in a bundle.
    let tested = "fn a(x) = x\ntest \"t\" {\n    assert_eq(a(1), 1)\n}\nfn b(y) = y"
    assert_eq(strip_module_lines(tested), "fn a(x) = x\nfn b(y) = y")
}

test "content types" {
    assert_eq(content_type_for("a/web.css"), "text/css")
    assert_eq(content_type_for("x.wasm.bak"), "application/octet-stream")
}
