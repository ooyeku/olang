# ledger — a personal-finance web app, entirely in olang

One olang process serves a SQLite-backed JSON API, the page, and the
frontend's own olang source; the browser loads the language as
WebAssembly and runs that source against the DOM. All analysis and
charts run **in the browser** on the ods data stack: the API serves raw
rows, and the frontend builds Frames, runs `group_by`, and renders
`viz` SVG client-side. Money is integer cents end to end.

## Run

```bash
# one-time: build the wasm runtime the frontend boots on
cargo build -p olang-playground --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/olang_playground.wasm examples/ledger/static/
# (or `make wasm` at the repo root, which stages it for every example)

cd examples/ledger
olang seed.ol          # optional: three months of demo data
olang main.ol          # http://127.0.0.1:7411, db ledger.db
olang main.ol 8080 my.db   # choose port and database
```

Set `LEDGER_TOKEN` to require `Authorization: Bearer <token>` on all
mutating requests; reads stay open.

## API

| Method | Path | Notes |
|---|---|---|
| GET | `/api/health` | uptime and row counts |
| GET | `/api/transactions?from=YYYY-MM&to=YYYY-MM` | rows joined with category, date desc; defaults to the last six months |
| POST | `/api/transactions` | `{date, amount_cents, category_id, note?}`; 422 lists field problems |
| PATCH | `/api/transactions/:id` | partial update |
| DELETE | `/api/transactions/:id` | returns the removed id |
| GET | `/api/categories` | with live transaction counts |
| POST | `/api/categories` | `{name, kind: expense\|income}`; duplicate names 422 |
| PATCH | `/api/categories/:id` | `{name}` |
| DELETE | `/api/categories/:id` | 409 while transactions reference it |
| GET | `/api/budgets?from=&to=` | month-keyed rows |
| PUT | `/api/budgets` | `{category_id, month, amount_cents}` upsert; 0 clears |

Errors always arrive in the body as `{"error": {code, message,
details?}}`, because the browser's `dom.fetch_json` does not expose HTTP
status codes.

## Layout

```
main.ol           routes + handlers; static files read once at boot
lib/router.ol     :param routing, 405/404, auth, the JSON error envelope
lib/store.ol      SQLite migrations + CRUD (integer cents; FK pragma on)
lib/validate.ol   Ok(fields) | Err([{field, message}]) validators
lib/format.ol     money()/to_cents()/month helpers, with test blocks
static/ledger.ol  the frontend: dom events, ods Frames, viz charts
static/index.html the page; static/olang-dom.js the generic wasm shim
seed.ol           seeded three-month demo data
```

`olang test .` runs the module self-checks;
`cargo test --test ledger_app_test` boots the real server against
`:memory:` and locks the API contract.

## Known limits (v1)

- `dates.today()` in the browser build is UTC, so the composer's default
  date can be one day ahead of local time in the evening; adjust in the
  picker. (A stdlib timezone story is a recorded gap.)
- Transaction date and category edits are delete-and-re-add; note and
  amount edit inline.
- Single user, localhost only (`http.serve` binds 127.0.0.1 by design).
