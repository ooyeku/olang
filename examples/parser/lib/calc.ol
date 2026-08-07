// An arithmetic-expression parser and evaluator, built from the combinators.
// It parses and computes in one pass over the grammar
//
//   expr   = term   (('+' | '-') term)*     -- left associative
//   term   = factor (('*' | '/') factor)*
//   factor = number | '(' expr ')'
//
// The three levels are mutually recursive (`factor` reaches back to `expr`
// inside parentheses), so they are written as named functions that build and
// apply their parser on each call — deferring construction breaks the cycle.

use lib.combinators {
    satisfy, is_digit, is_space, char_p, str_p, pmap, many1, many,
    keep_left, keep_right, alt, between, chainl1, run
}

// Whitespace-skipping token wrappers: a token swallows trailing spaces, so the
// grammar itself never mentions whitespace.
let spaces = many(satisfy("space", is_space))
fn tok(p) = keep_left(p, spaces)
fn sym(s) = tok(str_p(s))

// A number is a run of digits, turned into an integer value.
let number = tok(pmap(many1(satisfy("digit", is_digit)),
    (ds) => unwrap(str.parse_int(join(ds, "")))))

// Operator parsers yield the binary function to apply — parsers producing
// functions, which `chainl1` then folds with.
let addop = alt(
    pmap(sym("+"), (x) => (a, b) => a + b),
    pmap(sym("-"), (x) => (a, b) => a - b))
let mulop = alt(
    pmap(sym("*"), (x) => (a, b) => a * b),
    pmap(sym("/"), (x) => (a, b) => a / b))

// The grammar. `share` so the module re-closes them over the whole module
// scope, letting the three refer to one another regardless of order.
share fn factor(input, pos) = alt(number, between(sym("("), expr, sym(")")))(input, pos)
share fn term(input, pos) = chainl1(factor, mulop)(input, pos)
share fn expr(input, pos) = chainl1(term, addop)(input, pos)

// Parse and evaluate a whole expression. Returns { ok, value, at } where `at`
// is the failure position (or, on trailing junk, where parsing stopped).
share fn evaluate(text) = {
    let r = run(keep_right(spaces, expr), text)
    if r.ok && (r.pos == str.length(text)) => { ok: true, value: r.val, at: r.pos }
    else if !r.ok => { ok: false, value: 0, at: r.pos }
    else => { ok: false, value: 0, at: r.pos }
}
