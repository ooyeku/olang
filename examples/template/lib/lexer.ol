// Scan raw template text into a flat token stream. The scanner walks a
// shrinking `rest` string, slicing out `{{ ... }}` tags and the literal text
// around them — exercising str.index_of / substring / starts_with.

use lib.ast { Token, TText, TVar, TOpenEach, TOpenIf, TClose }

// Turn the trimmed inside of a `{{ ... }}` tag into the right token.
fn classify(inner) = if str.starts_with(inner, "#each ") =>
        TOpenEach(str.trim(str.substring(inner, 6, str.length(inner))))
    else if str.starts_with(inner, "#if ") =>
        TOpenIf(str.trim(str.substring(inner, 4, str.length(inner))))
    else if str.starts_with(inner, "/") =>
        TClose(str.trim(str.substring(inner, 1, str.length(inner))))
    else =>
        TVar(inner)

share fn tokenize(src) = {
    let mut tokens = []
    let mut rest = src
    while str.length(rest) > 0 {
        let open = str.index_of(rest, "{{")
        if open == () => {
            // no more tags: the remainder is all literal text
            tokens = tokens + [TText(rest)]
            rest = ""
        } else => {
            if open > 0 => {
                tokens = tokens + [TText(str.substring(rest, 0, open))]
            }
            let after = str.substring(rest, open + 2, str.length(rest))
            let close = str.index_of(after, "}}")
            let inner = str.trim(str.substring(after, 0, close))
            tokens = tokens + [classify(inner)]
            rest = str.substring(after, close + 2, str.length(after))
        }
    }
    tokens
}
