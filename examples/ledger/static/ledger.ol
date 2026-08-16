// ledger.ol — the ledger frontend, in olang, running as WebAssembly.
//
//   * Stateless logic: the server owns the data; the viewed month lives in
//     the URL (?month=YYYY-MM), and the fetched window sits in session
//     state (dom.state_*), re-derived on every render.
//   * Event delegation: listeners bind once at boot to elements that exist
//     at boot; rows and budget editors re-render as HTML strings, with
//     actions encoded in element ids (del-17, amt-17, bud-3).
//   * Analysis runs HERE, in the browser: the raw window becomes an ods
//     Frame, group_by produces the chart data, and viz renders SVG — the
//     data stack client-side.
//
// Money is integer cents on the wire and in state; dollars appear only in
// input fields, money() strings, and chart values. money/to_cents mirror
// lib/format.ol, whose test blocks are the reference behavior.

use viz

// ── escaping ───────────────────────────────────────────────────────────

fn esc(s) = show(s)
    |> str.replace("&", "&amp;")
    |> str.replace("<", "&lt;")
    |> str.replace(">", "&gt;")
    |> str.replace("\"", "&quot;")

fn json_esc(s) = show(s)
    |> str.replace("\\", "\\\\")
    |> str.replace("\"", "\\\"")
    |> str.replace("\n", "\\n")
    |> str.replace("\t", "\\t")

// ── money (mirrors lib/format.ol) ──────────────────────────────────────

fn thousands(n) = {
    let s = to_string(n)
    let ln = str.length(s)
    if ln <= 3 => s
    else => {
        let mut out = ""
        let mut i = 0
        while i < ln {
            if i > 0 && (ln - i) % 3 == 0 => { out = out + "," }
            out = out + str.substring(s, i, i + 1)
            i = i + 1
        }
        out
    }
}

fn money(cents) = {
    let sign = if cents < 0 => "-" else => ""
    let a = if cents < 0 => 0 - cents else => cents
    let frac = a % 100
    let frac_s = if frac < 10 => "0" + to_string(frac) else => to_string(frac)
    sign + "$" + thousands(a / 100) + "." + frac_s
}

// Signed dollars for an editable field: "-12.50".
fn dollars(cents) = {
    let sign = if cents < 0 => "-" else => ""
    let a = if cents < 0 => 0 - cents else => cents
    let frac = a % 100
    let frac_s = if frac < 10 => "0" + to_string(frac) else => to_string(frac)
    sign + to_string(a / 100) + "." + frac_s
}

fn to_cents(s) = {
    let t = str.trim(s)
    let neg = str.starts_with(t, "-")
    let body = if neg => str.substring(t, 1, str.length(t)) else => t
    let parts = str.split(body, ".")
    let whole_s = if parts[0] == "" => "0" else => parts[0]
    if len(parts) > 2 || body == "" || body == "." => Err("not an amount")
    else => match str.parse_int(whole_s) {
        Err(e) => Err("not an amount"),
        Ok(whole) => {
            let frac = if len(parts) == 1 => Ok(0)
                else => {
                    let f = parts[1]
                    if str.length(f) == 0 || str.length(f) > 2 => Err("use at most two decimals")
                    else => match str.parse_int(f) {
                        Err(e) => Err("not an amount"),
                        Ok(n) => Ok(if str.length(f) == 1 => n * 10 else => n)
                    }
                }
            match frac {
                Err(m) => Err(m),
                Ok(fr) => {
                    let cents = whole * 100 + fr
                    Ok(if neg => 0 - cents else => cents)
                }
            }
        }
    }
}

// ── months and URL state ───────────────────────────────────────────────

fn this_month() = unwrap(dates.format_date(dates.today(), "%Y-%m"))
fn month_of(date) = unwrap(dates.format_date(date, "%Y-%m"))
fn month_add(month, delta) =
    unwrap(dates.format_date(unwrap(dates.add_months(month + "-01", delta)), "%Y-%m"))

fn cur_month() = {
    let q = map_get(dom.location(), "query")
    if map_has_key(q, "month") => map_get(q, "month") else => this_month()
}

fn goto_month(m) = {
    dom.push_state(if m == this_month() => "/" else => "/?month=" + m)
    reload()
}

// Session-state accessors: the fetched window and the category list.
fn txs() = { let v = dom.state_get("txs"); if typeof(v) == "Unit" => [] else => v }
fn budgets() = { let v = dom.state_get("budgets"); if typeof(v) == "Unit" => [] else => v }
fn cats() = { let v = dom.state_get("cats"); if typeof(v) == "Unit" => [] else => v }

fn flash(msg) = dom.set_text(dom.query("#flash"), msg)

// ── rendering: transactions ────────────────────────────────────────────

