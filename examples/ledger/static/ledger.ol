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
    let frac_s = if frac < 10 => `0${frac}` else => to_string(frac)
    sign + "$" + thousands(a / 100) + "." + frac_s
}

// Signed dollars for an editable field: "-12.50".
fn dollars(cents) = {
    let sign = if cents < 0 => "-" else => ""
    let a = if cents < 0 => 0 - cents else => cents
    let frac = a % 100
    let frac_s = if frac < 10 => `0${frac}` else => to_string(frac)
    `${sign}${a / 100}.${frac_s}`
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

// ── toasts ─────────────────────────────────────────────────────────────
// A fresh element each time restarts the CSS fade; tone picks the accent:
// "ok" (mint), "err" (red), "hint" (quiet, longer-lived).

fn toast(msg, tone) =
    dom.set_html(dom.query("#flash"),
        if msg == "" => ""
        else => "<div class=\"toast " + tone + "\">" + esc(msg) + "</div>")

// The server's error envelope, made readable: prefer the field-level
// details ("date: must be YYYY-MM-DD"), fall back to the message. Parsed
// JSON objects arrive as JsonObject structs, not Maps; a bare string here
// is the shim's network-failure report.
fn err_text(envelope) = {
    if typeof(envelope) != "Map" && typeof(envelope) != "JsonObject" => "server unreachable"
    else => {
        if map_has_key(envelope, "details") => {
            let ds = map_get(envelope, "details")
            if typeof(ds) == "List" && len(ds) > 0 =>
                ds |> map((d) => show(map_get(d, "field")) + ": " + show(map_get(d, "message")))
                   |> join(" · ")
            else => show(map_get(envelope, "message"))
        }
        else => { if map_has_key(envelope, "message") => show(map_get(envelope, "message"))
                  else => "server unreachable" }
    }
}

fn toast_error(prefix, resp) =
    toast(prefix + ": " + err_text(if map_has_key(resp, "error") => map_get(resp, "error") else => resp), "err")

// ── rendering: transactions ────────────────────────────────────────────

// The id of a row whose delete button is one click from firing; delete is
// two-click ("×" then "sure?") so a stray tap never destroys data.
fn armed() = { let v = dom.state_get("armed"); if typeof(v) == "Unit" => "" else => v }

fn row_html(t) = {
    let id = show(map_get(t, "id"))
    let cents = map_get(t, "amount_cents")
    let tone = if cents < 0 => "neg" else => "pos"
    let del = if armed() == id
        => "<button class=\"sure\" id=\"del-" + id + "\" title=\"click again to delete\">sure?</button>"
        else => "<button class=\"del\" id=\"del-" + id + "\" title=\"delete\">×</button>"
    "<tr>"
        + "<td class=\"date\">" + esc(map_get(t, "date")) + "</td>"
        + "<td class=\"cat\">" + esc(map_get(t, "category")) + "</td>"
        + "<td><input class=\"cell\" id=\"note-" + id + "\" value=\"" + esc(map_get(t, "note")) + "\"></td>"
        + "<td class=\"amt " + tone + "\"><input class=\"cell\" style=\"width:7rem;text-align:right\" id=\"amt-"
        + id + "\" value=\"" + dollars(cents) + "\"></td>"
        + "<td class=\"act\">" + del + "</td>"
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
    let net = income - spent
    dom.set_html(dom.query("#summary"),
        "<div class=\"stat\"><small>in</small><b class=\"pos\">" + money(income) + "</b></div>"
        + "<div class=\"stat\"><small>out</small><b>" + money(spent) + "</b></div>"
        + "<div class=\"stat\"><small>net</small><b class=\"" + (if net >= 0 => "pos" else => "neg") + "\">"
        + money(net) + "</b></div>")
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
    let over = planned > 0 && spent > planned
    let status = if planned == 0 => "<span class=\"spent\">" + money(spent) + " spent</span>"
        else => {
            let tone = if over => "over" else => "under"
            "<span class=\"spent " + tone + "\">" + money(spent) + " of " + money(planned) + "</span>"
        }
    // The fill fraction, clamped to [0, 100]; no budget → a sliver of
    // neutral bar so the row still reads as a gauge.
    let pct = if planned == 0 => (if spent > 0 => 100 else => 0)
        else => { let p = spent * 100 / planned; if p > 100 => 100 else => p }
    let bar_tone = if planned == 0 => "none" else => { if over => "over" else => "" }
    "<div class=\"budget-row\">"
        + "<span class=\"name\">" + esc(map_get(c, "name")) + "</span>"
        + status
        + "<input id=\"bud-" + key + "\" placeholder=\"0.00\" inputmode=\"decimal\" value=\""
        + (if planned == 0 => "" else => dollars(planned)) + "\">"
        + "<div class=\"bar\"><i class=\"" + bar_tone + "\" style=\"width:" + show(pct) + "%\"></i></div>"
        + "</div>"
}

fn render_budgets() = {
    let expense_cats = cats() |> filter((c) => map_get(c, "kind") == "expense")
    let spent_map = spent_by_category()
    dom.set_html(dom.query("#budgets"),
        if len(expense_cats) == 0 => "<div class=\"empty\">no expense categories</div>"
        else => expense_cats |> map((c) => budget_row_html(c, spent_map)) |> join(""))
}

// ── rendering: category manager ────────────────────────────────────────
// Rename inline (change on the name input), delete with the same
// two-click arm as transactions; the server's 409 keeps a category with
// transactions alive, and its message lands in the toast.

fn armed_cat() = { let v = dom.state_get("armed_cat"); if typeof(v) == "Unit" => "" else => v }

fn cat_row_html(c) = {
    let id = show(map_get(c, "id"))
    let kind = map_get(c, "kind")
    let n = if map_has_key(c, "tx_count") => map_get(c, "tx_count") else => 0
    let del = if armed_cat() == id
        => "<button class=\"sure\" id=\"cdel-" + id + "\" title=\"click again to delete\">sure?</button>"
        else => "<button class=\"del\" id=\"cdel-" + id + "\" title=\"delete\">×</button>"
    "<div class=\"cat-row\">"
        + "<input class=\"cell\" id=\"cname-" + id + "\" value=\"" + esc(map_get(c, "name")) + "\">"
        + "<span class=\"kind " + (if kind == "income" => "income" else => "") + "\">" + esc(kind) + "</span>"
        + "<span class=\"count\" title=\"transactions\">" + show(n) + "</span>"
        + del
        + "</div>"
}

fn render_cats_manager() =
    dom.set_html(dom.query("#cats"),
        if len(cats()) == 0 => "<div class=\"empty\">no categories</div>"
        else => cats() |> map(cat_row_html) |> join(""))

// ── rendering: charts (ods + viz, in the browser) ──────────────────────

fn themed(s, w, h) =
    map_set(map_set(map_set(map_set(map_set(s,
        "theme", "dark"), "responsive", true), "interactive", true),
        "width", w), "height", h)

fn cents_to_f(cents) = to_float(cents) / 100.0

// One palette across all three charts: mint = money in / on plan,
// blue = plan, red = money out.
let PAL_IN = "#3ddc97"
let PAL_PLAN = "#5aa9e6"
let PAL_OUT = "#ef6b73"

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
        }, 1140, 260)))
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
        "<div class=\"empty\">no budgets set — type amounts in the budgets card</div>")
    else => dom.set_html(dom.query("#chart-budget"), viz.chart(themed(#{
        "data": recs, "mark": "bar", "x": "category", "y": "dollars",
        "color": "kind", "colors": [PAL_PLAN, PAL_IN]
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
        // viz assigns series colors in first-seen order, and the grouped
        // rows arrive in data order — put "in" rows first so mint is
        // always money in and red always money out.
        let rows = ods.to_records(by_month)
        let ordered = (rows |> filter((r) => map_get(r, "kind") == "in"))
            + (rows |> filter((r) => map_get(r, "kind") == "out"))
        dom.set_html(dom.query("#chart-trend"), viz.chart(themed(#{
            "data": ordered,
            "mark": "bar", "x": "month", "y": "dollars",
            "color": "kind", "colors": [PAL_IN, PAL_OUT]
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
    render_cats_manager()
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

fn update_kind_tag() = {
    let kind = selected_kind()
    let el = dom.query("#kind-tag")
    dom.set_text(el, kind)
    dom.set_class(el, if kind == "income" => "kind-tag income" else => "kind-tag")
}

// ── data flow ──────────────────────────────────────────────────────────
// The window needs two fetches; rendering waits for both, so a reload
// paints once instead of twice. A failed fetch (network down, server
// gone) surfaces in the toast instead of dying silently.

fn arrived() = {
    let left = dom.state_get("pending") - 1
    dom.state_set("pending", left)
    if left <= 0 => render_all()
}

fn reload() = {
    dom.state_set("armed", "")
    dom.state_set("armed_cat", "")
    dom.state_set("pending", 2)
    let to = cur_month()
    let from = month_add(to, -5)
    dom.fetch_json("GET", "/api/transactions?from=" + from + "&to=" + to, "", (resp) => {
        if map_has_key(resp, "items") => dom.state_set("txs", map_get(resp, "items"))
        else => toast_error("load", resp)
        arrived()
    })
    dom.fetch_json("GET", "/api/budgets?from=" + from + "&to=" + to, "", (resp) => {
        if map_has_key(resp, "items") => dom.state_set("budgets", map_get(resp, "items"))
        else => toast_error("load", resp)
        arrived()
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
        else => toast_error("load", resp)
    })

// ── mutations: transactions ────────────────────────────────────────────

fn add_transaction() = {
    let date = dom.value(dom.query("#new-date"))
    let raw = str.trim(dom.value(dom.query("#new-amount")))
    if date == "" => toast("pick a date first", "err")
    else => { if raw == "" => toast("amount: enter one, like 12.50", "err")
    else => match to_cents(raw) {
        Err(msg) => toast("amount: " + msg, "err"),
        Ok(entered) => {
            if entered == 0 => toast("amount: must not be zero", "err")
            else => {
                // The category's kind sets the sign; the field takes a
                // positive number either way.
                let a = if entered < 0 => 0 - entered else => entered
                let cents = if selected_kind() == "expense" => 0 - a else => a
                let cid = unwrap_or(str.parse_int(dom.value(dom.query("#new-category"))), 0)
                let body = "{\"date\": \"" + date + "\""
                    + ", \"amount_cents\": " + show(cents)
                    + ", \"category_id\": " + show(cid)
                    + ", \"note\": \"" + json_esc(str.trim(dom.value(dom.query("#new-note")))) + "\"}"
                dom.fetch_json("POST", "/api/transactions", body, (resp) => {
                    if map_has_key(resp, "error") => toast_error("add failed", resp)
                    else => {
                        dom.set_value(dom.query("#new-amount"), "")
                        dom.set_value(dom.query("#new-note"), "")
                        dom.focus(dom.query("#new-amount"))
                        toast("added " + money(cents), "ok")
                        reload()
                    }
                })
            }
        }
    } }
}

fn patch_tx(id, body) =
    dom.fetch_json("PATCH", "/api/transactions/" + id, body, (resp) => {
        if map_has_key(resp, "error") => toast_error("edit failed", resp)
        else => { toast("saved", "ok") reload() }
    })

fn on_rows_click(tid) = {
    if starts_with(tid, "del-") => {
        let id = str.substring(tid, 4, len(tid))
        if armed() == id => {
            dom.fetch_json("DELETE", "/api/transactions/" + id, "", (resp) => {
                if map_has_key(resp, "error") => toast_error("delete failed", resp)
                else => toast("deleted", "ok")
                reload()
            })
        }
        else => { dom.state_set("armed", id) render_rows() }
    }
}

fn on_rows_change(e) = {
    let tid = map_get(e, "id")
    let value = map_get(e, "value")
    if starts_with(tid, "amt-") => {
        let id = str.substring(tid, 4, len(tid))
        match to_cents(value) {
            Err(msg) => { toast("amount: " + msg, "err") reload() },
            Ok(cents) => if cents == 0 => { toast("amount: must not be zero", "err") reload() }
                else => patch_tx(id, "{\"amount_cents\": " + show(cents) + "}")
        }
    }
    else => { if starts_with(tid, "note-") => {
        let id = str.substring(tid, 5, len(tid))
        patch_tx(id, "{\"note\": \"" + json_esc(value) + "\"}")
    } }
}

// ── mutations: budgets ─────────────────────────────────────────────────

fn on_budget_change(e) = {
    let tid = map_get(e, "id")
    if starts_with(tid, "bud-") => {
        let cid = unwrap_or(str.parse_int(str.substring(tid, 4, len(tid))), 0)
        let raw = str.trim(map_get(e, "value"))
        let cents = if raw == "" => Ok(0) else => to_cents(raw)
        match cents {
            Err(msg) => { toast("budget: " + msg, "err") reload() },
            Ok(c) => if c < 0 => { toast("budget: must not be negative", "err") reload() }
                else => {
                    let body = "{\"category_id\": " + show(cid)
                        + ", \"month\": \"" + cur_month() + "\""
                        + ", \"amount_cents\": " + show(c) + "}"
                    dom.fetch_json("PUT", "/api/budgets", body, (resp) => {
                        if map_has_key(resp, "error") => toast_error("budget failed", resp)
                        else => { toast(if c == 0 => "budget cleared" else => "budget set", "ok") reload() }
                    })
                }
        }
    }
}

// ── mutations: categories ──────────────────────────────────────────────

fn add_category() = {
    let name = str.trim(dom.value(dom.query("#cat-name")))
    if name == "" => toast("category: name it first", "err")
    else => {
        let body = "{\"name\": \"" + json_esc(name)
            + "\", \"kind\": \"" + dom.value(dom.query("#cat-kind")) + "\"}"
        dom.fetch_json("POST", "/api/categories", body, (resp) => {
            if map_has_key(resp, "error") => toast_error("category", resp)
            else => {
                dom.set_value(dom.query("#cat-name"), "")
                toast("category added", "ok")
                load_categories()
            }
        })
    }
}

fn on_cats_click(tid) = {
    if starts_with(tid, "cdel-") => {
        let id = str.substring(tid, 5, len(tid))
        if armed_cat() == id => {
            dom.fetch_json("DELETE", "/api/categories/" + id, "", (resp) => {
                dom.state_set("armed_cat", "")
                if map_has_key(resp, "error") => { toast_error("category", resp) render_cats_manager() }
                else => { toast("category deleted", "ok") load_categories() }
            })
        }
        else => { dom.state_set("armed_cat", id) render_cats_manager() }
    }
}

fn on_cats_change(e) = {
    let tid = map_get(e, "id")
    if starts_with(tid, "cname-") => {
        let id = str.substring(tid, 6, len(tid))
        let name = str.trim(map_get(e, "value"))
        if name == "" => { toast("category: name must not be empty", "err") render_cats_manager() }
        else => dom.fetch_json("PATCH", "/api/categories/" + id,
            "{\"name\": \"" + json_esc(name) + "\"}", (resp) => {
                if map_has_key(resp, "error") => { toast_error("rename", resp) render_cats_manager() }
                else => { toast("renamed", "ok") load_categories() }
            })
    }
}

// ── boot: bind once, render forever ────────────────────────────────────

dom.on(dom.query("#month-prev"), "click", (e) => { goto_month(month_add(cur_month(), -1)) })
dom.on(dom.query("#month-next"), "click", (e) => { goto_month(month_add(cur_month(), 1)) })
dom.on(dom.query("#month-today"), "click", (e) => { goto_month(this_month()) })
dom.on(dom.query("#add-btn"), "click", (e) => { add_transaction() })
dom.on(dom.query("#new-amount"), "enter", (e) => { add_transaction() })
dom.on(dom.query("#new-note"), "enter", (e) => { add_transaction() })
dom.on(dom.query("#new-category"), "change", (e) => { update_kind_tag() })
dom.on(dom.query("#rows"), "click", (e) => { on_rows_click(map_get(e, "id")) })
dom.on(dom.query("#rows"), "change", (e) => { on_rows_change(e) })
dom.on(dom.query("#budgets"), "change", (e) => { on_budget_change(e) })
dom.on(dom.query("#cats"), "click", (e) => { on_cats_click(map_get(e, "id")) })
dom.on(dom.query("#cats"), "change", (e) => { on_cats_change(e) })
dom.on(dom.query("#cat-add-btn"), "click", (e) => { add_category() })
dom.on(dom.query("#cat-name"), "enter", (e) => { add_category() })
dom.on_route((r) => { reload() })

// One delegated tooltip covers every chart rendered into the grid.
viz.tooltip(dom.query("#charts"))

dom.set_value(dom.query("#new-date"), dates.today())
toast("frontend: olang (wasm) · charts: ods + viz, in the browser", "hint")
load_categories()
reload()
