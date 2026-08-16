// oshell lexer: a line of shell input → tokens.
//
// A token is { t, q } where q marks how it may be treated later:
//   "w" — plain word: variable expansion, ~, and globbing apply
//   "r" — raw word (some part was single-quoted): no expansion at all
//   "o" — operator: | ; < > >> && ||
//
// Quoting follows the shell conventions that matter: single quotes are
// literal to the closing quote, double quotes group words but leave $
// expansion on (with \" \\ \$ escapes), backslash escapes the next
// character outside quotes, quoted and bare segments concatenate into
// one token (--name="a b" is one word), and # starts a comment only at
// the start of a token.

share fn tokenize(line) = {
    let cs = str.chars(line)
    let n = len(cs)
    let mut tokens = []
    let mut cur = ""
    let mut has_cur = false
    let mut saw_single = false
    let mut err = ""
    let mut done = false
    let mut i = 0

    while i < n && err == "" && !done {
        let c = cs[i]
        if c == " " || c == "\t" => {
            if has_cur => {
                tokens = tokens + [{ t: cur, q: if saw_single => "r" else => "w" }]
                cur = ""
                has_cur = false
                saw_single = false
            }
            i = i + 1
        } else => if c == "#" && !has_cur => {
            done = true
        } else => if is_op_char(c) => {
            if has_cur => {
                tokens = tokens + [{ t: cur, q: if saw_single => "r" else => "w" }]
                cur = ""
                has_cur = false
                saw_single = false
            }
            let two = if i + 1 < n => c + cs[i + 1] else => c
            if two == ">>" || two == "&&" || two == "||" => {
                tokens = tokens + [{ t: two, q: "o" }]
                i = i + 2
            } else => if c == "&" => {
                err = "oshell: background jobs (&) are not supported"
            } else => {
                tokens = tokens + [{ t: c, q: "o" }]
                i = i + 1
            }
        } else => if c == "'" => {
            let mut closed = false
            i = i + 1
            while i < n && !closed {
                if cs[i] == "'" => { closed = true } else => { cur = cur + cs[i] }
                i = i + 1
            }
            if !closed => { err = "oshell: unterminated single quote" }
            has_cur = true
            saw_single = true
        } else => if c == "\"" => {
            let mut closed = false
            i = i + 1
            while i < n && !closed {
                let d = cs[i]
                if d == "\"" => {
                    closed = true
                    i = i + 1
                } else => if d == "\\" && i + 1 < n && (cs[i + 1] == "\"" || cs[i + 1] == "\\" || cs[i + 1] == "$") => {
                    cur = cur + cs[i + 1]
                    i = i + 2
                } else => {
                    cur = cur + d
                    i = i + 1
                }
            }
            if !closed => { err = "oshell: unterminated double quote" }
            has_cur = true
        } else => if c == "\\" && i + 1 < n => {
            cur = cur + cs[i + 1]
            has_cur = true
            // Escaping an expansion character makes the token literal —
            // \$HOME must not expand. (Raw applies token-wide; close
            // enough to the shell rule and documented.)
            if cs[i + 1] == "$" || cs[i + 1] == "*" || cs[i + 1] == "?" || cs[i + 1] == "~" => {
                saw_single = true
            }
            i = i + 2
        } else => {
            cur = cur + c
            has_cur = true
            i = i + 1
        }
    }

    if err != "" => Err(err)
    else => {
        if has_cur => {
            tokens = tokens + [{ t: cur, q: if saw_single => "r" else => "w" }]
        }
        Ok(tokens)
    }
}

fn is_op_char(c) = c == "|" || c == ";" || c == "<" || c == ">" || c == "&"