fn row_html(t) = {
    let id = show(map_get(t, "id"))
    let cents = map_get(t, "amount_cents")
    let tone = if cents < 0 => "neg" else => "pos"
    "<tr>"
        + "<td class=\"date\">" + esc(map_get(t, "date")) + "</td>"
        + "<td class=\"cat\">" + esc(map_get(t, "category")) + "</td>"
        + "<td><input class=\"cell\" id=\"note-" + id + "\" value=\"" + esc(map_get(t, "note")) + "\"></td>"
        + "<td class=\"amt " + tone + "\"><input class=\"cell\" style=\"width:7rem;text-align:right\" id=\"amt-"
        + id + "\" value=\"" + dollars(cents) + "\"></td>"
        + "<td class=\"act\"><button class=\"del\" id=\"del-" + id + "\" title=\"delete\">×</button></td>"
        + "</tr>"
}

fn month_rows() = txs() |> filter((t) => month_of(map_get(t, "date")) == cur_month())

fn render_rows() = {
    let rows = month_rows()
    dom.set_html(dom.query("#rows"),
        if len(rows) == 0 => "<tr><td colspan=\"5\" class=\"empty\">no transactions this month</td></tr>"
        else => rows |> map(row_html) |> join(""))
    let income = rows |> filter((t) => map_get(t, "amount_cents") > 0)
        |> map((t) => map_get(t, "amount_cents")) |> sum()
    let spent = rows |> filter((t) => map_get(t, "amount_cents") < 0)
        |> map((t) => 0 - map_get(t, "amount_cents")) |> sum()
    dom.set_html(dom.query("#summary"),
        "<span>in <b class=\"pos\">" + money(income) + "</b></span>"
        + "<span>out <b>" + money(spent) + "</b></span>"
        + "<span>net <b class=\"" + (if income >= spent => "pos" else => "neg") + "\">"
        + money(income - spent) + "</b></span>")
}

// ── rendering: budgets ─────────────────────────────────────────────────

// Spent this month per category id (expenses as positive cents).
fn spent_by_category() = {
    let mut acc = #{}
    for t in month_rows() {
        let cents = map_get(t, "amount_cents")
        if cents < 0 => {
            let key = show(map_get(t, "category_id"))
            let sofar = if map_has_key(acc, key) => map_get(acc, key) else => 0
            acc = map_set(acc, key, sofar + (0 - cents))
        }
    }
    acc
}

fn budget_for(cid) = {
    let month = cur_month()
    let hit = budgets() |> filter((b) =>
        map_get(b, "category_id") == cid && map_get(b, "month") == month)
    if len(hit) == 0 => 0 else => map_get(hit[0], "amount_cents")
}

fn budget_row_html(c, spent_map) = {
    let cid = map_get(c, "id")
    let planned = budget_for(cid)
    let key = show(cid)
    let spent = if map_has_key(spent_map, key) => map_get(spent_map, key) else => 0
    let status = if planned == 0 => "<span class=\"spent\">" + money(spent) + " spent</span>"
        else => {
            let tone = if spent > planned => "over" else => "under"
            "<span class=\"spent " + tone + "\">" + money(spent) + " of " + money(planned) + "</span>"
        }
    "<div class=\"budget-row\">"
        + "<span class=\"name\">" + esc(map_get(c, "name")) + "</span>"
        + status
        + "<input id=\"bud-" + key + "\" placeholder=\"0.00\" inputmode=\"decimal\" value=\""
        + (if planned == 0 => "" else => dollars(planned)) + "\">"
        + "</div>"
}

fn render_budgets() = {
    let expense_cats = cats() |> filter((c) => map_get(c, "kind") == "expense")
    let spent_map = spent_by_category()
    dom.set_html(dom.query("#budgets"),
        if len(expense_cats) == 0 => "<div class=\"empty\">no expense categories</div>"
        else => expense_cats |> map((c) => budget_row_html(c, spent_map)) |> join(""))
}

// ── rendering: charts (ods + viz, in the browser) ──────────────────────

fn themed(s, w, h) =
    map_set(map_set(map_set(map_set(map_set(s,
        "theme", "dark"), "responsive", true), "interactive", true),
        "width", w), "height", h)

fn cents_to_f(cents) = to_float(cents) / 100.0

// This month's expenses by category, summed on an ods Frame.
fn render_cats_chart() = {
    let mut recs = month_rows()
        |> filter((t) => map_get(t, "amount_cents") < 0)
        |> map((t) => #{ "category": map_get(t, "category"),
                         "dollars": cents_to_f(0 - map_get(t, "amount_cents")) })
    if len(recs) == 0 => dom.set_html(dom.query("#chart-cats"),
        "<div class=\"empty\">no spending this month</div>")
    else => {
        let by_cat = ods.frame_from_records(recs)
            |> ods.group_by("category", [["dollars", "sum", "dollars"]])
            |> ods.sort_by("dollars", true)
        dom.set_html(dom.query("#chart-cats"), viz.chart(themed(#{
            "data": ods.to_records(by_cat),
            "mark": "bar", "x": "category", "y": "dollars", "vary": true
        }, 1140, 300)))
    }
}

