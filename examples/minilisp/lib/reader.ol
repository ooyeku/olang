// The reader: source text -> LVal syntax trees.
//
// A tokenizer over str.* primitives feeds a recursive-descent reader. Lists
// are olang lists of LVal; symbols and numbers are enum payloads. The reader
// returns Ok(list of top-level forms) or Err(message) — no exceptions.

share type LVal = enum {
    LNum(Int),
    LSym(String),
    LBool(Bool),
    LStr(String),
    LList(List),
    LFn(List, LVal, Map),
    LRec(String, List, LVal, Map),
    LPrim(String)
}

// ── tokenizer ──

fn is_digit(c) = c >= "0" && c <= "9"
fn is_space(c) = c == " " || c == "\n" || c == "\t"
fn is_delim(c) = c == "(" || c == ")" || is_space(c)

share fn tokenize(src) = {
    let n = str.length(src)
    let mut tokens = []
    let mut i = 0
    while i < n {
        let c = str.char_at(src, i)
        if is_space(c) => { i = i + 1 }
        else if c == ";" => {
            // comment to end of line
            while i < n && str.char_at(src, i) != "\n" { i = i + 1 }
        }
        else if c == "(" || c == ")" => {
            tokens = concat(tokens, [c])
            i = i + 1
        }
        else if c == "\"" => {
            let mut j = i + 1
            while j < n && str.char_at(src, j) != "\"" { j = j + 1 }
            tokens = concat(tokens, [str.substring(src, i, j + 1)])
            i = j + 1
        }
        else => {
            let mut j = i
            while j < n && !is_delim(str.char_at(src, j)) { j = j + 1 }
            tokens = concat(tokens, [str.substring(src, i, j)])
            i = j
        }
    }
    tokens
}

// ── reader ──

fn atom(tok) = {
    let first = str.char_at(tok, 0)
    if tok == "true" => LBool(true)
    else if tok == "false" => LBool(false)
    else if first == "\"" => LStr(str.substring(tok, 1, str.length(tok) - 1))
    else if is_digit(first) || (first == "-" && str.length(tok) > 1 && is_digit(str.char_at(tok, 1))) =>
        match str.parse_int(tok) { Ok(v) => LNum(v), Err(_) => LSym(tok) }
    else => LSym(tok)
}

// Read one form starting at index i; returns Ok((form, next_index)) or Err.
fn read_form(tokens, i) = {
    if i >= len(tokens) => Err("unexpected end of input")
    else => {
        let tok = tokens[i]
        if tok == "(" => read_list(tokens, i + 1, [])
        else if tok == ")" => Err("unexpected )")
        else => Ok((atom(tok), i + 1))
    }
}

fn read_list(tokens, i, acc) = {
    if i >= len(tokens) => Err("unclosed (")
    else if tokens[i] == ")" => Ok((LList(acc), i + 1))
    else => match read_form(tokens, i) {
        Ok(pair) => {
            let (form, next) = pair
            read_list(tokens, next, concat(acc, [form]))
        },
        Err(e) => Err(e)
    }
}

// Parse a whole program: a list of top-level forms.
share fn read_program(src) = {
    let mut tokens = tokenize(src)
    let mut forms = []
    let mut i = 0
    let mut failure = ""
    while i < len(tokens) && failure == "" {
        match read_form(tokens, i) {
            Ok(pair) => {
                let (form, next) = pair
                forms = concat(forms, [form])
                i = next
            },
            Err(e) => { failure = e }
        }
    }
    if failure == "" => Ok(forms) else => Err(failure)
}
