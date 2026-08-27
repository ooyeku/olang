//! web — the foundation layer for full-stack olang web applications.
//!
//! One route table drives the whole stack: `route`/`rpc` declare
//! endpoints, `serve` runs them behind one JSON envelope alongside the
//! wasm frontend (shell, design system, dom shim, and the client
//! program — bundled with the SDK's browser modules on the way out).
//! Views are plain data (`web.html` nodes) rendered the same way on
//! the server and in the browser; `forms` declares fields once and
//! renders, validates, and reads them from that one declaration; `sql`
//! packages migrations and the disciplines that keep SQLite honest.
//!
//! Browser-side (`mount`, `apply`, `action`, `api.call`) arrives in
//! the client program through the server's bundling — write
//! `use web { mount, ... }` in client code and the server splices the
//! real definitions in place of the import.
//!
//! The full chapter: docs/web-sdk.md.

// markup — views as data
share use lib.html {
    el, text, raw, fragment, render, page, escape, escape_attr,
    div, span, p, h1, h2, h3, ul, li, a, form, label, button,
    section, header, footer, nav, pre, code, strong, em,
    input, img, br, hr, select, option, textarea
}

// the route table — one source of truth
share use lib.routes { route, rpc, match_path, find }

// the server — the whole backend in one call
share use lib.server {
    serve, dispatch, json_response, ok_data, error_response, invalid,
    body_json, q_str, q_int, q_enum, bundle_client, sdk_dir
}

// data layer
share use lib.sql { open_db, version, rows, row, exec, insert_row, update_row, tx }

// forms — declared once, rendered/validated/read from one declaration
share use lib.forms { field, rules, read, form_fields }

// browser side — real in the served bundle; importable everywhere so
// client code resolves, documents, and type-checks. Calling these
// natively reaches the dom module's own browser-only error.
share use lib.view { mount, rerender, apply, action, action_arg, input_value }
share use lib.state { init, current, set, update }
share use lib.api { call, fetch, unwrap_envelope, err_message, err_details }

// the opinionated components
share use lib.ui {
    stack, row, spread, grid, card, muted, badge,
    btn, btn_primary, btn_danger, data_table, topbar
}
