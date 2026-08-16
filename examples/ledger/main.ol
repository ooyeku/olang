// ledger — a personal-finance web app served entirely by olang.
//
//   olang main.ol [port] [db_path]     (defaults: 7411, ledger.db)
//
// The backend is a schema-migrated SQLite store (categories, transactions,
// budgets — money as integer cents) behind a validated JSON API. The
// frontend is olang running in the browser as wasm, served by this same
// process; all analysis and charts run client-side on the ods data stack.
//
//   GET    /                       the app
//   GET    /api/health             liveness: uptime, row counts
//   GET    /api/transactions       ?from=YYYY-MM&to=YYYY-MM (default: last 6 months)
//   POST   /api/transactions       {date, amount_cents, category_id, note?} (422 lists problems)
//   PATCH  /api/transactions/:id   partial update
//   DELETE /api/transactions/:id   remove; returns the removed row
//   GET    /api/categories         with live transaction counts
//   POST   /api/categories         {name, kind: expense|income}
//   PATCH  /api/categories/:id     {name}
//   DELETE /api/categories/:id     409 while transactions reference it
//   GET    /api/budgets            ?from=YYYY-MM&to=YYYY-MM
//   PUT    /api/budgets            {category_id, month, amount_cents} upsert; 0 clears
//
// Set LEDGER_TOKEN to require `Authorization: Bearer <token>` on writes.

use lib.router { route, dispatch, json_response, error_response, error_with_details, q_str }
use lib.store {
    open_store, seed_if_empty, list_transactions, get_transaction, create_transaction,
    update_transaction, delete_transaction, list_categories, create_category,
    rename_category, delete_category, list_budgets, put_budget
}
use lib.validate { validate_transaction, validate_category, validate_rename, validate_budget, validate_id, valid_month }
use lib.format { this_month, month_add }

let args = unwrap(os.args())
// Default 7411: macOS AirPlay Receiver listens on 5000/7000 and answers
// 403, which reads as a mysterious "access denied" when the app is down.
let port = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 7411
let db_path = if len(args) > 2 => args[2] else => "ledger.db"

let conn = open_store(db_path)
seed_if_empty(conn)
let boot_ms = time.monotonic_ms()

// ── static files, read once at startup ──

let page_html = unwrap(fs.read_file("static/index.html"))
let shim_js = unwrap(fs.read_file("static/olang-dom.js"))
let ledger_ol = unwrap(fs.read_file("static/ledger.ol"))
let suite_css = unwrap(fs.read_file("static/suite.css"))

// The wasm artifact is gitignored; warn loudly at boot when missing.
// (fs.exists, not read_file: the artifact is binary.)
if !unwrap_or(fs.exists("static/olang_playground.wasm"), false) => {
    println("WARNING: static/olang_playground.wasm is missing — the frontend cannot boot.")
    println("Build and copy it:")
    println("  cargo build -p olang-playground --target wasm32-unknown-unknown --release")
    println("  cp target/wasm32-unknown-unknown/release/olang_playground.wasm examples/ledger/static/")
}

