//! markdown — a Markdown renderer, in olang.
//!
//! `markdown.to_html(text)` turns a Markdown document into HTML:
//! headings, paragraphs, `code` spans and fenced blocks, **strong**,
//! *emphasis*, [links](url), bulleted and numbered lists, blockquotes,
//! and horizontal rules. Inline markers nest and unmatched markers
//! fall through as literal text; everything else is escaped, so
//! untrusted input renders as text rather than markup.
//!
//! `render_inline` and `escape` are shared for callers that already
//! have their own block structure — a chat message, a table cell.

/// Escape text for use as HTML element content: &, <, > become
/// entities. Attribute values need escape_attr's quote handling too.
share fn escape(s) =
    str.replace(str.replace(str.replace(s, "&", "&amp;"), "<", "&lt;"), ">", "&gt;")

// Escape a URL for use inside a double-quoted attribute.
fn escape_attr(s) = str.replace(escape(s), "\"", "&quot;")

// Find `needle` in `s` at or after `from`; Unit if absent.
fn find_from(s, needle, from) = {
    let rest = str.substring(s, from, str.length(s))
    let rel = str.index_of(rest, needle)
    if rel == () => () else => from + rel
}

/// Render one line's inline spans to HTML: `code`, **strong**,
/// *emphasis*, and [text](url), nesting handled, unmatched markers
/// kept as literal text, everything escaped.
share fn render_inline(s) = {
    let n = str.length(s)
    let mut out = ""
    let mut i = 0
    while i < n {
        let c = str.char_at(s, i)
        let two = str.substring(s, i, i + 2)
        if c == "`" => {
            let close = find_from(s, "`", i + 1)
            if close == () => { out = out + "`"; i = i + 1 }
            else => {
                out = out + "<code>" + escape(str.substring(s, i + 1, close)) + "</code>"
                i = close + 1
            }
        }
        else if two == "**" => {
            let close = find_from(s, "**", i + 2)
            if close == () => { out = out + escape(two); i = i + 2 }
            else => {
                out = out + "<strong>" + render_inline(str.substring(s, i + 2, close)) + "</strong>"
                i = close + 2
            }
        }
        else if c == "*" => {
            let close = find_from(s, "*", i + 1)
            if close == () => { out = out + "*"; i = i + 1 }
            else => {
                out = out + "<em>" + render_inline(str.substring(s, i + 1, close)) + "</em>"
                i = close + 1
            }
        }
        else if c == "[" => {
            let mid = find_from(s, "](", i + 1)
            let close = if mid == () => () else => find_from(s, ")", mid + 2)
            if (mid == ()) || (close == ()) => { out = out + "["; i = i + 1 }
            else => {
                let text = str.substring(s, i + 1, mid)
                let url = str.substring(s, mid + 2, close)
                out = out + "<a href=\"" + escape_attr(url) + "\">" + render_inline(text) + "</a>"
                i = close + 1
            }
        }
        else => {
            out = out + escape(c)
            i = i + 1
        }
    }
    out
}

// How many leading `#` characters (up to 6), for headings.
fn heading_level(line) = {
    let mut level = 0
    while (level < 6) && (str.char_at(line, level) == "#") { level = level + 1 }
    if (level > 0) && (str.char_at(line, level) == " ") => level else => 0
}

// Is this line an ordered-list item like `12. text`? Returns the dot index
// (so the text starts at dot + 2), or Unit.
fn ordered_prefix(line) = {
    let dot = str.index_of(line, ". ")
    if (dot == ()) || (dot == 0) => ()
    else if is_ok(str.parse_int(str.substring(line, 0, dot))) => dot
    else => ()
}

fn is_blank(line) = str.length(str.trim(line)) == 0
fn is_quote(line) = str.starts_with(line, "> ") || (str.trim(line) == ">")
/// One bullet's `<li>`. GitHub-style task items — `[ ]` / `[x]` at the
/// start — render as a disabled checkbox inside `<li class="task">`,
/// kept in source order so a click can map back to the n-th task line.
fn list_item(text) = {
    let t = str.trim_start(text)
    if str.starts_with(t, "[ ] ") || t == "[ ]" =>
        "<li class=\"task\"><input type=\"checkbox\" disabled> "
            + render_inline(str.trim_start(str.substring(t, 3, str.length(t)))) + "</li>"
    else if str.starts_with(t, "[x] ") || str.starts_with(t, "[X] ") || t == "[x]" =>
        "<li class=\"task done\"><input type=\"checkbox\" disabled checked> "
            + render_inline(str.trim_start(str.substring(t, 3, str.length(t)))) + "</li>"
    else => "<li>" + render_inline(text) + "</li>"
}

