# tracker — a full web app served entirely by olang

One process, one command: a persistent SQLite backend, a JSON API with
real backend discipline, and a spreadsheet-style frontend served from
the same port.

```bash
cd examples/app
olang main.ol                 # http://127.0.0.1:7000, tracker.db
olang main.ol 8080 my.db      # pick a port and database
TRACKER_TOKEN=s3cret olang main.ol   # writes now need Authorization: Bearer s3cret
```

## The API

| Route | What it does |
|---|---|
| `GET /health` | liveness: uptime, db path, issue count |
| `GET /api/issues` | list — `?status=` `assignee=` `q=` (search) `sort=` `order=` `limit=` `offset=`; returns `{items, total, limit, offset}` |
| `POST /api/issues` | create; invalid input → 422 naming each field problem |
| `GET /api/issues/:id` | one issue with comments embedded |
| `PATCH /api/issues/:id` | partial update (validated) |
| `DELETE /api/issues/:id` | remove issue + comments in one transaction |
| `GET/POST /api/issues/:id/comments` | the discussion |
| `GET /api/activity` | the audit trail — every mutation, newest first |
| `GET /api/stats` | SQL rollups by status/assignee + point quantiles via the ods data stack |
| `GET /api/export.csv` | the backlog as CSV |
| `POST /api/admin/backup` | JSON snapshot into `backups/` |

Failures wear one envelope everywhere:
`{"error": {"code", "message", "details?"}}` — including 404s, 405s
(with an `Allow` header), 401s, and validation 422s.

## Backend discipline on display

- **Schema migrations** — a `schema_version` table; each migration runs
  once, in order, in a transaction. v2 added comments, the audit trail,
  and indexes without touching v1.
- **Validation before SQL** — enums, lengths, ranges checked in
  `lib/validate.ol`; column names come from whitelists, never the wire;
  values only ever reach SQL as `?` parameters.
- **Transactions** — deleting an issue removes its comments atomically.
- **Auth middleware** — set `TRACKER_TOKEN` and every mutating request
  must carry the bearer token; reads stay open.
- **Observability** — every request logs `[id] METHOD path -> status (ms)`;
  every mutation lands in the `events` audit table.
- **HEAD rides GET** — probes and load balancers see 200, not 405.

## Layout

| File | Role |
|---|---|
| `main.ol` | handlers and wiring |
| `lib/router.ol` | routing, middleware (auth, logging), the error envelope, query-param helpers |
| `lib/store.ol` | migrations, queries, transactions, the audit trail |
| `lib/validate.ol` | request validation → clean fields or field-by-field problems |
| `static/` | the spreadsheet frontend (plain HTML + JS, no dependencies) |

## The frontend

Everything the API can do is on the surface: server-driven search,
status/assignee filters (named as clearable chips), header sorting and
paging — all mirrored into the URL hash so a refresh or a shared link
keeps the view. A comments drawer with avatars and ⌘-Enter submit, a
stats dashboard (status and per-assignee bars, point quantiles from the
ods data stack), a live activity feed with relative times, CSV export
and one-click backups, dark mode with a toggle, keyboard shortcuts
(`/` search, `n` new issue, `Esc` closes), two-step delete, an inline
create row with all fields, and validation errors surfaced field by
field as toasts. A 401 prompts once for the bearer token and remembers
it.

The API contract — validation shapes, filter semantics, 404/405/401
behavior, CSV output, the audit trail — is locked by
`tests/tracker_app_test.rs`, which boots this actual app on an ephemeral
port and drives it over the wire.
