// dash — the dashboard kit, written in olang and bundled as a builtin
// package (`use dash`).
//
// A dashboard is a grid of cards: KPI tiles for the numbers that
// matter, chart cards for the shapes behind them. Everything here
// builds HTML as STRINGS — pure functions, testable anywhere — and
// `dash.styles()` ships the styling, so a page needs no CSS of its
// own. The wiring pattern (fetch → compute → set_html per card) stays
// in your program, where it belongs; see /board.html in examples/app.
//
//   dom.set_html(mount, dash.styles() + dash.grid([
//       dash.kpi("open issues", "12", "+3 this week"),
//       dash.card("by status", "<div id=\"b-status\"></div>")
//   ], 4))
//   dom.set_html(dom.query("#b-status"), viz.chart(...))

fn esc(s) = {
    let raw = if typeof(s) == "String" => s else => show(s)
    str.replace(str.replace(str.replace(str.replace(raw,
        "&", "&amp;"), "<", "&lt;"), ">", "&gt;"), "\"", "&quot;")
}

// ── tiles and cards (pure HTML builders) ───────────────────────────────

// A KPI tile: the number big, the label above it, a note below.
// Pass "" for `note` to omit it.
share fn kpi(label, value, note) = {
    let tail = if note == "" => ""
        else => "<div class=\"dash-note\">" + esc(note) + "</div>"
    "<div class=\"dash-kpi\"><div class=\"dash-label\">" + esc(label)
        + "</div><div class=\"dash-value\">" + esc(value) + "</div>" + tail + "</div>"
}

// A card: a titled panel around arbitrary inner HTML. The inner HTML
// is NOT escaped — it is your markup (a chart mount, a table); escape
// any user data you interpolate into it.
share fn card(title, inner) =
    "<div class=\"dash-card\"><div class=\"dash-title\">" + esc(title)
        + "</div>" + inner + "</div>"

// A card that spans the full grid width.
share fn wide(title, inner) =
    "<div class=\"dash-card dash-wide\"><div class=\"dash-title\">" + esc(title)
        + "</div>" + inner + "</div>"

// The grid: cards flow into `columns` columns; KPI tiles and cards
// mix freely, and dash-wide cards break out to full width.
share fn grid(cards, columns) = {
    let mut inner = ""
    for c in cards {
        inner = inner + c
    }
    "<div class=\"dash-grid\" style=\"grid-template-columns:repeat("
        + show(columns) + ",1fr)\">" + inner + "</div>"
}

// The kit's stylesheet — dark, matching the suite. Prepend it once to
// the mount's HTML and the classes above just work.
share fn styles() = "<style>"
    + ".dash-grid{display:grid;gap:1rem;align-items:stretch}"
    + ".dash-kpi{background:#0e1116;border:1px solid #1e2430;border-radius:12px;"
    + "padding:0.9rem 1.1rem;display:flex;flex-direction:column;gap:0.15rem}"
    + ".dash-label{color:#5b6577;font-size:11px;text-transform:uppercase;letter-spacing:0.08em}"
    + ".dash-value{color:#7fd1b9;font-size:26px;font-weight:600;font-family:ui-monospace,monospace}"
    + ".dash-note{color:#9aa4b2;font-size:12px}"
    + ".dash-card{background:#0e1116;border:1px solid #1e2430;border-radius:12px;"
    + "padding:0.6rem;overflow:hidden}"
    + ".dash-title{color:#9aa4b2;font-size:12px;padding:0.1rem 0.3rem 0.4rem}"
    + ".dash-wide{grid-column:1 / -1}"
    + "</style>"
