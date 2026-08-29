// A parser combinator library, self-hosted in olang.
//
// A parser is a function `(input, pos) -> result`, where `result` is a record
//   { ok, val, pos, exp }
// On success `ok` is true, `val` is the produced value, and `pos` is the index
// just past what was consumed. On failure `ok` is false, `pos` is where the
// parse stalled, and `exp` names what was expected — enough for an error
// message. Parsers never mutate input; backtracking is just reusing the
// original `pos`, so `alt` is free.
//
// Combinators take parsers and return parsers — ordinary function values, so
// the whole library is closures over closures. Recursive grammars are written
// as named functions (see calc.ol), which defer and can refer to each other.

// ── result constructors ────────────────────────────────────────────────
fn win(val, pos) = { ok: true, val: val, pos: pos, exp: "" }
fn lose(pos, exp) = { ok: false, val: "", pos: pos, exp: exp }

// ── character predicates (string ordering drives the ranges) ────────────
share fn is_digit(ch) = (ch >= "0") && (ch <= "9")
share fn is_alpha(ch) = ((ch >= "a") && (ch <= "z")) || ((ch >= "A") && (ch <= "Z"))
share fn is_space(ch) = (ch == " ") || (ch == "\t") || (ch == "\n")

// ── primitive parsers ──────────────────────────────────────────────────

// Succeed with `v`, consuming nothing.
share fn pure(v) = (input, pos) => win(v, pos)

// Match one character satisfying `pred`; `exp` labels it for errors.
share fn satisfy(exp, pred) = (input, pos) =>
    if (pos < str.length(input)) && pred(str.char_at(input, pos)) =>
        win(str.char_at(input, pos), pos + 1)
    else => lose(pos, exp)

// Match a specific character.
share fn char_p(c) = satisfy("'" + c + "'", (ch) => ch == c)

// Match a literal string.
share fn str_p(s) = (input, pos) => {
    let n = str.length(s)
    let mut i = 0
    let mut good = true
    while (i < n) && good {
        if ((pos + i) < str.length(input)) && (str.char_at(input, pos + i) == str.char_at(s, i)) =>
            { i = i + 1 }
        else => { good = false }
    }
    if good => win(s, pos + n) else => lose(pos, "\"" + s + "\"")
}

// ── combinators ────────────────────────────────────────────────────────

// Transform a parser's result value.
share fn pmap(p, f) = (input, pos) => {
    let r = p(input, pos)
    if r.ok => win(f(r.val), r.pos) else => r
}

// Run two parsers in order, yielding the pair [v1, v2].
share fn seq(p1, p2) = (input, pos) => {
    let r1 = p1(input, pos)
    if !r1.ok => r1 else => {
        let r2 = p2(input, r1.pos)
        if !r2.ok => r2 else => win([r1.val, r2.val], r2.pos)
    }
}

// Sequence, keeping only the left or only the right result.
share fn keep_left(p1, p2) = pmap(seq(p1, p2), (pair) => pair[0])
share fn keep_right(p1, p2) = pmap(seq(p1, p2), (pair) => pair[1])

// Try `p1`; on failure, try `p2` from the original position (backtracking).
share fn alt(p1, p2) = (input, pos) => {
    let r1 = p1(input, pos)
    if r1.ok => r1 else => p2(input, pos)
}

// First succeeding parser from a list, all tried at the same position.
share fn alts(ps) = (input, pos) => {
    let mut i = 0
    let mut res = lose(pos, "no alternative matched")
    let mut go = true
    while go && (i < len(ps)) {
        let r = ps[i](input, pos)
        if r.ok => { res = r; go = false } else => { i = i + 1 }
    }
    res
}

// Zero or more of `p`, collected into a list. Stops on failure or when `p`
// succeeds without advancing (which would otherwise loop forever).
share fn many(p) = (input, pos) => {
    let mut acc = []
    let mut cur = pos
    let mut go = true
    while go {
        let r = p(input, cur)
        if r.ok && (r.pos > cur) => { acc = acc + [r.val]; cur = r.pos }
        else => { go = false }
    }
    win(acc, cur)
}

// One or more of `p`.
share fn many1(p) = (input, pos) => {
    let r = many(p)(input, pos)
    if len(r.val) == 0 => lose(pos, "at least one") else => r
}

// One or more `p` separated by `sep`; yields the list of `p` values.
share fn sep_by1(p, sep) = (input, pos) => {
    let first = p(input, pos)
    if !first.ok => first else => {
        let rest = many(keep_right(sep, p))(input, first.pos)
        win([first.val] + rest.val, rest.pos)
    }
}

// `open p close`, yielding p's value.
share fn between(open, p, close) = keep_left(keep_right(open, p), close)

// Left-associative chain: `p (op p)*`. `op` is a parser that yields a binary
// function `(acc, next) -> value`; results fold left. This is how infix
// operators of one precedence level parse without left recursion.
share fn chainl1(p, op) = (input, pos) => {
    let first = p(input, pos)
    if !first.ok => first else => {
        let rest = many(seq(op, p))(input, first.pos)
        let total = rest.val |> fold(first.val, (acc, pair) => pair[0](acc, pair[1]))
        win(total, rest.pos)
    }
}

// Run a parser over a whole input. Returns the raw result record.
share fn run(p, input) = p(input, 0)