// Budget vs actual for the viewed month, grouped bars per category.
fn render_budget_chart() = {
    let month = cur_month()
    let spent_map = spent_by_category()
    let mut recs = []
    for c in cats() {
        if map_get(c, "kind") == "expense" => {
            let key = show(map_get(c, "id"))
            let planned = budget_for(map_get(c, "id"))
            let spent = if map_has_key(spent_map, key) => map_get(spent_map, key) else => 0
            if planned > 0 || spent > 0 => {
                recs = recs + [
                    #{ "category": map_get(c, "name"), "kind": "budget", "dollars": cents_to_f(planned) },
                    #{ "category": map_get(c, "name"), "kind": "actual", "dollars": cents_to_f(spent) }
                ]
            }
        }
    }
    if len(recs) == 0 => dom.set_html(dom.query("#chart-budget"),
        "<div class=\"empty\">no budgets set — type amounts below</div>")
    else => dom.set_html(dom.query("#chart-budget"), viz.chart(themed(#{
        "data": recs, "mark": "bar", "x": "category", "y": "dollars",
        "color": "kind", "colors": ["#5aa9e6", "#3ddc97"]
    }, 560, 300)))
}

// Six-month in/out trend over the whole fetched window.
fn render_trend_chart() = {
    let mut recs = txs() |> map((t) => {
        let cents = map_get(t, "amount_cents")
        #{ "month": month_of(map_get(t, "date")),
           "kind": if cents < 0 => "out" else => "in",
           "dollars": cents_to_f(if cents < 0 => 0 - cents else => cents) }
    })
    if len(recs) == 0 => dom.set_html(dom.query("#chart-trend"),
        "<div class=\"empty\">no data yet</div>")
    else => {
        let by_month = ods.frame_from_records(recs)
            |> ods.group_by(["month", "kind"], [["dollars", "sum", "dollars"]])
            |> ods.sort_by("month", false)
        dom.set_html(dom.query("#chart-trend"), viz.chart(themed(#{
            "data": ods.to_records(by_month),
            "mark": "bar", "x": "month", "y": "dollars",
            "color": "kind", "colors": ["#3ddc97", "#ef6b73"]
        }, 560, 300)))
    }
}

// ── render everything from state ───────────────────────────────────────

fn render_all() = {
    let month = cur_month()
    dom.set_text(dom.query("#month-label"), month)
    dom.set_text(dom.query("#tx-month"), month)
    dom.set_text(dom.query("#budget-month"), month)
    render_category_select()
    render_rows()
    render_budgets()
    render_cats_chart()
    render_budget_chart()
    render_trend_chart()
}

fn render_category_select() = {
    let options = cats() |> map((c) =>
        "<option value=\"" + show(map_get(c, "id")) + "\">"
        + esc(map_get(c, "name")) + "</option>") |> join("")
    let el = dom.query("#new-category")
    let before = dom.value(el)
    dom.set_html(el, options)
    if before != "" => dom.set_value(el, before)
    update_kind_tag()
}

fn selected_kind() = {
    let cid = unwrap_or(str.parse_int(dom.value(dom.query("#new-category"))), 0)
    let hit = cats() |> filter((c) => map_get(c, "id") == cid)
    if len(hit) == 0 => "expense" else => map_get(hit[0], "kind")
}

fn update_kind_tag() = dom.set_text(dom.query("#kind-tag"), selected_kind())

// ── data flow ──────────────────────────────────────────────────────────

fn reload() = {
    let to = cur_month()
    let from = month_add(to, -5)
    dom.fetch_json("GET", "/api/transactions?from=" + from + "&to=" + to, "", (resp) => {
        if map_has_key(resp, "items") => {
            dom.state_set("txs", map_get(resp, "items"))
            render_all()
        }
    })
    dom.fetch_json("GET", "/api/budgets?from=" + from + "&to=" + to, "", (resp) => {
        if map_has_key(resp, "items") => {
            dom.state_set("budgets", map_get(resp, "items"))
            render_all()
        }
    })
}

fn load_categories() =
    dom.fetch_json("GET", "/api/categories", "", (resp) => {
        // The endpoint returns a bare JSON array; anything map-shaped here
        // is the error envelope.
        if typeof(resp) == "List" => {
            dom.state_set("cats", resp)
            render_all()
        }
    })

// ── mutations ──────────────────────────────────────────────────────────