fn is_bullet(line) = str.starts_with(line, "- ")
fn is_rule(line) = {
    let t = str.trim(line)
    (t == "---") || (t == "***") || (t == "___")
}

/// Convert a whole Markdown document to HTML. Blocks are separated
/// by blank lines; consecutive list items, quote lines, and plain
/// lines merge into one element each.
share fn to_html(md) = {
    let lines = str.lines(md)
    let n = len(lines)
    let mut out = []
    let mut i = 0

    while i < n {
        let line = lines[i]

        if is_blank(line) => { i = i + 1 }

        else if str.starts_with(line, "```") => {
            // fenced code: verbatim until the closing fence, escaped once
            let mut body = []
            i = i + 1
            while (i < n) && (!str.starts_with(lines[i], "```")) {
                body = body + [escape(lines[i])]
                i = i + 1
            }
            i = i + 1   // skip the closing fence
            out = out + ["<pre><code>" + join(body, "\n") + "</code></pre>"]
        }

        else if heading_level(line) > 0 => {
            let level = heading_level(line)
            let text = str.substring(line, level + 1, str.length(line))
            let tag = `h${level}`
            out = out + ["<" + tag + ">" + render_inline(str.trim(text)) + "</" + tag + ">"]
            i = i + 1
        }

        else if is_rule(line) => { out = out + ["<hr>"]; i = i + 1 }

        else if is_quote(line) => {
            let mut quoted = []
            while (i < n) && is_quote(lines[i]) {
                let t = str.trim(lines[i])
                quoted = quoted + [str.trim(str.substring(t, 1, str.length(t)))]
                i = i + 1
            }
            out = out + ["<blockquote><p>" + render_inline(join(quoted, " ")) + "</p></blockquote>"]
        }

        else if is_bullet(line) => {
            let mut items = []
            while (i < n) && is_bullet(lines[i]) {
                let text = str.substring(lines[i], 2, str.length(lines[i]))
                items = items + [list_item(text)]
                i = i + 1
            }
            out = out + ["<ul>" + join(items, "") + "</ul>"]
        }

        else if ordered_prefix(line) != () => {
            let mut items = []
            while (i < n) && (ordered_prefix(lines[i]) != ()) {
                let dot = ordered_prefix(lines[i])
                let text = str.substring(lines[i], dot + 2, str.length(lines[i]))
                items = items + ["<li>" + render_inline(text) + "</li>"]
                i = i + 1
            }
            out = out + ["<ol>" + join(items, "") + "</ol>"]
        }

        else => {
            // paragraph: consecutive plain lines joined with a space
            let mut para = []
            let mut go = true
            while go && (i < n) {
                let l = lines[i]
                if is_blank(l) || str.starts_with(l, "```") || (heading_level(l) > 0)
                    || is_rule(l) || is_quote(l) || is_bullet(l) || (ordered_prefix(l) != ()) =>
                    { go = false }
                else => { para = para + [str.trim(l)]; i = i + 1 }
            }
            out = out + ["<p>" + render_inline(join(para, " ")) + "</p>"]
        }
    }

    join(out, "\n")
}

test "inline spans render and nest" {
    assert_eq(render_inline("plain"), "plain")
    assert_eq(render_inline("a **b** c"), "a <strong>b</strong> c")
    assert_eq(render_inline("**b *i* b**"), "<strong>b <em>i</em> b</strong>")
    assert_eq(render_inline("`<x>`"), "<code>&lt;x&gt;</code>")
    assert_eq(render_inline("[go](http://a.b)"), "<a href=\"http://a.b\">go</a>")
    assert_eq(render_inline("2 * 3"), "2 * 3")
}

test "escaping keeps untrusted input textual" {
    assert_eq(escape("<script>"), "&lt;script&gt;")
    assert_eq(to_html("<b>hi</b>"), "<p>&lt;b&gt;hi&lt;/b&gt;</p>")
}

test "blocks" {
    assert_eq(to_html("# Title"), "<h1>Title</h1>")
    assert_eq(to_html("- a\n- b"), "<ul><li>a</li><li>b</li></ul>")
    assert_eq(to_html("- [ ] write tests\n- [x] ship"),
        "<ul><li class=\"task\"><input type=\"checkbox\" disabled> write tests</li>"
        + "<li class=\"task done\"><input type=\"checkbox\" disabled checked> ship</li></ul>")
    assert_eq(to_html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>")
    assert_eq(to_html("> quoted\n> words"), "<blockquote><p>quoted words</p></blockquote>")
    assert_eq(to_html("---"), "<hr>")
    assert_eq(to_html("```\ncode < here\n```"), "<pre><code>code &lt; here</code></pre>")
    assert_eq(to_html("one\ntwo\n\nthree"), "<p>one two</p>\n<p>three</p>")
}
