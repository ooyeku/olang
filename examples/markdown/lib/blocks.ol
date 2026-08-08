// Block-level parsing: the document's lines are grouped into blocks —
// fenced code, headings, horizontal rules, blockquotes, lists, paragraphs —
// each rendered to an HTML element. Consecutive same-kind lines merge (list
// items into one list, quote lines into one blockquote, plain lines into
// one paragraph); a blank line separates blocks.

use lib.inline { escape, render_inline }

// How many leading `#` characters (up to 6), for headings.
fn heading_level(line) = {
    let mut level = 0
    while (level < 6) && (str.char_at(line, level) == "#") { level = level + 1 }
    if (level > 0) && (str.char_at(line, level) == " ") => level else => 0
}

// Is this line an ordered-list item like `12. text`? Returns the dot index
// (so the text starts at dot + 2), or -1.
fn ordered_prefix(line) = {
    let dot = str.index_of(line, ". ")
    if dot <= 0 => -1
    else if is_ok(str.parse_int(str.substring(line, 0, dot))) => dot
    else => -1
}

fn is_blank(line) = str.length(str.trim(line)) == 0
fn is_quote(line) = str.starts_with(line, "> ") || (str.trim(line) == ">")
fn is_bullet(line) = str.starts_with(line, "- ")
fn is_rule(line) = {
    let t = str.trim(line)
    (t == "---") || (t == "***") || (t == "___")
}

// Convert a whole markdown document to HTML.
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
            let tag = "h" + to_string(level)
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
                items = items + ["<li>" + render_inline(text) + "</li>"]
                i = i + 1
            }
            out = out + ["<ul>" + join(items, "") + "</ul>"]
        }

        else if ordered_prefix(line) >= 0 => {
            let mut items = []
            while (i < n) && (ordered_prefix(lines[i]) >= 0) {
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
                    || is_rule(l) || is_quote(l) || is_bullet(l) || (ordered_prefix(l) >= 0) =>
                    { go = false }
                else => { para = para + [str.trim(l)]; i = i + 1 }
            }
            out = out + ["<p>" + render_inline(join(para, " ")) + "</p>"]
        }
    }

    join(out, "\n")
}
