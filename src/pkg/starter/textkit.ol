//! textkit — text layout and humane formatting.
//!
//! The functions every report, table, and CLI ends up writing by hand:
//! padding, centering, word wrap, aligned columns, and the humane
//! renderings of money, byte counts, and durations. Pure olang, no
//! effects — every function is a value in, a string (or list) out.

/// Pad `s` with spaces on the right to width `w`; longer strings pass
/// through untouched.
share fn rpad(s, w) = {
    let g = w - len(s)
    if g > 0 => s + str.repeat(" ", g) else => s
}

/// Pad `s` with spaces on the left to width `w`; longer strings pass
/// through untouched. The right-alignment half of a numeric column.
share fn lpad(s, w) = {
    let g = w - len(s)
    if g > 0 => str.repeat(" ", g) + s else => s
}

/// Center `s` in width `w`, extra space going to the right.
share fn center(s, w) = {
    let g = w - len(s)
    if g <= 0 => s
    else => str.repeat(" ", g / 2) + s + str.repeat(" ", g - g / 2)
}

/// A horizontal rule of `w` box-drawing dashes: `rule(6)` is "──────".
share fn rule(w) = str.repeat("─", w)

/// Greedy word wrap: `text` broken into lines at most `width` wide,
/// returned as a list of lines. A single word longer than `width`
/// stands alone on its own line rather than being split.
share fn wrap(text, width) = {
    let words = text |> str.split(" ") |> filter((w) => len(w) > 0)
    let mut lines = []
    let mut line = ""
    for w in words {
        if len(line) == 0 => { line = w }
        else if len(line) + 1 + len(w) <= width => { line = line + " " + w }
        else => { lines = lines + [line]; line = w }
    }
    if len(line) > 0 => { lines = lines + [line] }
    lines
}

/// Align rows of cells into columns, `gap` spaces apart. `rows` is a
/// list of rows, each a list of strings; every column takes the width
/// of its widest cell. Returns the rendered lines.
share fn columns(rows, gap) = {
    if len(rows) == 0 => []
    else => {
        let ncols = fold(rows, 0, (m, r) => math.max(m, len(r)))
        let widths = map(range(0, ncols), (c) =>
            fold(rows, 0, (m, r) => if c < len(r) => math.max(m, len(r[c])) else => m))
        map(rows, (r) => {
            let cells = map(range(0, len(r)), (c) =>
                if c == len(r) - 1 => r[c] else => rpad(r[c], widths[c]))
            str.trim_end(str.join(cells, str.repeat(" ", gap)))
        })
    }
}

/// A finished table: `headers` (a list of strings) over `rows`, with a
/// rule between, as one printable string.
share fn table(headers, rows) = {
    let all = [headers] + rows
    let lines = columns(all, 2)
    let width = fold(lines, 0, (m, l) => math.max(m, len(l)))
    str.join([lines[0], rule(width)] + tail(lines), "\n")
}

/// Group the digits of a non-negative integer with commas: 1234567
/// becomes "1,234,567".
fn group_digits(n) = {
    let digits = to_string(n)
    let count = len(digits)
    let mut out = ""
    for i in range(0, count) {
        let remaining = count - i
        if i > 0 && remaining % 3 == 0 => { out = out + "," }
        out = out + str.char_at(digits, i)
    }
    out
}

/// Integer cents as currency, negative-safe and comma-grouped:
/// `money(123456789)` is "$1,234,567.89", `money(-150)` is "-$1.50".
/// Keeping amounts in cents end to end is what makes them exact.
share fn money(cents) = {
    let neg = cents < 0
    let abs = if neg => 0 - cents else => cents
    let body = group_digits(abs / 100) + "." + str.pad_start(to_string(abs % 100), 2, "0")
    if neg => "-$" + body else => "$" + body
}

/// A byte count at a humane scale with one decimal: `human_bytes(1536)`
/// is "1.5 KB". Powers of 1024, up to TB.
share fn human_bytes(n) = {
    let mut value = n * 10
    let mut unit = "B"
    for next in ["KB", "MB", "GB", "TB"] {
        if value >= 10240 => { value = value / 1024; unit = next }
    }
    if unit == "B" => `${n} B`
    else => `${value / 10}.${value % 10} ${unit}`
}

/// A millisecond count as people say it: "412ms", "3.2s", "5m 07s",
/// "2h 05m".
share fn human_duration(ms) = {
    if ms < 1000 => `${ms}ms`
    else if ms < 60000 => `${ms / 1000}.${ms % 1000 / 100}s`
    else if ms < 3600000 => {
        let s = ms / 1000
        `${s / 60}m ${str.pad_start(to_string(s % 60), 2, "0")}s`
    }
    else => {
        let m = ms / 60000
        `${m / 60}h ${str.pad_start(to_string(m % 60), 2, "0")}m`
    }
}

/// A string reduced to a url- and filename-safe slug: lowercase,
/// alphanumerics kept, everything else collapsed to single dashes.
/// `slugify("Hello, World!")` is "hello-world".
share fn slugify(s) = {
    let lower = str.to_lower(s)
    let mut out = ""
    let mut pending = false
    for c in str.chars(lower) {
        let keep = (c >= "a" && c <= "z") || (c >= "0" && c <= "9")
        if keep => {
            if pending && len(out) > 0 => { out = out + "-" }
            out = out + c
            pending = false
        }
        else => { pending = true }
    }
    out
}

test "padding and centering" {
    assert_eq(rpad("ab", 5), "ab   ")
    assert_eq(lpad("42", 5), "   42")
    assert_eq(rpad("longer", 3), "longer")
    assert_eq(center("hi", 6), "  hi  ")
    assert_eq(center("hi", 5), " hi  ")
    assert_eq(rule(3), "───")
}

test "wrap breaks at word boundaries" {
    assert_eq(wrap("the quick brown fox", 10), ["the quick", "brown fox"])
    assert_eq(wrap("indivisible", 4), ["indivisible"])
    assert_eq(wrap("", 10), [])
}

test "columns and table align" {
    let lines = columns([["a", "bb"], ["ccc", "d"]], 2)
    assert_eq(lines, ["a    bb", "ccc  d"])
    let t = table(["name", "qty"], [["apples", "3"], ["figs", "12"]])
    assert_eq(t, "name    qty\n───────────\napples  3\nfigs    12")
}

test "money is exact, grouped, negative-safe" {
    assert_eq(money(123456789), "$1,234,567.89")
    assert_eq(money(5), "$0.05")
    assert_eq(money(-150), "-$1.50")
    assert_eq(money(0), "$0.00")
}

test "humane scales" {
    assert_eq(human_bytes(512), "512 B")
    assert_eq(human_bytes(1536), "1.5 KB")
    assert_eq(human_bytes(3 * 1024 * 1024), "3.0 MB")
    assert_eq(human_duration(412), "412ms")
    assert_eq(human_duration(3200), "3.2s")
    assert_eq(human_duration(307000), "5m 07s")
    assert_eq(human_duration(7500000), "2h 05m")
}

test "slugify" {
    assert_eq(slugify("Hello, World!"), "hello-world")
    assert_eq(slugify("  a  b  "), "a-b")
    assert_eq(slugify("v2.0 release"), "v2-0-release")
}
