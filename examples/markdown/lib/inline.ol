// Inline span rendering: the text inside a block becomes HTML. The renderer
// scans left to right; markers open spans whose inner text is rendered
// recursively (so **bold with *italic* inside** nests), while code spans
// take their content verbatim (escaped, never re-parsed). Unmatched markers
// fall through as literal text.
//
// Supported: `code`, **strong**, *emphasis*, [text](url). Everything else is
// escaped literal text.

// Escape text for HTML element content.
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