fn add_transaction() = {
    let raw = str.trim(dom.value(dom.query("#new-amount")))
    if raw != "" => match to_cents(raw) {
        Err(msg) => flash("amount: " + msg),
        Ok(entered) => {
            if entered == 0 => flash("amount: must not be zero")
            else => {
                // The category's kind sets the sign; the field takes a
                // positive number either way.
                let a = if entered < 0 => 0 - entered else => entered
                let cents = if selected_kind() == "expense" => 0 - a else => a
                let cid = unwrap_or(str.parse_int(dom.value(dom.query("#new-category"))), 0)
                let body = "{\"date\": \"" + dom.value(dom.query("#new-date")) + "\""
                    + ", \"amount_cents\": " + show(cents)
                    + ", \"category_id\": " + show(cid)
                    + ", \"note\": \"" + json_esc(str.trim(dom.value(dom.query("#new-note")))) + "\"}"
                dom.fetch_json("POST", "/api/transactions", body, (resp) => {
                    if map_has_key(resp, "error") =>
                        flash("add failed: " + unwrap(json.stringify(map_get(resp, "error"))))
                    else => {
                        dom.set_value(dom.query("#new-amount"), "")
                        dom.set_value(dom.query("#new-note"), "")
                        dom.focus(dom.query("#new-amount"))
                        flash("")
                        reload()
                    }
                })
            }
        }
    }
}

fn patch_tx(id, body) =
    dom.fetch_json("PATCH", "/api/transactions/" + id, body, (resp) => {
        if map_has_key(resp, "error") =>
            flash("edit failed: " + unwrap(json.stringify(map_get(resp, "error"))))
        else => { flash("") reload() }
    })

fn on_rows_click(tid) = {
    if starts_with(tid, "del-") => {
        let id = str.substring(tid, 4, len(tid))
        dom.fetch_json("DELETE", "/api/transactions/" + id, "", (resp) => { reload() })
    }
}

fn on_rows_change(e) = {
    let tid = map_get(e, "id")
    let value = map_get(e, "value")
    if starts_with(tid, "amt-") => {
        let id = str.substring(tid, 4, len(tid))
        match to_cents(value) {
            Err(msg) => { flash("amount: " + msg) reload() },
            Ok(cents) => if cents == 0 => { flash("amount: must not be zero") reload() }
                else => patch_tx(id, "{\"amount_cents\": " + show(cents) + "}")
        }
    }
    else => { if starts_with(tid, "note-") => {
        let id = str.substring(tid, 5, len(tid))
        patch_tx(id, "{\"note\": \"" + json_esc(value) + "\"}")
    }}
}

fn on_budget_change(e) = {
    let tid = map_get(e, "id")
    if starts_with(tid, "bud-") => {
        let cid = unwrap_or(str.parse_int(str.substring(tid, 4, len(tid))), 0)
        let raw = str.trim(map_get(e, "value"))
        let cents = if raw == "" => Ok(0) else => to_cents(raw)
        match cents {
            Err(msg) => { flash("budget: " + msg) reload() },
            Ok(c) => if c < 0 => { flash("budget: must not be negative") reload() }
                else => {
                    let body = "{\"category_id\": " + show(cid)
                        + ", \"month\": \"" + cur_month() + "\""
                        + ", \"amount_cents\": " + show(c) + "}"
                    dom.fetch_json("PUT", "/api/budgets", body, (resp) => {
                        if map_has_key(resp, "error") =>
                            flash("budget failed: " + unwrap(json.stringify(map_get(resp, "error"))))
                        else => { flash("") reload() }
                    })
                }
        }
    }
}

// ── boot: bind once, render forever ────────────────────────────────────

dom.on(dom.query("#month-prev"), "click", (e) => { goto_month(month_add(cur_month(), -1)) })
dom.on(dom.query("#month-next"), "click", (e) => { goto_month(month_add(cur_month(), 1)) })
dom.on(dom.query("#add-btn"), "click", (e) => { add_transaction() })
dom.on(dom.query("#new-amount"), "enter", (e) => { add_transaction() })
dom.on(dom.query("#new-note"), "enter", (e) => { add_transaction() })
dom.on(dom.query("#new-category"), "change", (e) => { update_kind_tag() })
dom.on(dom.query("#rows"), "click", (e) => { on_rows_click(map_get(e, "id")) })
dom.on(dom.query("#rows"), "change", (e) => { on_rows_change(e) })
dom.on(dom.query("#budgets"), "change", (e) => { on_budget_change(e) })
dom.on_route((r) => { reload() })

// One delegated tooltip covers every chart rendered into the grid.
viz.tooltip(dom.query("#charts"))

dom.set_value(dom.query("#new-date"), dates.today())
flash("frontend: olang (wasm) · charts: ods + viz, in the browser")
load_categories()
reload()
