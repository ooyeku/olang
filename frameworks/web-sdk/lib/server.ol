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
use lib.html { page, raw, render, div }

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

/// The row you named is not here: 404 with `what` in the message —
/// `not_found("issue " + id)`. `Err(message)` from a handler is a 500,
/// which is for failures; an absent row is an answer.
share fn not_found(what) = error_response(404, "not_found", what + " not found")

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
share fn dispatch(routes, req) = dispatch_with(routes, req, ())

/// `dispatch` with the request line under the app's control: `log` is
/// `(req, response, ms) => ...` — or Unit for the default line. Passed
/// as a value (never a cell: handlers run on worker threads), so an app
/// with its own structured logging owns every line of its output.
share fn dispatch_with(routes, req, log) = {
    let started = time.monotonic_ms()
    let outcome = find(routes, req.method, req.path)
    let hit = map_get(outcome, "hit")
    let response = if map_get(hit, "found") => {
        let h = map_get(hit, "handler")
        let params = map_get(hit, "params")
        // Under `olang test` the handler runs on a task thread, the way
        // `serve`'s workers run it: a handler that captured a cell then
        // fails in the test that exercises it ("cell escaped its
        // thread"), not in production. A raise on the task arrives as
        // the Err the 500 envelope already handles.
        let result = if under_test() => task.join(spawn { h(req, params) })
            else => h(req, params)
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
    if log == () =>
        println(req.method + " " + req.path + " -> " + show(response.status) + " (" + show(ms) + "ms)")
    else => log(req, response, ms)
    response
}

fn under_test() = match os.get_env("OLANG_TEST") { Ok(v) => v != "", Err(e) => false }

// ── the SDK's own files (for bundling and assets) ────────────────────

/// Where the SDK lives on this machine: `WEB_SDK_DIR`, the project's
/// own resolution of `web` (its `olang.lock`), the shelf's registration,
/// or the in-repo path — the project's answer before the machine's.
share fn sdk_dir() = {
    match os.get_env("WEB_SDK_DIR") {
        Ok(dir) => dir,
        Err(e) => {
            let from_lock = locked_web_dir()
            if from_lock != () => from_lock
            else => sdk_dir_from_machine()
        }
    }
}

/// The directory `olang.lock` (in the working directory) resolved `web`
/// to — a path entry relative to the manifest, or a shelf entry looked up
/// on this machine's shelf. Unit when the lock says nothing.
fn locked_web_dir() = {
    match fs.read_file("olang.lock") {
        Err(e) => (),
        Ok(text) => match toml.parse(text) {
            Err(e2) => (),
            Ok(lock) => {
                let packages = map_get(lock, "package")
                let entry = if packages == () => () else => map_get(packages, "web")
                let source = if entry == () => () else => map_get(entry, "source")
                if source == () => ()
                else if map_get(source, "kind") == "path" => {
                    let p = map_get(source, "path")
                    if fs.exists(p + "/olang.toml") => p else => ()
                }
                else if map_get(source, "kind") == "shelf" => shelf_dir(map_get(source, "shelf"))
                else => ()
            }
        }
    }
}

/// A shelf library's directory, or Unit.
fn shelf_dir(name) = {
    let home = match os.get_env("HOME") { Ok(h) => h, Err(e) => "" }
    match fs.read_file(home + "/.olang/shelf.toml") {
        Err(e) => (),
        Ok(text) => match toml.parse(text) {
            Err(e2) => (),
            Ok(t) => {
                let libs = map_get(t, "libraries")
                if libs != () && map_has_key(libs, name) => map_get(libs, name) else => ()
            }
        }
    }
}

fn sdk_dir_from_machine() = {
    {
        {
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
                        "lib/state.ol", "lib/view.ol", "lib/api.ol",
                        "lib/store.ol"]

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
share fn bundle_client(client_source) = bundle_clients([client_source])

/// Several client modules, one bundle: each source is stripped the same
/// way (its `use` lines, `share`, and test blocks) and spliced in list
/// order after the SDK's modules — so a browser helper can live in a
/// tested lib module and be imported by server code too.
share fn bundle_clients(client_sources) = bundle_clients_in(sdk_dir(), client_sources)

/// `bundle_clients` with the SDK read from `dir` — what `serve` uses when
/// its config names an `"sdk_dir"`.
share fn bundle_clients_in(dir, client_sources) = {
    let sdk = browser_modules()
        |> map((m) => strip_module_lines(unwrap(fs.read_file(dir + "/" + m))))
        |> join("\n\n")
    let app_src = client_sources
        |> map((src) => strip_module_lines(src))
        |> join("\n\n// ── module ──\n")
    let bundled = sdk + "\n\n// ── application ──\n" + app_src
    // Expanded here, once, so the browser parses the bundle exactly once:
    // a bundle carrying `meta fn` declarations and `@` sites otherwise
    // costs the wasm parser several complete passes at boot — parsing
    // was the whole of a 4.5 s session start once the download was
    // solved.
    let expanded = match meta.expand(bundled) {
        Err(e) => unwrap(Err("client bundle does not expand: " + e)),
        Ok(text) => text
    }
    match meta.parse(expanded) {
        Err(e) => unwrap(Err("client bundle does not parse: " + e)),
        Ok(tree) => expanded
    }
}

test "the served bundle is pre-expanded: no meta fn, no @ sites" {
    let bundle = bundle_client("use web { mount }\nlet SPEC = @store(\"t\", [[\"n\", \"mem\", 0]])\nprintln(\"x\")")
    assert_eq(str.contains(bundle, "@store"), false)
    assert_eq(str.contains(bundle, "meta fn"), false)
    assert_eq(str.contains(bundle, "println(\"x\")"), true)
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

// ── static asset caching ─────────────────────────────────────────────
// Every static asset carries an ETag and `no-cache` (use the cache,
// but revalidate): an unchanged asset answers 304 with an empty body,
// which matters most for the multi-megabyte wasm the browser would
// otherwise re-download on every load.

fn etag_of(content) = "\"" + str.substring(crypto.sha256(content), 0, 16) + "\""

fn if_none_match(req) =
    if map_has_key(req.headers, "if-none-match") =>
        map_get(req.headers, "if-none-match")
    else => ""

/// A cache-validated response: 304 when the client already holds this
/// exact content, the full body (with its ETag) otherwise.
share fn static_response(req, body, ctype, tag) =
    if if_none_match(req) == tag =>
        http.response_with_headers(304, "",
            #{ "ETag": tag, "Cache-Control": "no-cache" })
    else => http.response_with_headers(200, body,
        #{ "Content-Type": ctype, "ETag": tag, "Cache-Control": "no-cache" })

// ── the shell ────────────────────────────────────────────────────────

/// The HTML shell. `wasm_url` is the runtime the shim fetches (the
/// content-addressed form); `has_image` says whether `/app.olb`, the
/// program image, is served. With a `first_paint` node tree and its
/// `state`, the mount point arrives already rendered — the first paint
/// needs no runtime — and the state travels beside it in a JSON
/// `<script>`, named by the mount point's `data-olang-state`, so the
/// browser's `mount` starts from exactly what the page shows.
share fn shell(title, head, wasm_url, has_image, first_paint, state) = {
    let app = if first_paint == () => "<main id=\"app\"></main>"
        else => "<main id=\"app\" data-olang-state=\"olang-state\">"
            + render(first_paint) + "</main>"
            + "<script type=\"application/json\" id=\"olang-state\">"
            + state_json(state) + "</script>"
    // The preload names the URL the shim will actually fetch — the same
    // credentials mode as fetch()'s default, so the browser reuses it
    // rather than downloading the runtime twice.
    page(title,
        "<link rel=\"stylesheet\" href=\"/web.css\">"
            + "<link rel=\"preload\" href=\"" + wasm_url
            + "\" as=\"fetch\" type=\"application/wasm\" crossorigin>" + head,
        raw(app
            + "<script type=\"module\" src=\"/olang-dom.js\" data-src=\"/app.ol\""
            + (if has_image => " data-bin=\"/app.olb\"" else => "")
            + " data-wasm=\"" + wasm_url + "\"></script>"))
}

/// The state as JSON that is safe inside a `<script>` element: `<` is
/// escaped, so no string value can close the element early.
fn state_json(state) = match json.stringify(state) {
    Ok(text) => str.replace(text, "<", "\\u003c"),
    Err(e) => unwrap(Err("first paint: the initial state must be JSON data "
        + "(maps, lists, strings, numbers, booleans): " + show(e)))
}

test "the shell carries the first paint and its state when given a view" {
    let s = shell("t", "", "/olang.abc.wasm", true,
        div(#{ "class": "x" }, ["hello"]), #{ "n": 1, "s": "</script><" })
    assert_eq(str.contains(s,
        "<main id=\"app\" data-olang-state=\"olang-state\"><div class=\"x\">hello</div></main>"), true)
    assert_eq(str.contains(s,
        "<script type=\"application/json\" id=\"olang-state\">{\"n\":1,\"s\":\"\\u003c/script>\\u003c\"}</script>"), true)
    assert_eq(str.contains(s, "data-bin=\"/app.olb\""), true)
    assert_eq(str.contains(s, "data-wasm=\"/olang.abc.wasm\""), true)
    assert_eq(str.contains(s, "href=\"/olang.abc.wasm\" as=\"fetch\""), true)
    // No view: an empty mount point, no state, and no image offered.
    let bare = shell("t", "<meta name=\"x\">", "/olang.wasm", false, (), ())
    assert_eq(str.contains(bare, "<main id=\"app\"></main>"), true)
    assert_eq(str.contains(bare, "olang-state"), false)
    assert_eq(str.contains(bare, "data-bin"), false)
    assert_eq(str.contains(bare, "<meta name=\"x\"></head>"), true)
}

/// The full stack, one call. Config keys:
///
///   title    the page title (default "olang app")
///   routes   the route table (route(...) and rpc(...) entries)
///   client   path of the app's client source (default "client.ol"),
///            or a list of paths bundled in order; "" for an API-only
///            server
///   port     default 7500
///   head     extra HTML for <head> (default "")
///   log      (req, response, ms) => ... to own the request line
///            (default: the SDK's `METHOD /path -> status (ms)`)
///   view     the client's view function, for a server-rendered first
///            paint: the shell arrives with the mount point already
///            rendered, so the page shows before the runtime loads
///   initial  the state that first paint renders — a value, or a
///            `() => state` function evaluated per request (fresh data
///            on every load). The browser's `mount` starts from it.
///   bind     the address to listen on (default "127.0.0.1"; "0.0.0.0"
///            for other machines on the network)
///   headers  a map added to every response — the security headers
///            (Content-Security-Policy, X-Content-Type-Options,
///            Referrer-Policy) that a <meta> cannot carry
///   drain    true to stop accepting and finish in-flight requests on
///            SIGINT/SIGTERM instead of dying mid-request (default false)
///   trust_proxy  take req.remote_addr from X-Forwarded-For (default false)
///
/// `head` may be a function of the request — `(req) => html` — so a
/// per-response value (a CSP nonce) is possible; the shell is then
/// built per request.
///   sdk_dir  where the SDK's assets are read from (default: `sdk_dir()`
///            — WEB_SDK_DIR, the project's lock, the shelf)
///
/// Serves `/` (the shell), `/web.css`, `/olang-dom.js`, `/app.ol`
/// (the bundled client), `/app.olb` (the client as a program image the
/// runtime loads without parsing), the wasm runtime, and every route in
/// the table. The runtime is served two ways: `/olang.<hash>.wasm`, the
/// content-addressed URL the shell references (immutable, cached for
/// a year — a new build is a new URL), and `/olang.wasm` (revalidated
/// each load). Both negotiate `Accept-Encoding`: a pre-compressed
/// sibling on disk (`olang_playground.wasm.br` or `.gz`, next to the
/// wasm) is served with the matching `Content-Encoding` — a 6.7 MB
/// runtime is 1.9 MB gzipped, and less with brotli.
/// Blocks serving; returns only on failure to bind.
share fn serve(config) = {
    let title = get_or(config, "title", "olang app")
    let user_routes = get_or(config, "routes", [])
    let client_path = get_or(config, "client", "client.ol")
    let port = get_or(config, "port", 7500)
    // Named apart from the global `head` builtin: a call through a local
    // of that name reaches the builtin.
    let head_html = get_or(config, "head", "")
    let log = get_or(config, "log", ())
    let view_fn = get_or(config, "view", ())
    let initial = get_or(config, "initial", ())
    let bind = get_or(config, "bind", "127.0.0.1")
    let sdk = get_or(config, "sdk_dir", sdk_dir())
    let extra_headers = get_or(config, "headers", #{})
    let drain = get_or(config, "drain", false)
    let trust_proxy = get_or(config, "trust_proxy", false)
    let compress_responses = get_or(config, "compress", true)
    let head_is_fn = typeof(head_html) == "Function"

    // Read once at boot: assets, the bundle, and the wasm's location.
    let css = unwrap(fs.read_file(sdk + "/static/web.css"))
    let shim = unwrap(fs.read_file(sdk + "/static/olang-dom.js"))
    let client_paths = if typeof(client_path) == "List" => client_path
        else if client_path == "" => []
        else => [client_path]
    let bundle = if len(client_paths) == 0 => ""
        else => bundle_clients_in(sdk, map(client_paths, (p) => unwrap(fs.read_file(p))))
    // The program image: the bundle parsed here, once, and handed to the
    // browser as bytes its runtime loads without parsing (meta.encode,
    // src/olb.rs). The source stays at /app.ol — the shim's fallback
    // when the runtime in the browser is another olang version than
    // this server.
    let image = if bundle == "" => () else => match meta.encode(bundle) {
        Err(e) => unwrap(Err("client bundle does not encode: " + e)),
        Ok(bytes) => bytes
    }
    let wasm_path = if fs.exists("static/olang_playground.wasm") =>
        "static/olang_playground.wasm"
    else => sdk + "/static/olang_playground.wasm"
    if len(client_paths) > 0 && !fs.exists(wasm_path) => {
        println("WARNING: olang_playground.wasm not found — the frontend cannot boot.")
        println("Build and copy it:")
        println("  cargo build -p olang-playground --target wasm32-unknown-unknown --release")
        println("  cp target/wasm32-unknown-unknown/release/olang_playground.wasm static/")
    }

    let css_tag = etag_of(css)
    let shim_tag = etag_of(shim)
    let bundle_tag = etag_of(bundle)
    let image_tag = if image == () => "" else => etag_of(image)
    // The wasm's hash comes from its bytes, computed once at boot
    // (crypto hashes accept Bytes directly); it is both the ETag and
    // the content-addressed URL's name.
    let wasm_hash = match fs.read_bytes(wasm_path) {
        Ok(b) => str.substring(crypto.sha256(b), 0, 16),
        Err(e) => ""
    }
    let wasm_tag = if wasm_hash == "" => "" else => "\"" + wasm_hash + "\""
    let hashed_wasm_url = if wasm_hash == "" => "/olang.wasm" else => "/olang." + wasm_hash + ".wasm"

    // The shell: built once when nothing is server-rendered; per request
    // when a view is given, so a first paint from a `() => state`
    // function shows fresh data on every load.
    fn first_state() = if typeof(initial) == "Function" => initial() else => initial
    fn head_for(req) = if head_is_fn => head_html(req) else => head_html
    fn shell_for(req) =
        if view_fn == () => shell(title, head_for(req), hashed_wasm_url, image != (), (), ())
        else => {
            let state = first_state()
            shell(title, head_for(req), hashed_wasm_url, image != (), view_fn(state), state)
        }
    let per_request = view_fn != () || head_is_fn
    let static_shell = if per_request => "" else => shell_for(())
    fn image_response(req) =
        if if_none_match(req) == image_tag =>
            http.response_with_headers(304, "",
                #{ "ETag": image_tag, "Cache-Control": "no-cache" })
        else => { status: 200, body: image,
                  headers: #{ "Content-Type": "application/octet-stream",
                              "ETag": image_tag, "Cache-Control": "no-cache" } }

    // The wasm response: a pre-compressed sibling when the client
    // accepts its encoding, the raw file otherwise. `cache` is the
    // Cache-Control the URL wants.
    fn wasm_response(req, cache) = {
        let accepts = if map_has_key(req.headers, "accept-encoding") =>
            map_get(req.headers, "accept-encoding") else => ""
        let encoded = if str.contains(accepts, "br") && fs.exists(wasm_path + ".br") =>
            ["br", wasm_path + ".br"]
        else if str.contains(accepts, "gzip") && fs.exists(wasm_path + ".gz") =>
            ["gzip", wasm_path + ".gz"]
        else => ()
        let base = #{ "Content-Type": "application/wasm", "ETag": wasm_tag,
                      "Cache-Control": cache, "Vary": "Accept-Encoding" }
        if encoded == () => { status: 200, body_file: wasm_path, headers: base }
        else => { status: 200, body_file: encoded[1],
                  headers: map_set(base, "Content-Encoding", encoded[0]) }
    }

    let static_routes = [
        #{ "method": "GET", "pattern": "/", "name": "",
           "handler": (req, p) => text_response(
               if per_request => shell_for(req) else => static_shell,
               "text/html; charset=utf-8") },
        #{ "method": "GET", "pattern": "/web.css", "name": "",
           "handler": (req, p) => static_response(req, css, "text/css", css_tag) },
        #{ "method": "GET", "pattern": "/olang-dom.js", "name": "",
           "handler": (req, p) =>
               static_response(req, shim, "application/javascript", shim_tag) },
        #{ "method": "GET", "pattern": "/app.ol", "name": "",
           "handler": (req, p) =>
               static_response(req, bundle, "text/plain; charset=utf-8", bundle_tag) },
        #{ "method": "GET", "pattern": "/app.olb", "name": "",
           "handler": (req, p) =>
               if image == () => error_response(404, "not_found", "no client program")
               else => image_response(req) },
        #{ "method": "GET", "pattern": "/olang.wasm", "name": "",
           "handler": (req, p) =>
               if wasm_tag != "" && if_none_match(req) == wasm_tag =>
                   http.response_with_headers(304, "",
                       #{ "ETag": wasm_tag, "Cache-Control": "no-cache" })
               else => wasm_response(req, "no-cache") },
        #{ "method": "GET", "pattern": hashed_wasm_url, "name": "",
           "handler": (req, p) =>
               wasm_response(req, "public, max-age=31536000, immutable") }
    ]
    let table = static_routes + user_routes

    // Every response carries the configured headers — the static routes'
    // included, which is where a <meta> policy could not reach.
    // The response keeps whichever body it carries — `body` or a
    // streamed `body_file` — with the merged headers set on it; a
    // rebuild that named `body` alone turned every file response (the
    // hashed wasm included) into a 500.
    fn with_headers(response) =
        if len(map_keys(extra_headers)) == 0 => response
        else => {
            let own = if map_has_key(response, "headers") => response.headers else => #{}
            let merged = fold(map_keys(extra_headers), own,
                (acc, k) => map_set(acc, k, map_get(extra_headers, k)))
            map_set(response, "headers", merged)
        }
    // Dynamic responses gzip on the wire when the client accepts it: a
    // text body of a kilobyte or more (JSON, HTML, the client bundle)
    // that carries no encoding of its own. Files and pre-compressed
    // assets pass through; `"compress": false` turns it off.
    fn encoded(req, response) =
        if compress_responses == false => response
        else if map_has_key(response, "body") == false => response
        else if typeof(response.body) != "String" => response
        else if str.length(response.body) < 1024 => response
        else if map_has_key(req.headers, "accept-encoding") == false => response
        else if str.contains(map_get(req.headers, "accept-encoding"), "gzip") == false => response
        else if map_has_key(response.headers, "Content-Encoding") => response
        else => {
            let hs = map_set(map_set(response.headers, "Content-Encoding", "gzip"), "Vary", "Accept-Encoding")
            map_set(map_set(response, "body", compress.gzip(response.body)), "headers", hs)
        }
    if drain => {
        // A graceful stop: the signal asks the server to drain; in-flight
        // requests finish, then `serve` returns.
        let r = os.on_shutdown((why) => http.shutdown())
    }

    println(title + " listening on http://" + bind + ":" + to_string(port))
    // `Ok(())` on a drained stop, `Err` when the port could not be
    // taken; anything else the runtime might answer is still a stop.
    match http.serve(port, (req) => encoded(req, with_headers(dispatch_with(table, req, log))),
                     #{ "bind": bind, "trust_proxy": trust_proxy }) {
        Err(e) => {
            println("could not start on port " + show(port) + ": " + show(e))
            Err(e)
        },
        other => Ok(())
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

test "not_found is the 404 with the row named" {
    let r = not_found("issue OT-9")
    assert_eq(r.status, 404)
    assert_eq(str.contains(r.body, "\"not_found\""), true)
    assert_eq(str.contains(r.body, "issue OT-9 not found"), true)
}

test "under olang test a handler runs on a task thread, so a captured cell fails here" {
    let counter = cell.new(0)
    let table = [
        #{ "method": "GET", "pattern": "/cell", "name": "",
           "handler": (req, p) => { cell.set(counter, 1) "touched" } },
        #{ "method": "GET", "pattern": "/plain", "name": "",
           "handler": (req, p) => "fine" }
    ]
    let r = dispatch(table, fake_req("GET", "/cell"))
    assert_eq(r.status, 500)
    assert_eq(str.contains(r.body, "cell escaped its thread"), true)
    assert_eq(dispatch(table, fake_req("GET", "/plain")).status, 200)
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

test "static responses revalidate: 304 on a matching ETag" {
    let tag = etag_of("body text")
    let cold = static_response(fake_req("GET", "/x"), "body text", "text/css", tag)
    assert_eq(cold.status, 200)
    assert_eq(map_get(cold.headers, "ETag"), tag)
    assert_eq(map_get(cold.headers, "Cache-Control"), "no-cache")
    let warm = static_response(
        { method: "GET", path: "/x", query: #{}, body: "",
          headers: #{ "if-none-match": tag } },
        "body text", "text/css", tag)
    assert_eq(warm.status, 304)
    assert_eq(warm.body, "")
    // A stale tag gets the full body again.
    let stale = static_response(
        { method: "GET", path: "/x", query: #{}, body: "",
          headers: #{ "if-none-match": "\"old\"" } },
        "body text", "text/css", tag)
    assert_eq(stale.status, 200)
}

test "content types" {
    assert_eq(content_type_for("a/web.css"), "text/css")
    assert_eq(content_type_for("x.wasm.bak"), "application/octet-stream")
}
