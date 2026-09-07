// term — the terminal toolkit, an embedded olang package (`use term`).
//
// What `viz` is for the browser, for the terminal: color and text
// styling, aligned tables and rules, progress bars, and interactive
// prompts. Styling is emitted only when it will actually render —
// standard output is a TTY and `NO_COLOR` is unset — or when
// `CLICOLOR_FORCE` is set (the convention that also makes styled
// output testable through a pipe). The decision is made once and
// remembered until the program changes its environment: a color call
// is then a concatenation, and a request path that colors three
// fragments per log line pays nothing for the TTY and environment
// checks (they were 4 % of a request-heavy run).
//
//   use term
//   println(term.green("ok") + " built in " + term.bold("1.2s"))
//   println(term.table(["name", "age"], [["ada", "36"], ["eve", "41"]]))
//   print(term.bar(0.6, 20)); os.flush()

/// Whether styled output is emitted — decided once, re-decided only
/// after the program's own `os.set_env`/`os.remove_env`.
share fn color() = os.color_enabled()

let ESC = "\x1b["
let RESET = "\x1b[0m"

fn wrap(codes, s) = if color() => ESC + codes + "m" + s + RESET else => s

// ── named colors and attributes ────────────────────────────────────────

/// Wrap `s` in black.
share fn black(s) = wrap("30", s)
/// Wrap `s` in red.
share fn red(s) = wrap("31", s)
/// Wrap `s` in green.
share fn green(s) = wrap("32", s)
/// Wrap `s` in yellow.
share fn yellow(s) = wrap("33", s)
/// Wrap `s` in blue.
share fn blue(s) = wrap("34", s)
/// Wrap `s` in magenta.
share fn magenta(s) = wrap("35", s)
/// Wrap `s` in cyan.
share fn cyan(s) = wrap("36", s)
/// Wrap `s` in white.
share fn white(s) = wrap("37", s)
/// Wrap `s` in gray.
share fn gray(s) = wrap("90", s)

/// Bold `s`.
share fn bold(s) = wrap("1", s)
/// Dim `s`.
share fn dim(s) = wrap("2", s)
/// Italicize `s`.
share fn italic(s) = wrap("3", s)
/// Underline `s`.
share fn underline(s) = wrap("4", s)

// ── general style: term.style(s, #{ "fg": "red", "bg": "blue", ... }) ──

fn fg_code(name) = {
    let codes = #{ "black": "30", "red": "31", "green": "32", "yellow": "33",
                   "blue": "34", "magenta": "35", "cyan": "36", "white": "37", "gray": "90" }
    if map_has_key(codes, name) => map_get(codes, name) else => "39"
}
fn bg_code(name) = {
    let codes = #{ "black": "40", "red": "41", "green": "42", "yellow": "43",
                   "blue": "44", "magenta": "45", "cyan": "46", "white": "47", "gray": "100" }
    if map_has_key(codes, name) => map_get(codes, name) else => "49"
}
fn flag(opts, key) = if map_has_key(opts, key) => map_get(opts, key) else => false

/// Style `s` with `opts`: `fg`, `bg`, `bold`, `dim`, `italic`, `underline`.
share fn style(s, opts) = {
    let mut codes = []
    if map_has_key(opts, "fg") => { codes = codes + [fg_code(map_get(opts, "fg"))] }
    if map_has_key(opts, "bg") => { codes = codes + [bg_code(map_get(opts, "bg"))] }
    if flag(opts, "bold") => { codes = codes + ["1"] }
    if flag(opts, "dim") => { codes = codes + ["2"] }
    if flag(opts, "italic") => { codes = codes + ["3"] }
    if flag(opts, "underline") => { codes = codes + ["4"] }
    if len(codes) == 0 => s else => wrap(join(codes, ";"), s)
}

// ── structure ──────────────────────────────────────────────────────────

/// A horizontal rule of box-drawing dashes.
share fn rule(width) = str.repeat("─", width)

// Replace element `i` of a list (the width scan grows column widths).
fn with_at(xs, i, v) = map(range(0, len(xs)), (j) => if j == i => v else => xs[j])

/// The visible length of `s` — its width on screen, with ANSI styling
/// escapes discounted. `visible_len(term.red("hi"))` is 2, not 11.
share fn visible_len(s) = {
    let mut n = 0
    let mut in_esc = false
    for ch in s {
        if in_esc => { if ch == "m" => { in_esc = false } }
        else => {
            if ch == "\x1b" => { in_esc = true } else => { n = n + 1 }
        }
    }
    n
}

/// An aligned table. `headers` is a list of column titles (bold);
/// `rows` is a list of rows, each a list of cell strings. Columns are
/// padded to their widest cell by *visible* width, so styled cells
/// (colored, bold) align just as plain ones do.
share fn table(headers, rows) = {
    let ncols = len(headers)
    let mut widths = map(headers, (h) => visible_len(h))
    for row in rows {
        let mut c = 0
        while c < ncols {
            if c < len(row) => {
                let w = visible_len(row[c])
                if w > widths[c] => { widths = with_at(widths, c, w) }
            }
            c = c + 1
        }
    }
    let render_row = (cells) => {
        let mut out = ""
        let mut c = 0
        while c < ncols {
            let cell = if c < len(cells) => cells[c] else => ""
            // Pad by the invisible-escape allowance so the visible column
            // width lands exactly, whether or not the cell is styled.
            let target = widths[c] + str.length(cell) - visible_len(cell)
            out = out + str.pad_end(cell, target, " ")
            if c < ncols - 1 => { out = out + "  " }
            c = c + 1
        }
        out
    }
    let mut lines = [bold(render_row(headers))]
    for row in rows {
        lines = lines + [render_row(row)]
    }
    join(lines, "\n")
}

// ── progress ───────────────────────────────────────────────────────────

/// A progress bar for a fraction in [0, 1]: "[████████░░░░]  67%". Print
/// it with a leading "\r" and `os.flush()` in your loop to redraw in
/// place; print a newline when done.
share fn bar(fraction, width) = {
    let f = if fraction < 0.0 => 0.0 else => (if fraction > 1.0 => 1.0 else => fraction)
    let filled = to_int(f * to_float(width))
    let pct = str.pad_start(show(to_int(f * 100.0)) + "%", 4, " ")
    "[" + str.repeat("█", filled) + str.repeat("░", width - filled) + "] " + pct
}

// ── input ──────────────────────────────────────────────────────────────

fn ask(question) = {
    print(question + " ")
    os.flush()
    str.trim_end(unwrap_or(os.read_line(), ""))
}

/// Prompt for a line of input.
share fn prompt(question) = ask(question)

/// Yes/no question; anything starting with y/Y is true, else false.
share fn confirm(question) = {
    let answer = ask(question + " [y/N]")
    str.starts_with(str.to_lower(answer), "y")
}

/// A numbered menu; returns Ok(chosen option string) or Err on a bad
/// choice. Options are printed 1..N.
share fn select(question, options) = {
    println(question)
    let mut i = 0
    for o in options {
        println("  " + show(i + 1) + ") " + o)
        i = i + 1
    }
    let raw = ask("choose 1-" + show(len(options)) + ":")
    match str.parse_int(raw) {
        Err(e) => Err("not a number: " + raw),
        Ok(n) => if n >= 1 && n <= len(options) => Ok(options[n - 1])
            else => Err("out of range: " + raw)
    }
}
