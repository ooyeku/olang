// Money and month formatting. Money is integer cents everywhere in the
// ledger; these helpers are the only place cents meet strings. The
// frontend (static/ledger.ol) carries copies of money/to_cents because it
// is served as a standalone file — the test blocks here are the reference
// behavior for both.

// "1234567" -> "1,234,567"
share fn thousands(n: Int) = {
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

// -123456 cents -> "-$1,234.56". Negative-safe, always two decimals.
share fn money(cents: Int) = {
    let sign = if cents < 0 => "-" else => ""
    let a = if cents < 0 => 0 - cents else => cents
    let frac = a % 100
    let frac_s = if frac < 10 => "0" + to_string(frac) else => to_string(frac)
    sign + "$" + thousands(a / 100) + "." + frac_s
}

// "12.50" | "12.5" | "12" | "-3.07" -> Ok(cents). Rejects more than two
// decimals and anything non-numeric.
share fn to_cents(s: String) = {
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

// ── months ("YYYY-MM" keys) ──────────────────────────────────────────

share fn this_month() = unwrap(dates.format_date(dates.today(), "%Y-%m"))

share fn month_of(date: String) = unwrap(dates.format_date(date, "%Y-%m"))

// "2026-08" + (-1) -> "2026-07"; clamping and year wrap come from
// dates.add_months on the month's first day.
share fn month_add(month: String, delta: Int) =
    unwrap(dates.format_date(unwrap(dates.add_months(month + "-01", delta)), "%Y-%m"))

// ── module self-checks (olang test .) ────────────────────────────────

test "money is negative-safe with two decimals and separators" {
    assert_eq(money(0), "$0.00")
    assert_eq(money(5), "$0.05")
    assert_eq(money(1250), "$12.50")
    assert_eq(money(-1250), "-$12.50")
    assert_eq(money(123456789), "$1,234,567.89")
    assert_eq(money(-100), "-$1.00")
    assert_eq(money(-5), "-$0.05")
}

test "to_cents parses amounts and rejects junk" {
    assert_eq(unwrap(to_cents("12.50")), 1250)
    assert_eq(unwrap(to_cents("12.5")), 1250)
    assert_eq(unwrap(to_cents("12")), 1200)
    assert_eq(unwrap(to_cents("-3.07")), -307)
    assert_eq(unwrap(to_cents(".5")), 50)
    assert_eq(unwrap(to_cents("0")), 0)
    assert_true(is_err(to_cents("12.505")), "three decimals rejected")
    assert_true(is_err(to_cents("abc")), "words rejected")
    assert_true(is_err(to_cents("")), "empty rejected")
    assert_true(is_err(to_cents("1.2.3")), "double dot rejected")
}

test "month arithmetic wraps years and formats keys" {
    assert_eq(month_add("2026-08", -1), "2026-07")
    assert_eq(month_add("2026-01", -1), "2025-12")
    assert_eq(month_add("2026-12", 1), "2027-01")
    assert_eq(month_add("2026-08", -5), "2026-03")
    assert_eq(month_of("2026-08-15"), "2026-08")
}
