// A backtracking matcher over the `Re` AST, in continuation-passing style.
//
// `m(re, s, pos, k)` returns the end position at which `re` — followed by the
// continuation `k` — matches starting at `pos`, or -1 if it cannot. `k` is
// itself `(pos) -> Int`, standing for "match the rest of the pattern from
// here". Sequencing threads continuations; alternation and the quantifiers
// backtrack by trying one branch and, on -1, falling back to another. The
// character guards lean on short-circuit `&&` so a bounds check protects the
// lookup that follows it.

use lib.ast {
    Re, Empty, Lit, Any, Class, Concat, Alt, Star, Plus, Opt, AtStart, AtEnd
}

// Is `ch` inside any [lo, hi] range of a character class?
fn in_class(ch, ranges) =
    ranges |> fold(false, (acc, r) => acc || ((ch >= r[0]) && (ch <= r[1])))

share fn m(re, s, pos, k) = match re {
    Empty => k(pos),
    Lit(c) =>
        if (pos < str.length(s)) && (str.char_at(s, pos) == c) => k(pos + 1) else => -1,
    Any =>
        if pos < str.length(s) => k(pos + 1) else => -1,
    Class(ranges, neg) =>
        if (pos < str.length(s)) && (in_class(str.char_at(s, pos), ranges) != neg) =>
            k(pos + 1)
        else => -1,
    Concat(a, b) => m(a, s, pos, (p) => m(b, s, p, k)),
    Alt(a, b) => {
        let r = m(a, s, pos, k)
        if r >= 0 => r else => m(b, s, pos, k)
    },
    Star(r) => mstar(r, s, pos, k),
    Plus(r) => m(r, s, pos, (p) => mstar(r, s, p, k)),
    Opt(r) => {
        let r2 = m(r, s, pos, k)
        if r2 >= 0 => r2 else => k(pos)
    },
    AtStart => if pos == 0 => k(pos) else => -1,
    AtEnd => if pos == str.length(s) => k(pos) else => -1
}

// Greedy repetition: consume one `r` (which must advance, guarding against a
// zero-width loop), recurse for more, and only then let the continuation run;
// if that whole path fails, match zero copies and hand `pos` to `k`.
share fn mstar(r, s, pos, k) = {
    let more = m(r, s, pos, (p) => if p > pos => mstar(r, s, p, k) else => -1)
    if more >= 0 => more else => k(pos)
}

// ── search API ─────────────────────────────────────────────────────────

// Leftmost match at or after `from`: { found, start, end, text }.
fn find_from(re, s, from) = {
    let n = str.length(s)
    let mut i = from
    let mut res = { found: false, start: 0, end: 0, text: "" }
    while (!res.found) && (i <= n) {
        let e = m(re, s, i, (p) => p)
        if e >= 0 => { res = { found: true, start: i, end: e, text: str.substring(s, i, e) } }
        else => { i = i + 1 }
    }
    res
}

// The leftmost match anywhere in `s`.
share fn find(re, s) = find_from(re, s, 0)

// Does `re` match anywhere in `s`?
share fn matches(re, s) = find(re, s).found

// Every non-overlapping match, left to right. A zero-width match advances one
// character so the search always makes progress.
share fn find_all(re, s) = {
    let n = str.length(s)
    let mut hits = []
    let mut from = 0
    let mut go = true
    while go && (from <= n) {
        let f = find_from(re, s, from)
        if !f.found => { go = false }
        else => {
            hits = hits + [f]
            from = if f.end > f.start => f.end else => f.start + 1
        }
    }
    hits
}
