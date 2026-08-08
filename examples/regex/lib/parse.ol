// A recursive-descent parser from regex syntax to the `Re` AST. The grammar,
// lowest precedence first:
//
//   alt     = concat ('|' concat)*
//   concat  = repeat*
//   repeat  = atom ('*' | '+' | '?')*
//   atom    = '(' alt ')' | '[' class ']' | '.' | '^' | '$' | '\' esc | char
//
// Each function threads the position through its result record { node, pos };
// no backtracking is needed since regex syntax is LL(1).

use lib.ast {
    Re, Empty, Lit, Any, Class, Concat, Alt, Star, Plus, Opt, AtStart, AtEnd
}

fn at(pat, pos) = str.char_at(pat, pos)
fn len(pat) = str.length(pat)

// alt = concat ('|' concat)*
fn parse_alt(pat, pos) = {
    let first = parse_concat(pat, pos)
    let mut node = first.node
    let mut p = first.pos
    while (p < len(pat)) && (at(pat, p) == "|") {
        let rhs = parse_concat(pat, p + 1)
        node = Alt(node, rhs.node)
        p = rhs.pos
    }
    { node: node, pos: p }
}

// concat = repeat*  (until '|', ')', or end)
fn parse_concat(pat, pos) = {
    let mut node = Empty
    let mut p = pos
    let mut first = true
    while (p < len(pat)) && (at(pat, p) != "|") && (at(pat, p) != ")") {
        let r = parse_repeat(pat, p)
        node = if first => r.node else => Concat(node, r.node)
        first = false
        p = r.pos
    }
    { node: node, pos: p }
}

// repeat = atom ('*' | '+' | '?')*
fn parse_repeat(pat, pos) = {
    let a = parse_atom(pat, pos)
    let mut node = a.node
    let mut p = a.pos
    let mut go = true
    while go && (p < len(pat)) {
        let c = at(pat, p)
        if c == "*" => { node = Star(node); p = p + 1 }
        else if c == "+" => { node = Plus(node); p = p + 1 }
        else if c == "?" => { node = Opt(node); p = p + 1 }
        else => { go = false }
    }
    { node: node, pos: p }
}

// A backslash escape: \d \w \s expand to classes; anything else is a literal
// (so `\.` matches a real dot).
fn parse_escape(pat, pos) = {
    let c = at(pat, pos)
    if c == "d" => { node: Class([["0", "9"]], false), pos: pos + 1 }
    else if c == "w" => { node: Class([["a", "z"], ["A", "Z"], ["0", "9"], ["_", "_"]], false), pos: pos + 1 }
    else if c == "s" => { node: Class([[" ", " "], ["\t", "\t"], ["\n", "\n"]], false), pos: pos + 1 }
    else => { node: Lit(c), pos: pos + 1 }
}

// class body after '[', up to ']'. A leading '^' negates; `a-z` is a range.
fn parse_class(pat, pos) = {
    let mut p = pos
    let mut neg = false
    if (p < len(pat)) && (at(pat, p) == "^") => { neg = true; p = p + 1 }
    let mut ranges = []
    let mut go = true
    while go && (p < len(pat)) && (at(pat, p) != "]") {
        let lo = at(pat, p)
        if ((p + 2) < len(pat)) && (at(pat, p + 1) == "-") && (at(pat, p + 2) != "]") => {
            ranges = ranges + [[lo, at(pat, p + 2)]]
            p = p + 3
        }
        else => {
            ranges = ranges + [[lo, lo]]
            p = p + 1
        }
    }
    { node: Class(ranges, neg), pos: p + 1 } // skip ']'
}

// atom = group | class | '.' | anchors | escape | literal char
fn parse_atom(pat, pos) = {
    let c = at(pat, pos)
    if c == "(" => {
        let inner = parse_alt(pat, pos + 1)
        { node: inner.node, pos: inner.pos + 1 } // skip ')'
    }
    else if c == "[" => parse_class(pat, pos + 1)
    else if c == "." => { node: Any, pos: pos + 1 }
    else if c == "^" => { node: AtStart, pos: pos + 1 }
    else if c == "$" => { node: AtEnd, pos: pos + 1 }
    else if c == "\\" => parse_escape(pat, pos + 1)
    else => { node: Lit(c), pos: pos + 1 }
}

// Parse a whole pattern into a Re tree.
share fn parse(pattern) = parse_alt(pattern, 0).node