fn page(req, params) =
    http.response_with_headers(200, page_html,
        #{ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" })
fn shim(req, params) =
    http.response_with_headers(200, shim_js,
        #{ "Content-Type": "text/javascript; charset=utf-8", "Cache-Control": "no-store" })
fn frontend_source(req, params) =
    http.response_with_headers(200, ledger_ol,
        #{ "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" })
fn styles(req, params) =
    http.response_with_headers(200, suite_css,
        #{ "Content-Type": "text/css; charset=utf-8", "Cache-Control": "no-store" })
// The wasm is binary: body_file serves raw bytes straight from disk.
fn wasm_binary(req, params) = {
    status: 200,
    body_file: "static/olang_playground.wasm",
    headers: #{ "Content-Type": "application/wasm" }
}

// ── request plumbing ──

fn parse_body(req) = {
    let body = if len(req.body) == 0 => "{}" else => req.body
    json.parse(body)
}

// The month window handlers share: ?from/?to month keys, defaulting to
// the last six months ending now. Bad values fall back rather than 400 —
// a read with a mistyped month should still show something.
fn month_window(req) = {
    let to_raw = q_str(req, "to", "")
    let to = if valid_month(to_raw) => to_raw else => this_month()
    let from_raw = q_str(req, "from", "")
    let from = if valid_month(from_raw) => from_raw else => month_add(to, -5)
    { from: from, to: to }
}

// ── transactions ──

fn transactions_list(req, params) = {
    let w = month_window(req)
    json_response(200, {
        items: list_transactions(conn, w.from, w.to),
        from: w.from, to: w.to
    })
}

fn transactions_create(req, params) = match parse_body(req) {
    Err(e) => error_response(400, "bad_json", "body must be JSON"),
    Ok(doc) => match validate_transaction(doc, false) {
        Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
        Ok(fields) => json_response(201, create_transaction(conn, fields))
    }
}

fn transactions_update(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match parse_body(req) {
        Err(e) => error_response(400, "bad_json", "body must be JSON"),
        Ok(doc) => match validate_transaction(doc, true) {
            Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
            Ok(fields) => match update_transaction(conn, id, fields) {
                Err(e) => error_response(404, "not_found", "no transaction " + show(id)),
                Ok(updated) => json_response(200, updated)
            }
        }
    }
}

fn transactions_delete(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match delete_transaction(conn, id) {
        Err(e) => error_response(404, "not_found", "no transaction " + show(id)),
        Ok(gone) => json_response(200, { deleted: map_get(gone, "id") })
    }
}

// ── categories ──

fn categories_list(req, params) = json_response(200, list_categories(conn))

fn categories_create(req, params) = match parse_body(req) {
    Err(e) => error_response(400, "bad_json", "body must be JSON"),
    Ok(doc) => match validate_category(doc) {
        Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
        Ok(c) => match create_category(conn, map_get(c, "name"), map_get(c, "kind")) {
            Err(e) => error_with_details(422, "invalid", "validation failed",
                [{ field: "name", message: show(e) }]),
            Ok(row) => json_response(201, row)
        }
    }
}

fn categories_rename(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match parse_body(req) {
        Err(e) => error_response(400, "bad_json", "body must be JSON"),
        Ok(doc) => match validate_rename(doc) {
            Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
            Ok(c) => match rename_category(conn, id, map_get(c, "name")) {
                Err(msg) => if msg == "not found" =>
                    error_response(404, "not_found", "no category " + show(id))
                else =>
                    error_with_details(422, "invalid", "validation failed",
                        [{ field: "name", message: msg }]),
                Ok(row) => json_response(200, row)
            }
        }
    }
}

fn categories_delete(req, params) = match validate_id(map_get(params, "id")) {
    Err(msg) => error_response(400, "bad_id", msg),
    Ok(id) => match delete_category(conn, id) {
        Err(msg) => if msg == "in use" =>
            error_response(409, "in_use", "category " + show(id) + " has transactions; reassign them first")
        else =>
            error_response(404, "not_found", "no category " + show(id)),
        Ok(gone) => json_response(200, { deleted: gone })
    }
}

// ── budgets ──

fn budgets_list(req, params) = {
    let w = month_window(req)
    json_response(200, { items: list_budgets(conn, w.from, w.to), from: w.from, to: w.to })
}

fn budgets_put(req, params) = match parse_body(req) {
    Err(e) => error_response(400, "bad_json", "body must be JSON"),
    Ok(doc) => match validate_budget(doc) {
        Err(problems) => error_with_details(422, "invalid", "validation failed", problems),
        Ok(b) => match put_budget(conn, map_get(b, "category_id"), map_get(b, "month"),
                                  map_get(b, "amount_cents")) {
            Err(msg) => error_with_details(422, "invalid", "validation failed",
                [{ field: "category_id", message: msg }]),
            Ok(row) => json_response(200, row)
        }
    }
}

// ── health ──

fn health(req, params) = {
    let txs = map_get(unwrap(db.query_one(conn, "SELECT COUNT(*) AS n FROM transactions")), "n")
    let cats = map_get(unwrap(db.query_one(conn, "SELECT COUNT(*) AS n FROM categories")), "n")
    json_response(200, {
        status: "ok", service: "olang-ledger", db: db_path,
        transactions: txs, categories: cats,
        uptime_ms: time.monotonic_ms() - boot_ms
    })
}

// ── wiring ──

let routes = [
    route("GET", "/", page),
    route("GET", "/olang-dom.js", shim),
    route("GET", "/ledger.ol", frontend_source),
    route("GET", "/suite.css", styles),
    route("GET", "/olang.wasm", wasm_binary),
    route("GET", "/api/health", health),
    route("GET", "/api/transactions", transactions_list),
    route("POST", "/api/transactions", transactions_create),
    route("PATCH", "/api/transactions/:id", transactions_update),
    route("DELETE", "/api/transactions/:id", transactions_delete),
    route("GET", "/api/categories", categories_list),
    route("POST", "/api/categories", categories_create),
    route("PATCH", "/api/categories/:id", categories_rename),
    route("DELETE", "/api/categories/:id", categories_delete),
    route("GET", "/api/budgets", budgets_list),
    route("PUT", "/api/budgets", budgets_put)
]

fn app(req) = dispatch(routes, req)

println("ledger db=" + db_path + (match os.get_env("LEDGER_TOKEN") { Ok(t) => " auth=on", Err(e) => " auth=off" }))
http.serve(port, app)
