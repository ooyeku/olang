// peg — parsing structured text from a grammar (`use peg`).
//
// A grammar is text: rules in lower case may have space between their
// parts, as `skip` says; rules whose name starts with a capital are
// tokens, matched with nothing in between. In a template string the
// grammar's backslashes are written doubled (`\\n`, `\\"`).
//
//   let Csv = peg.grammar(`
//     file   = row ("\\n" row)*
//     row    = field ("," field)*
//     field  = Quoted / Bare
//     Quoted = "\\"" ([^"] / "\\"\\"")* "\\""
//     Bare   = [^,\\n"]*
//   `)?
//   Csv.parse("a,b\n1,\"two\"")     // Ok(tree): kind, text, span, children
//
// How it is built. `peg.grammar` reads the notation into rule data,
// checks it, and analyses it (what can match nothing, which characters
// can start each rule, which rules are left-recursive, where a rule is
// tried twice at one place); every message, `check`, `examples` and
// `dump` are written here. The matching is native: the rules go as data
// to `__peg.compile` (src/stdlib/peg), which flattens them into a
// program for a recursive-descent matcher and answers a handle; a parse
// hands it the input and gets back the tree — built in Rust as the maps
// below, the actions called bottom up — or the data of the error, which
// `parse_error` makes the error value. plans/PEG-NATIVE.md has the
// design and the measurements.
//
// The reference engine (`#{ "engine": "olang" }`, for testing peg
// itself) writes each rule out as olang source instead: recursive
// descent with the character tests, loops and choices inlined, compiled
// once with `meta.eval`. Each rule has three functions: `b_` builds its
// node, `k_` only recognises (says where the match ends), and `tk_`
// recognises while noting failures. A parse runs the building ones;
// only a failed parse runs again with the noting ones, to find the
// furthest failure and what was expected there, so a successful parse
// pays nothing for good errors. The native matcher has the same three
// families and the same bookkeeping, so the two answer alike, to the
// text of an error (peg_native_test.ol checks it).
//
// The olang engine's parse state is a struct: the input as a list of
// code points with a -1 sentinel at the end (so no test needs a bounds
// check), the table of every rule's functions (the call that closes a
// cycle in the grammar goes through it), and the few cells a parse may
// need, made per parse on the parsing thread: the seeds of left
// recursion being grown, the one-entry cache of rules tried twice at a
// place, and in a noting parse the furthest failure. Positions are
// code-point offsets, as `str.substring` takes them.
//
// Its nodes are built compact during the parse (`[kind, start, end,
// kids, fields]`) and turned into maps — `kind`, `text`, `span`,
// `children` and the labels — once, at the end, where actions run too: a
// node thrown away by backtracking never costs a map or an action call.

// ── characters ─────────────────────────────────────────────────────────

// The input as code points, with -1 after the last: `cs[p]` at the end
// reads -1, which no test accepts, so no test needs `p < n`.
fn codes(text) =
    if str.is_ascii(text) => bytes.to_list(bytes.from_string(text)) + [-1]
    else => {
        let mut out = []
        for ch in str.chars(text) { out = out + [str.char_code(ch)] }
        out + [-1]
    }

// The input's text between two positions, by its bytes when it is
// ASCII (a position is then a byte offset, and the slice is native).
fn input_text(bs, cs, a, b) =
    if b <= a => ""
    else if bs == () => str.from_char_code(slice(cs, a, b))
    else => unwrap(bytes.to_string(bytes.slice(bs, a, b)))

// The text between two positions.
fn text_of(cs, a, b) = if b <= a => "" else => str.from_char_code(slice(cs, a, b))

// `col.slice` without the copy of the whole list a call out of compiled
// code makes: the parse's lists are long, and the slices short.
fn slice(xs, a, b) = {
    let mut out = []
    let mut i = a
    while i < b {
        out = out + [xs[i]]
        i = i + 1
    }
    out
}

fn is_id_start(c) = (c >= 97 && c <= 122) || (c >= 65 && c <= 90) || c == 95
fn is_id_char(c) = is_id_start(c) || (c >= 48 && c <= 57)
fn is_digit(c) = c >= 48 && c <= 57

// A string as a grammar literal would write it, for messages.
fn quote(t) = {
    let mut out = "\""
    for ch in str.chars(t) {
        let c = str.char_code(ch)
        out = out + (if ch == "\"" => "\\\""
            else if ch == "\\" => "\\\\"
            else if ch == "\n" => "\\n"
            else if ch == "\t" => "\\t"
            else if ch == "\r" => "\\r"
            else if c < 32 => "\\u{" + str.fmt("{:x}", c) + "}"
            else => ch)
    }
    out + "\""
}

// "a", "a or b", "a, b or c"
fn or_list(xs) =
    if len(xs) == 0 => ""
    else if len(xs) == 1 => xs[0]
    else => join(take(xs, len(xs) - 1), ", ") + " or " + last(xs)

fn dedupe(xs) = {
    let mut out = []
    for x in xs { if !contains(out, x) => { out = out + [x] } }
    out
}

// ── positions and errors ───────────────────────────────────────────────

// (line, column, start of the line) of a position, 1-based.
fn line_col(cs, pos) = {
    let mut line = 1
    let mut ls = 0
    let mut i = 0
    while i < pos && cs[i] >= 0 {
        if cs[i] == 10 => {
            line = line + 1
            ls = i + 1
        }
        i = i + 1
    }
    (line, pos - ls + 1, ls)
}

fn line_text(cs, ls) = {
    let mut e = ls
    while cs[e] >= 0 && cs[e] != 10 { e = e + 1 }
    if e > ls && cs[e - 1] == 13 => text_of(cs, ls, e - 1) else => text_of(cs, ls, e)
}

// The line holding `pos` and a caret under it. Tabs before the caret
// stay tabs, so the caret lines up however the terminal draws them.
fn caret_lines(cs, pos) = {
    let (line, column, ls) = line_col(cs, pos)
    let num = to_string(line)
    let gutter = str.repeat(" ", len(num))
    let mut pad = ""
    let mut i = ls
    while i < pos {
        pad = pad + (if cs[i] == 9 => "\t" else => " ")
        i = i + 1
    }
    " " + num + " | " + line_text(cs, ls) + "\n " + gutter + " | " + pad + "^"
}

// An error value: the data an application reads (`message`, `span`,
// `expected`, `found`, `after`) and `show()`, the message with the line
// and a caret.
fn error_value(cs, pos, message, expected, found, after) = {
    let (line, column, ls) = line_col(cs, pos)
    let width = if found == () => 0 else => str.length(found)
    let full = message + " (" + to_string(line) + ":" + to_string(column) + ")"
    let shown = full + "\n" + caret_lines(cs, pos)
    #{ "message": full,
       "span": #{ "line": line, "column": column, "start": pos, "end": pos + width },
       "expected": expected, "found": found, "after": after,
       "show": () => shown }
}

fn grammar_error(cs, pos, message) = Err(error_value(cs, pos, "peg.grammar: " + message, [], (), ()))

// ── reading the grammar notation ───────────────────────────────────────
//
// Expressions are maps tagged by "t":
//   lit    s, codes, ci           "abc", "abc"i
//   class  ranges, neg, src       [a-z_], [^"\\], \d \w \s inside
//   any                           .
//   ref    name, at               rule
//   seq    items                  a b c
//   alt    items                  a / b / c
//   star plus opt  e              e*  e+  e?
//   and not  e                    &e  !e
//   label  name, e                key:e
//   prec   atom, levels           precedence(atom) { left "+" "-"; prefix "-" }

// Spaces, newlines and comments (`#` or `//` to the end of the line).
fn g_ws(cs, p) = {
    let mut q = p
    loop {
        let c = cs[q]
        if c == 32 || c == 9 || c == 10 || c == 13 => { q = q + 1 }
        else if c == 35 || (c == 47 && cs[q + 1] == 47) => {
            while cs[q] >= 0 && cs[q] != 10 { q = q + 1 }
        }
        else => break
    }
    q
}

fn g_ident(cs, p) = {
    if !is_id_start(cs[p]) => -1
    else => {
        let mut q = p + 1
        while is_id_char(cs[q]) { q = q + 1 }
        q
    }
}

fn hex_val(c) =
    if c >= 48 && c <= 57 => c - 48
    else if c >= 97 && c <= 102 => c - 87
    else if c >= 65 && c <= 70 => c - 55
    else => -1

// One escape after a backslash at `p` (which points past the `\`):
// (code, end), or (-1, p) for a shorthand class letter left to the
// caller, or Err.
fn g_escape(cs, p) = {
    let c = cs[p]
    if c == 110 => Ok((10, p + 1))
    else if c == 116 => Ok((9, p + 1))
    else if c == 114 => Ok((13, p + 1))
    else if c == 48 => Ok((0, p + 1))
    else if c == 102 => Ok((12, p + 1))
    else if c == 118 => Ok((11, p + 1))
    else if c == 120 => {
        let a = hex_val(cs[p + 1])
        let b = hex_val(cs[p + 2])
        if a < 0 || b < 0 => grammar_error(cs, p - 1, "`\\x` takes two hex digits")
        else => Ok((a * 16 + b, p + 3))
    }
    else if c == 117 => {
        if cs[p + 1] == 123 => {
            let mut q = p + 2
            let mut v = 0
            while hex_val(cs[q]) >= 0 {
                v = v * 16 + hex_val(cs[q])
                q = q + 1
            }
            if cs[q] != 125 || q == p + 2 => grammar_error(cs, p - 1, "`\\u{...}` takes hex digits in braces")
            else => Ok((v, q + 1))
        }
        else => {
            let mut v = 0
            let mut q = p + 1
            while q < p + 5 && hex_val(cs[q]) >= 0 {
                v = v * 16 + hex_val(cs[q])
                q = q + 1
            }
            if q != p + 5 => grammar_error(cs, p - 1, "`\\u` takes four hex digits, or `\\u{...}`")
            else => Ok((v, q))
        }
    }
    else if c < 0 => grammar_error(cs, p - 1, "the text ends inside an escape")
    else => Ok((c, p + 1))
}

// A quoted literal at `p` ("..." or '...'), with `i` after it for one
// that ignores case: (node, end).
fn g_string(cs, p) = {
    let open = cs[p]
    let mut q = p + 1
    let mut out = []
    let mut bad = ()
    while bad == () && cs[q] != open {
        let c = cs[q]
        if c < 0 || c == 10 => { bad = "this literal is not closed on its line" }
        else if c == 92 => {
            match g_escape(cs, q + 1) {
                Ok((v, e)) => {
                    out = out + [v]
                    q = e
                },
                Err(e) => { bad = e.message }
            }
        }
        else => {
            out = out + [c]
            q = q + 1
        }
    }
    if bad != () => grammar_error(cs, p, bad)
    else => {
        let ci = cs[q + 1] == 105 && !is_id_char(cs[q + 2])
        let e = if ci => q + 2 else => q + 1
        Ok((#{ "t": "lit", "codes": out, "s": str.from_char_code(out), "ci": ci }, e))
    }
}

// The ranges a shorthand letter stands for inside or outside a class.
fn shorthand(c) =
    if c == 100 => [(48, 57)]                                         // \d
    else if c == 119 => [(48, 57), (65, 90), (95, 95), (97, 122)]     // \w
    else if c == 115 => [(9, 13), (32, 32)]                           // \s
    else => ()

// A class at `p`: [abc], [a-z], [^...], with escapes and \d \w \s.
fn g_class(cs, p) = {
    let mut q = p + 1
    let neg = cs[q] == 94
    if neg => { q = q + 1 }
    let mut ranges = []
    let mut bad = ()
    while bad == () && cs[q] != 93 {
        let c = cs[q]
        if c < 0 || c == 10 => { bad = "this class is not closed on its line" }
        else => {
            // one item: a character (maybe escaped), maybe the start of a range
            let mut lo = -1
            if c == 92 && shorthand(cs[q + 1]) != () => {
                ranges = ranges + shorthand(cs[q + 1])
                q = q + 2
            }
            else if c == 92 => {
                match g_escape(cs, q + 1) {
                    Ok((v, e)) => {
                        lo = v
                        q = e
                    },
                    Err(e) => { bad = e.message }
                }
            }
            else => {
                lo = c
                q = q + 1
            }
            if bad == () && lo >= 0 => {
                if cs[q] == 45 && cs[q + 1] != 93 && cs[q + 1] >= 0 => {
                    let mut hi = cs[q + 1]
                    q = q + 2
                    if hi == 92 => {
                        match g_escape(cs, q) {
                            Ok((v, e)) => {
                                hi = v
                                q = e
                            },
                            Err(e) => { bad = e.message }
                        }
                    }
                    if bad == () && hi < lo => { bad = "a range runs backwards: " + str.from_char_code([lo]) + "-" + str.from_char_code([hi]) }
                    ranges = ranges + [(lo, hi)]
                }
                else => { ranges = ranges + [(lo, lo)] }
            }
        }
    }
    if bad != () => grammar_error(cs, p, bad)
    else => Ok((#{ "t": "class", "ranges": ranges, "neg": neg, "src": text_of(cs, p, q + 1) }, q + 1))
}

// At `p`, does a rule start — `name =` or `name "display" =`?
fn g_rule_start(cs, p) = {
    let e = g_ident(cs, p)
    if e < 0 => false
    else => {
        let q = g_ws(cs, e)
        if cs[q] == 61 => true
        else if cs[q] == 34 || cs[q] == 39 => match g_string(cs, q) {
            Ok((_, e2)) => cs[g_ws(cs, e2)] == 61,
            Err(_) => false
        }
        else => false
    }
}

let PREC_WORDS = ["left", "right", "prefix", "postfix"]

fn g_primary(cs, p) = {
    let c = cs[p]
    if c == 40 => {
        let (e, q) = g_alt(cs, g_ws(cs, p + 1))?
        let q2 = g_ws(cs, q)
        if cs[q2] != 41 => grammar_error(cs, q2, "expected `)` to close the group")
        else => Ok((e, q2 + 1))
    }
    else if c == 34 || c == 39 => g_string(cs, p)
    else if c == 91 => g_class(cs, p)
    else if c == 46 => Ok((#{ "t": "any" }, p + 1))
    else if c == 92 && shorthand(cs[p + 1]) != () =>
        Ok((#{ "t": "class", "ranges": shorthand(cs[p + 1]), "neg": false, "src": text_of(cs, p, p + 2) }, p + 2))
    else if is_id_start(c) => {
        let e = g_ident(cs, p)
        let name = text_of(cs, p, e)
        let after = g_ws(cs, e)
        if name == "precedence" && cs[after] == 40 => g_prec(cs, after)
        else => Ok((#{ "t": "ref", "name": name, "at": p }, e))
    }
    else if c < 0 => grammar_error(cs, p, "the grammar ends where an expression should be")
    else => grammar_error(cs, p, "expected an expression, found " + quote(str.from_char_code([c])))
}

fn g_suffix(cs, p) = {
    let (e, q) = g_primary(cs, p)?
    let c = cs[q]
    if c == 42 => Ok((#{ "t": "star", "e": e }, q + 1))
    else if c == 43 => Ok((#{ "t": "plus", "e": e }, q + 1))
    else if c == 63 => Ok((#{ "t": "opt", "e": e }, q + 1))
    else => Ok((e, q))
}

fn g_prefix(cs, p) = {
    let c = cs[p]
    if c == 38 => {
        let (e, q) = g_prefix(cs, g_ws(cs, p + 1))?
        Ok((#{ "t": "and", "e": e }, q))
    }
    else if c == 33 => {
        let (e, q) = g_prefix(cs, g_ws(cs, p + 1))?
        Ok((#{ "t": "not", "e": e }, q))
    }
    else => {
        // a label: `name:` before an expression
        let e = g_ident(cs, p)
        let colon = if e < 0 => -1 else => g_ws(cs, e)
        if e > 0 && cs[colon] == 58 => {
            let name = text_of(cs, p, e)
            let (inner, q) = g_prefix(cs, g_ws(cs, colon + 1))?
            Ok((#{ "t": "label", "name": name, "e": inner, "at": p }, q))
        }
        else => g_suffix(cs, p)
    }
}

// Where a sequence ends: `/`, `)`, `}`, `;`, the end, or the next rule.
fn g_seq_ends(cs, p) = {
    let c = cs[p]
    c < 0 || c == 47 || c == 41 || c == 125 || c == 59 || g_rule_start(cs, p)
}

fn g_seq(cs, p) = {
    let mut items = []
    let mut q = p
    let mut bad = ()
    while bad == () && !g_seq_ends(cs, q) {
        match g_prefix(cs, q) {
            Ok((e, q2)) => {
                items = items + [e]
                q = g_ws(cs, q2)
            },
            Err(e) => { bad = e }
        }
    }
    if bad != () => Err(bad)
    else if len(items) == 0 => grammar_error(cs, q, "expected an expression")
    else if len(items) == 1 => Ok((items[0], q))
    else => Ok((#{ "t": "seq", "items": items }, q))
}

fn g_alt(cs, p) = {
    let (first, q0) = g_seq(cs, p)?
    let mut items = [first]
    let mut q = q0
    let mut bad = ()
    while bad == () && cs[q] == 47 && cs[q + 1] != 47 {
        match g_seq(cs, g_ws(cs, q + 1)) {
            Ok((e, q2)) => {
                items = items + [e]
                q = g_ws(cs, q2)
            },
            Err(e) => { bad = e }
        }
    }
    if bad != () => Err(bad)
    else if len(items) == 1 => Ok((first, q))
    else => Ok((#{ "t": "alt", "items": items }, q))
}

// `precedence(atom) { left "+" "-"; right "^"; prefix "-"; postfix "!" }`,
// loosest level first. `p` is at the `(`.
fn g_prec(cs, p) = {
    let (atom, q0) = g_alt(cs, g_ws(cs, p + 1))?
    let q1 = g_ws(cs, q0)
    if cs[q1] != 41 => grammar_error(cs, q1, "expected `)` after precedence's atom")
    else => {
        let q2 = g_ws(cs, q1 + 1)
        if cs[q2] != 123 => grammar_error(cs, q2, "expected `{` and the operator levels after precedence(...)")
        else => {
            let mut q = g_ws(cs, q2 + 1)
            let mut levels = []
            let mut bad = ()
            while bad == () && cs[q] != 125 {
                if cs[q] == 59 => { q = g_ws(cs, q + 1) }
                else => {
                    let e = g_ident(cs, q)
                    let word = if e < 0 => "" else => text_of(cs, q, e)
                    if !contains(PREC_WORDS, word) => {
                        let why = if cs[q] < 0 => "precedence's levels are not closed with `}`"
                            else => "expected left, right, prefix or postfix to start a level"
                        bad = error_value(cs, q, "peg.grammar: " + why, [], (), ())
                    }
                    else => {
                        q = g_ws(cs, e)
                        let mut ops = []
                        while bad == () && cs[q] >= 0 && cs[q] != 59 && cs[q] != 125 && !prec_word_at(cs, q) {
                            match g_primary(cs, q) {
                                Ok((op, q3)) => {
                                    ops = ops + [op]
                                    q = g_ws(cs, q3)
                                },
                                Err(err) => { bad = err }
                            }
                        }
                        if bad == () && len(ops) == 0 => {
                            bad = error_value(cs, q, "peg.grammar: the level `" + word + "` names no operator", [], (), ())
                        }
                        levels = levels + [#{ "assoc": word, "ops": ops }]
                    }
                }
            }
            if bad != () => Err(bad)
            else if len(levels) == 0 => grammar_error(cs, q, "precedence(...) { } needs at least one level")
            else => Ok((#{ "t": "prec", "atom": atom, "levels": levels }, q + 1))
        }
    }
}

fn prec_word_at(cs, p) = {
    let e = g_ident(cs, p)
    e > 0 && contains(PREC_WORDS, text_of(cs, p, e)) && cs[g_ws(cs, e)] != 58
}

// A token is a rule whose name, past any leading `_`, starts with a
// capital; a silent rule (`_name`) makes no node of its own.
fn is_token_name(name) = {
    let mut i = 0
    let cs = str.chars(name)
    while i < len(cs) && cs[i] == "_" { i = i + 1 }
    i < len(cs) && cs[i] >= "A" && cs[i] <= "Z"
}

// The rules of a grammar text: Ok([#{ name, desc, e, token, silent, at }]).
fn read_rules(text) = {
    let cs = codes(text)
    let mut q = g_ws(cs, 0)
    let mut rules = []
    let mut bad = ()
    while bad == () && cs[q] >= 0 {
        let e = g_ident(cs, q)
        if e < 0 => { bad = error_value(cs, q, "peg.grammar: expected a rule name", [], (), ()) }
        else => {
            let name = text_of(cs, q, e)
            let at = q
            let mut r = g_ws(cs, e)
            let mut desc = ()
            if cs[r] == 34 || cs[r] == 39 => {
                match g_string(cs, r) {
                    Ok((lit, e2)) => {
                        desc = lit.s
                        r = g_ws(cs, e2)
                    },
                    Err(err) => { bad = err }
                }
            }
            if bad == () && cs[r] != 61 => {
                bad = error_value(cs, r, "peg.grammar: expected `=` after the rule name `" + name + "`", [], (), ())
            }
            if bad == () => {
                match g_alt(cs, g_ws(cs, r + 1)) {
                    Ok((body, q2)) => {
                        q = g_ws(cs, q2)
                        if cs[q] >= 0 && !g_rule_start(cs, q) => {
                            bad = error_value(cs, q, "peg.grammar: expected the next rule, found " + quote(str.from_char_code([cs[q]])), [], (), ())
                        }
                        rules = rules + [#{ "name": name, "desc": desc, "e": body, "at": at,
                            "token": is_token_name(name), "silent": str.starts_with(name, "_") }]
                    },
                    Err(err) => { bad = err }
                }
            }
        }
    }
    if bad != () => Err(bad)
    else if len(rules) == 0 => grammar_error(cs, 0, "the grammar has no rules")
    else => Ok((rules, cs))
}

// ── analysis ───────────────────────────────────────────────────────────

// The expressions directly inside one.
fn parts_of(e) = {
    let t = e.t
    if t == "seq" || t == "alt" => e.items
    else if t == "star" || t == "plus" || t == "opt" || t == "and" || t == "not" || t == "label" => [e.e]
    else if t == "prec" => [e.atom] + flatten(map(e.levels, (l) => l.ops))
    else => []
}

// Every expression in `e`, itself included.
fn all_exprs(e) = [e] + flatten(map(parts_of(e), all_exprs))

// The rule names `e` refers to.
fn refs_in(e) = map(filter(all_exprs(e), (x) => x.t == "ref"), (x) => x.name)

// Can `e` succeed without consuming anything? `nul` maps rule → Bool.
fn nullable(e, nul) = {
    let t = e.t
    if t == "lit" => len(e.codes) == 0
    else if t == "class" || t == "any" => false
    else if t == "ref" => nul[e.name] ?? false
    else if t == "seq" => col.all(e.items, (x) => nullable(x, nul))
    else if t == "alt" => col.any(e.items, (x) => nullable(x, nul))
    else if t == "star" || t == "opt" || t == "and" || t == "not" => true
    else if t == "plus" || t == "label" => nullable(e.e, nul)
    else if t == "prec" => nullable(e.atom, nul)
    else => false
}

fn nullables(rules) = {
    let mut nul = #{}
    for r in rules { nul = map_set(nul, r.name, false) }
    let mut changed = true
    while changed {
        changed = false
        for r in rules {
            if !nul[r.name] && nullable(r.e, nul) => {
                nul = map_set(nul, r.name, true)
                changed = true
            }
        }
    }
    nul
}

// The characters an expression can start with: #{ "a": 128 Bools for
// ASCII, "w": whether a character past ASCII can start it }.
fn fs_empty() = #{ "a": col.filled(128, false), "w": false }
fn fs_all() = #{ "a": col.filled(128, true), "w": true }
fn fs_union(x, y) = #{ "a": map(zip(x.a, y.a), ((p, q)) => p || q), "w": x.w || y.w }

fn fs_ranges(ranges, neg) = {
    let mut a = col.filled(128, neg)
    let mut w = neg
    for (lo, hi) in ranges {
        let mut c = lo
        while c <= hi && c < 128 {
            a = col.set(a, c, !neg)
            c = c + 1
        }
        if hi >= 128 && !neg => { w = true }
    }
    #{ "a": a, "w": w }
}

fn lower_code(c) = if c >= 65 && c <= 90 => c + 32 else => c
fn upper_code(c) = if c >= 97 && c <= 122 => c - 32 else => c
fn is_letter(c) = (c >= 65 && c <= 90) || (c >= 97 && c <= 122)

fn first_of(e, fst, nul) = {
    let t = e.t
    if t == "lit" => {
        if len(e.codes) == 0 => fs_empty()
        else => {
            let c = e.codes[0]
            if e.ci && is_letter(c) => fs_ranges([(lower_code(c), lower_code(c)), (upper_code(c), upper_code(c))], false)
            else => fs_ranges([(c, c)], false)
        }
    }
    else if t == "class" => fs_ranges(e.ranges, e.neg)
    else if t == "any" => fs_all()
    else if t == "ref" => fst[e.name] ?? fs_empty()
    else if t == "seq" => {
        let mut acc = fs_empty()
        let mut going = true
        for x in e.items {
            if going => {
                acc = fs_union(acc, first_of(x, fst, nul))
                if !nullable(x, nul) => { going = false }
            }
        }
        acc
    }
    else if t == "alt" => fold(e.items, fs_empty(), (acc, x) => fs_union(acc, first_of(x, fst, nul)))
    else if t == "star" || t == "plus" || t == "opt" || t == "label" => first_of(e.e, fst, nul)
    else if t == "prec" => {
        let pre = flatten(map(filter(e.levels, (l) => l.assoc == "prefix"), (l) => l.ops))
        fold(pre, first_of(e.atom, fst, nul), (acc, x) => fs_union(acc, first_of(x, fst, nul)))
    }
    else => fs_empty()
}

fn firsts(rules, nul) = {
    let mut fst = #{}
    for r in rules { fst = map_set(fst, r.name, fs_empty()) }
    let mut changed = true
    while changed {
        changed = false
        for r in rules {
            let f = first_of(r.e, fst, nul)
            if f != fst[r.name] => {
                fst = map_set(fst, r.name, f)
                changed = true
            }
        }
    }
    fst
}

// The rules `e` may call before consuming anything.
fn left_calls(e, nul) = {
    let t = e.t
    if t == "ref" => [e.name]
    else if t == "seq" => {
        let mut acc = []
        let mut going = true
        for x in e.items {
            if going => {
                acc = acc + left_calls(x, nul)
                if !nullable(x, nul) => { going = false }
            }
        }
        acc
    }
    else if t == "alt" => flatten(map(e.items, (x) => left_calls(x, nul)))
    else if t == "star" || t == "plus" || t == "opt" || t == "label" || t == "and" || t == "not" => left_calls(e.e, nul)
    else if t == "prec" => {
        let pre = flatten(map(filter(e.levels, (l) => l.assoc == "prefix"), (l) => l.ops))
        left_calls(e.atom, nul) + flatten(map(pre, (x) => left_calls(x, nul)))
    }
    else => []
}

// rule → the rules it may call at its own start position.
fn left_graph(rules, nul) =
    fold(rules, #{}, (m, r) => map_set(m, r.name, dedupe(left_calls(r.e, nul))))

// Every rule reachable from `from` along `graph`, not entering `avoid`.
fn reach(graph, from, avoid) = {
    let mut seen = []
    let mut todo = graph[from] ?? []
    while len(todo) > 0 {
        let x = todo[0]
        todo = skip(todo, 1)
        if !contains(seen, x) && !contains(avoid, x) => {
            seen = seen + [x]
            todo = todo + (graph[x] ?? [])
        }
    }
    seen
}

// Left recursion: #{ "rec": [rules on a cycle], "heads": [rules that grow
// a seed], "scc": rule → the heads of its cycle }. The heads are a
// feedback set — every cycle passes through one — so the rest of the
// cycle runs plainly and only the heads memoise.
fn left_recursion(rules, graph) = {
    let names = map(rules, (r) => r.name)
    let rec = filter(names, (n) => contains(reach(graph, n, []), n))
    let mut heads = []
    for n in rec {
        if contains(reach(graph, n, heads), n) => { heads = heads + [n] }
    }
    // the heads that share a strongly connected part
    let mut scc = #{}
    for h in heads {
        let mates = filter(heads, (o) => o != h && contains(reach(graph, h, []), o) && contains(reach(graph, o, []), h))
        scc = map_set(scc, h, mates)
    }
    #{ "rec": rec, "heads": heads, "scc": scc }
}

// Rules tried more than once at one position: those an ordered choice
// can reach from two of its alternatives before either consumes
// anything. Tokens are cheaper to scan again than to memoise, and a
// rule on a left-recursive cycle must not memoise (its answer changes
// while the cycle's seed grows).
fn memo_rules(rules, nul, graph, lr) = {
    let byname = fold(rules, #{}, (m, r) => map_set(m, r.name, r))
    let mut found = []
    for r in rules {
        for e in all_exprs(r.e) {
            if e.t == "alt" => {
                let reached = map(e.items, (it) => {
                    let direct = dedupe(left_calls(it, nul))
                    dedupe(direct + flatten(map(direct, (d) => reach(graph, d, []))))
                })
                let mut i = 0
                while i < len(reached) {
                    let mut j = i + 1
                    while j < len(reached) {
                        for x in reached[i] {
                            if contains(reached[j], x) && !contains(found, x) => { found = found + [x] }
                        }
                        j = j + 1
                    }
                    i = i + 1
                }
            }
        }
    }
    filter(found, (x) => {
        let r = byname[x]
        r != () && !r.token && !contains(lr.rec, x)
    })
}

// The labels a rule (or a synthetic label node) owns: name → (count,
// repeated). A label on a rule — `key:Key`, `items:item*`, `val:_value?`
// — names the nodes that rule gathers; a label on anything else makes a
// node of its own, whose inner labels are its own. Labels in a silent
// rule belong to the rule that uses it.
fn label_wraps(e, byname) = {
    let inner = if e.t == "opt" || e.t == "star" || e.t == "plus" => e.e else => e
    inner.t == "ref" && byname[inner.name] != ()
}

fn label_scan(e, rep, byname, seen) = {
    let t = e.t
    if t == "label" => {
        let many = rep || ((e.e.t == "star" || e.e.t == "plus") && label_wraps(e.e, byname))
        [(e.name, many)]
    }
    else if t == "star" || t == "plus" => label_scan(e.e, true, byname, seen)
    else if t == "ref" => {
        let r = byname[e.name]
        if r != () && r.silent && !r.token && !contains(seen, e.name) => label_scan(r.e, rep, byname, seen + [e.name])
        else => []
    }
    else if t == "prec" || t == "and" || t == "not" => []
    else => flatten(map(parts_of(e), (x) => label_scan(x, rep, byname, seen)))
}

// The labels of a scope that hold lists: repeated, or written twice.
fn list_labels(e, byname) = {
    let found = label_scan(e, false, byname, [])
    let names = dedupe(map(found, (x) => x[0]))
    filter(names, (n) => {
        let hits = filter(found, (x) => x[0] == n)
        len(hits) > 1 || col.any(hits, (x) => x[1])
    })
}

fn has_labels(e, byname) = len(label_scan(e, false, byname, [])) > 0

// ── code generation ────────────────────────────────────────────────────
//
// Each expression becomes an olang expression that answers the position
// after its match, or -1. Inside a function `s` is the parse state, `cs`
// its code points, and the expression's start position is a variable
// name. In a rule written in lower case every element first skips what
// `skip` matches (unless the position is already past it: a rule is
// entered skipped, by its caller or the driver).
//
// A rule has three functions: `b_` builds (answers its node, or ()),
// `k_` recognises (answers the end, or -1), and `tk_` recognises while
// noting each failure, for the error of a parse that failed. A building
// function gathers its children in two locals, `kd` (a list) and `nk`
// (how much of it is in use): a child is written at `nk`, and backing
// out of a failed alternative is `nk = mark`, which frees nothing and
// allocates nothing. Nothing on the way is shared or mutable, so the
// functions stay on the bytecode tier.
//
// The context `cx`:
//   build   gather nodes (else only recognise)
//   ws      skip between elements (a lower-case rule)
//   track   note failures for the error message
//   tok     in a tracking token: the variable holding its start, where
//           its own failures are not noted ("" elsewhere)
//   inner   inside a token: rule calls recognise without tracking

// The runtime the generated functions share, written into the same
// source above them, so every call to it is a direct one.
let RUNTIME = "
fn pg_slice(xs, a, b) = {
    let mut out = []
    let mut i = a
    while i < b {
        out = out + [xs[i]]
        i = i + 1
    }
    out
}
fn pg_unlabel(kids, lists) = {
    let mut out = []
    let mut f = #{}
    for nm in lists { f = map_set(f, nm, []) }
    for z in kids {
        if typeof(z) == \"Tuple\" => {
            let nm = z[0]
            let i = len(out)
            out = out + [z[1]]
            let cur = f[nm]
            if typeof(cur) == \"List\" => { f = map_set(f, nm, cur + [i]) } else => { f = map_set(f, nm, i) }
        }
        else => { out = out + [z] }
    }
    (out, f)
}
fn pg_lnode(kind, p, r, raw, lists) = {
    let (kids, f) = pg_unlabel(raw, lists)
    [kind, p, r, kids, f]
}
fn pg_grp(kd, a, b, kind, p) =
    if b - a == 1 => kd[a]
    else => {
        let kids = pg_slice(kd, a, b)
        let e = if len(kids) == 0 => p else => kids[len(kids) - 1][2]
        [kind, p, e, kids, ()]
    }
fn pg_seed_find(s, slot, p) = {
    let xs = cell.get(s.seeds)
    let mut i = len(xs) - 1
    let mut out = ()
    while i >= 0 {
        let e = xs[i]
        if e[0] == slot && e[1] == p => {
            out = e
            i = -1
        }
        else => { i = i - 1 }
    }
    out
}
fn pg_seed_set(s, slot, p, v) = {
    let xs = cell.take(s.seeds)
    let mut out = []
    for e in xs { if !(e[0] == slot && e[1] == p) => { out = out + [e] } }
    cell.set(s.seeds, out + [[slot, p, v]])
}
fn pg_seed_drop(s, slot, p) = {
    let xs = cell.take(s.seeds)
    let mut out = []
    for e in xs { if !(e[0] == slot && e[1] == p) => { out = out + [e] } }
    cell.set(s.seeds, out)
}
fn pg_cget(s, k) = cell.get(s.cache)[k]
fn pg_cput(s, k, v) = {
    let c = s.cache
    let mut xs = cell.take(c)
    xs[k] = v
    cell.set(c, xs)
}
fn pg_lc(c) = if c >= 65 && c <= 90 => c + 32 else => c
fn pg_text_of(cs, a, b) = if b <= a => \"\" else => str.from_char_code(pg_slice(cs, a, b))
fn pg_fail(s, x, i) = {
    let c = s.err
    let mut e = cell.take(c)
    let far = e[0]
    if x > far => { e = [x, [i]] }
    else if x == far && !contains(e[1], i) => { e = [far, e[1] + [i]] }
    cell.set(c, e)
    -1
}
fn pg_fail_in(s, x, i, t) = if x > t => pg_fail(s, x, i) else => -1
fn pg_note(s, x, i) = {
    pg_fail(s, x, i)
    x
}
fn pg_note_in(s, x, i, t) = {
    pg_fail_in(s, x, i, t)
    x
}
fn pg_mute_end(s, saved, p, r, i, tok) = {
    let c = s.err
    let e = cell.get(c)
    if r < 0 => {
        if e[0] == p => {
            let before = if saved[0] == p => saved[1] else => []
            cell.set(c, [p, if contains(before, i) => before else => before + [i]])
        }
        else if e[0] < p => { pg_fail(s, p, i) }
    }
    else if tok || e[0] == p => { cell.set(c, saved) }
    r
}
fn pg_okk(s, a, b) = {
    let c = s.oks
    let mut o = cell.take(c)
    if o[b] < 0 => { o[b] = a }
    cell.set(c, o)
    b
}
"

fn fresh(g, pre) = {
    let n = cell.get(g.ctr)
    cell.set(g.ctr, n + 1)
    pre + to_string(n)
}

// The index of an expectation's words in the grammar's table.
fn desc_idx(g, d) = {
    let ds = cell.get(g.descs)
    match col.index_of(ds, d) {
        () => {
            cell.set(g.descs, ds + [d])
            len(ds)
        },
        i => i
    }
}

fn emit(g, src) = cell.set(g.extra, cell.get(g.extra) + [src])

// Lines as one expression: a block, or the line itself.
fn blk(lines) =
    if len(lines) == 1 && !str.contains(lines[0], "let ") => (if str.starts_with(lines[0], "if ") => "(" + lines[0] + ")" else => lines[0])
    else => "{ " + join(lines, "; ") + " }"

fn with_cx(cx, k, v) = map_set(cx, k, v)

// Sorted, merged ranges.
fn merge_ranges(ranges) = {
    let sorted = sort(ranges)
    let mut out = []
    for (lo, hi) in sorted {
        if len(out) > 0 && lo <= out[len(out) - 1][1] + 1 => {
            let (plo, phi) = out[len(out) - 1]
            out = col.set(out, len(out) - 1, (plo, if hi > phi => hi else => phi))
        }
        else => { out = out + [(lo, hi)] }
    }
    out
}

fn ranges_test(ranges, cv) = {
    let parts = map(merge_ranges(ranges), ((lo, hi)) =>
        if lo == hi => cv + " == " + to_string(lo)
        else => "(" + cv + " >= " + to_string(lo) + " && " + cv + " <= " + to_string(hi) + ")")
    if len(parts) == 0 => "false" else => "(" + join(parts, " || ") + ")"
}

fn class_test(e, cv) =
    if e.neg => "(" + cv + " >= 0 && !" + ranges_test(e.ranges, cv) + ")"
    else => ranges_test(e.ranges, cv)

// A first-character set as a test, or "" when it lets everything through.
fn first_test(f, cv) = {
    if f.w && col.all(f.a, (b) => b) => ""
    else => {
        let mut ranges = []
        let mut c = 0
        while c < 128 {
            if f.a[c] => {
                let lo = c
                while c < 128 && f.a[c] { c = c + 1 }
                ranges = ranges + [(lo, c - 1)]
            }
            else => { c = c + 1 }
        }
        let wide = if f.w => [(128, 1114111)] else => []
        ranges_test(ranges + wide, cv)
    }
}

fn lit_test(e, x) = join(map(enumerate(e.codes), ((i, c)) => {
    let at = if i == 0 => "cs[" + x + "]" else => "cs[" + x + " + " + to_string(i) + "]"
    if e.ci && is_letter(c) => "pg_lc(" + at + ") == " + to_string(lower_code(c))
    else => at + " == " + to_string(c)
}), " && ")

fn skip_code(g, p) =
    if g.skipclass != () => {
        let v = fresh(g, "w")
        let c = fresh(g, "c")
        "{ let mut " + v + " = " + p + "; loop { let " + c + " = cs[" + v + "]; if " + class_test(g.skipclass, c) +
            " => { " + v + " = " + v + " + 1 } else => break }; " + v + " }"
    }
    else => fnm(g, "sk", "") + "(s, " + p + ")"

// (lines to put first, the variable holding the position after skip).
fn at_pos(g, cx, p, skd) =
    if cx.ws && !skd && g.skip != () => {
        let x = fresh(g, "x")
        (["let " + x + " = " + skip_code(g, p)], x)
    }
    else => ([], p)

fn fail_code(g, cx, x, desc) =
    if !cx.track => "-1"
    else if cx.tok != "" => "pg_fail_in(s, " + x + ", " + to_string(desc_idx(g, desc)) + ", " + cx.tok + ")"
    else => "pg_fail(s, " + x + ", " + to_string(desc_idx(g, desc)) + ")"

fn note_code(g, cx, x, desc) =
    if !cx.track => x
    else if cx.tok != "" => "pg_note_in(s, " + x + ", " + to_string(desc_idx(g, desc)) + ", " + cx.tok + ")"
    else => "pg_note(s, " + x + ", " + to_string(desc_idx(g, desc)) + ")"

fn ok_code(cx, x, r) = if cx.track && !cx.inner => "pg_okk(s, " + x + ", " + r + ")" else => r

// A generated function's name: its family (b, k, tk; sk for skip),
// the rule, and the grammar's own suffix. Names are unique to a grammar
// so two grammars' `b_value`s never meet in the bytecode tier's books.
fn fnm(g, fam, rule) = fam + "_" + rule + g.uid

// The family a reference calls in this context.
fn fn_fam(cx) =
    if cx.build => "b"
    else if cx.track && !cx.inner => "tk"
    else => "k"

let FAMILIES = ["b", "k", "tk"]

// A call to a rule's function: direct when that function is already
// written above (or is the one being written), else through the state's
// table. A nested function sees the siblings written before it, so a
// call to one written later — a cycle in the grammar — goes through
// `s.t`, which holds every rule's three functions.
fn call_code(g, fam, rule, x) = {
    let fname = fnm(g, fam, rule)
    if fname == cell.get(g.cur) || map_has_key(cell.get(g.declared), fname) => fname + "(s, " + x + ")"
    else => "s.t[" + to_string(col.index_of(FAMILIES, fam) * len(g.rules) + g.index[rule]) + "](s, " + x + ")"
}

fn declare(g, fname) = cell.set(g.declared, map_set(cell.get(g.declared), fname, true))

// Code that appends the node in variable `v` to the children.
fn push_code(v) = "{ if nk < len(kd) => { kd[nk] = " + v + " } else => { kd = kd + [" + v + "] }; nk = nk + 1 }"

// Code that appends the children of the silent match in `v`.
fn splice_code(v) = "{ for z in " + v + "[3] " + push_code("z") + " }"

// Does `e` gather nodes when built?
fn produces(g, e) = {
    let t = e.t
    if t == "lit" || t == "class" || t == "any" || t == "and" || t == "not" => false
    else if t == "ref" => {
        let r = g.byname[e.name]
        if !r.silent => true
        else if r.token => false
        else if contains(cell.get(g.seen), e.name) => true
        else => {
            cell.set(g.seen, cell.get(g.seen) + [e.name])
            let out = produces(g, r.e)
            cell.set(g.seen, filter(cell.get(g.seen), (x) => x != e.name))
            out
        }
    }
    else if t == "label" || t == "prec" => true
    else => col.any(parts_of(e), (x) => produces(g, x))
}

fn cg(g, e, p, cx, skd) = {
    let t = e.t
    if t == "lit" => cg_lit(g, e, p, cx, skd)
    else if t == "class" => cg_class(g, e, p, cx, skd)
    else if t == "any" => {
        let (pre, x) = at_pos(g, cx, p, skd)
        blk(pre + ["if cs[" + x + "] >= 0 => " + ok_code(cx, x, x + " + 1") + " else => " + fail_code(g, cx, x, "any character")])
    }
    else if t == "ref" => cg_ref(g, e, p, cx, skd)
    else if t == "seq" => cg_seq(g, e.items, p, cx, skd)
    else if t == "alt" => cg_alt(g, e.items, p, cx, skd)
    else if t == "star" => cg_star(g, e.e, p, cx)
    else if t == "plus" => cg_plus(g, e.e, p, cx, skd)
    else if t == "opt" => {
        let r = fresh(g, "r")
        "{ let " + r + " = " + cg(g, e.e, p, cx, skd) + "; if " + r + " >= 0 => " + r + " else => " + p + " }"
    }
    else if t == "and" || t == "not" => cg_pred(g, e, p, cx, skd)
    else if t == "label" => cg_label(g, e, p, cx, skd)
    else if t == "prec" => {
        let (pre, x) = at_pos(g, cx, p, skd)
        let f = emit_prec(g, e, cx, cell.get(g.rule))
        if cx.build => {
            let n = fresh(g, "n")
            blk(pre + ["let " + n + " = " + f + "(s, " + x + ", 0)", "if " + n + " == () => -1 else => { " + push_code(n) + "; " + n + "[2] }"])
        }
        else => blk(pre + [f + "(s, " + x + ", 0)"])
    }
    else => raise("peg: no code for " + t)
}

fn cg_ref(g, e, p, cx, skd) = {
    let (pre, x) = at_pos(g, cx, p, skd)
    let r = g.byname[e.name]
    if !cx.build => blk(pre + [call_code(g, fn_fam(cx), e.name, x)])
    else if r.token && r.silent => blk(pre + [call_code(g, "k", e.name, x)])
    else if r.token => {
        // a token's leaf is made here, from where it ends
        let v = fresh(g, "v")
        let n = fresh(g, "n")
        blk(pre + ["let " + v + " = " + call_code(g, "k", e.name, x),
            "if " + v + " < 0 => -1 else => { let " + n + " = [" + quoted_name(e.name) + ", " + x + ", " + v + ", [], ()]; " + push_code(n) + "; " + v + " }"])
    }
    else => {
        let n = fresh(g, "n")
        let add = if r.silent => splice_code(n) else => push_code(n)
        blk(pre + ["let " + n + " = " + call_code(g, "b", e.name, x),
            "if " + n + " == () => -1 else => { " + add + "; " + n + "[2] }"])
    }
}

fn cg_lit(g, e, p, cx, skd) = {
    let k = len(e.codes)
    if k == 0 => p
    else => {
        let (pre, x) = at_pos(g, cx, p, skd)
        let desc = quote(e.s)
        blk(pre + ["if " + lit_test(e, x) + " => " + ok_code(cx, x, x + " + " + to_string(k)) + " else => " + fail_code(g, cx, x, desc)])
    }
}

fn cg_class(g, e, p, cx, skd) = {
    let (pre, x) = at_pos(g, cx, p, skd)
    let c = fresh(g, "c")
    blk(pre + ["let " + c + " = cs[" + x + "]",
        "if " + class_test(e, c) + " => " + ok_code(cx, x, x + " + 1") + " else => " + fail_code(g, cx, x, e.src)])
}

// A sequence. When a later element fails, the children the earlier ones
// gathered are dropped: `nk` goes back to where the sequence began.
fn cg_seq(g, items, p, cx, skd) = {
    let n = len(items)
    let needs_mark = cx.build && col.any(take(items, n - 1), (x) => produces(g, x))
    let m = if needs_mark => fresh(g, "m") else => ""
    let body = seq_from(g, items, 0, p, cx, skd, m, false)
    if needs_mark => "{ let " + m + " = nk; " + body + " }" else => body
}

fn seq_from(g, items, i, p, cx, skd, m, made) = {
    let code = cg(g, items[i], p, cx, skd)
    let failed = if made => "{ nk = " + m + "; -1 }" else => "-1"
    if i == len(items) - 1 => {
        if made => {
            let v = fresh(g, "v")
            "{ let " + v + " = " + code + "; if " + v + " < 0 => " + failed + " else => " + v + " }"
        }
        else => code
    }
    else => {
        let v = fresh(g, "v")
        let rest = seq_from(g, items, i + 1, v, cx, false, m, made || (cx.build && produces(g, items[i])))
        "{ let " + v + " = " + code + "; if " + v + " < 0 => " + failed + " else => " + rest + " }"
    }
}

// An ordered choice. When no alternative can match nothing, the choice
// skips once and looks at the next character: an alternative that
// cannot start with it is not tried at all. (A tracking parse tries
// them all, since their failures are what the error says.)
fn cg_alt(g, items, p, cx, skd) = {
    let dispatch = !cx.track && !col.any(items, (x) => nullable(x, g.nul))
    if dispatch => {
        let (pre, x) = at_pos(g, cx, p, skd)
        let c = fresh(g, "c")
        // a lone class or literal tests its own first character
        let tests = map(items, (it) => if it.t == "class" || it.t == "lit" => "" else => first_test(first_of(it, g.fst, g.nul), c))
        blk(pre + ["let " + c + " = cs[" + x + "]", alt_from(g, items, tests, 0, x, cx, true)])
    }
    else => alt_from(g, items, map(items, (it) => ""), 0, p, cx, skd)
}

fn alt_from(g, items, tests, i, p, cx, skd) = {
    let code = cg(g, items[i], p, cx, skd)
    let guarded = if tests[i] == "" => code else => "(if " + tests[i] + " => " + code + " else => -1)"
    if i == len(items) - 1 => guarded
    else => {
        let r = fresh(g, "r")
        "{ let " + r + " = " + guarded + "; if " + r + " >= 0 => " + r + " else => " + alt_from(g, items, tests, i + 1, p, cx, skd) + " }"
    }
}

// The test a run loop can inline: a class, `.` or one character,
// where nothing is skipped between repetitions.
fn run_test(g, e, cx, cv) =
    if cx.ws => ()
    else if e.t == "class" => class_test(e, cv)
    else if e.t == "any" => cv + " >= 0"
    else if e.t == "lit" && len(e.codes) == 1 && !e.ci => cv + " == " + to_string(e.codes[0])
    else => ()

fn run_desc(e) = if e.t == "class" => e.src else if e.t == "any" => "any character" else => quote(e.s)

fn cg_star(g, e, p, cx) = {
    let v = fresh(g, "q")
    let c = fresh(g, "c")
    let test = run_test(g, e, cx, c)
    let lead = if e.t == "alt" && len(e.items) > 1 => run_test(g, e.items[0], cx, c) else => ()
    if test == () && lead != () && !cx.track => {
        // (A / B / ...)* with A one character: run over A's characters
        // in place, and try the others only where A stops
        let r = fresh(g, "r")
        let rest = if len(e.items) == 2 => e.items[1] else => #{ "t": "alt", "items": skip(e.items, 1) }
        "{ let mut " + v + " = " + p + "; loop { let " + c + " = cs[" + v + "]; if " + lead + " => { " + v + " = " + v + " + 1 } else => { let " + r +
            " = " + cg(g, rest, v, cx, false) + "; if " + r + " <= " + v + " => break; " + v + " = " + r + " } }; " + v + " }"
    }
    else if test != () =>
        "{ let mut " + v + " = " + p + "; loop { let " + c + " = cs[" + v + "]; if " + test + " => { " + v + " = " + v +
            " + 1 } else => break }; " + note_code(g, cx, v, run_desc(e)) + " }"
    else => {
        let r = fresh(g, "r")
        "{ let mut " + v + " = " + p + "; loop { let " + r + " = " + cg(g, e, v, cx, false) + "; if " + r + " <= " + v +
            " => break; " + v + " = " + r + " }; " + v + " }"
    }
}

fn cg_plus(g, e, p, cx, skd) = {
    let f = fresh(g, "f")
    "{ let " + f + " = " + cg(g, e, p, cx, skd) + "; if " + f + " < 0 => -1 else => " + cg_star(g, e, f, cx) + " }"
}

fn cg_pred(g, e, p, cx, skd) = {
    let icx = with_cx(with_cx(cx, "build", false), "track", false)
    let r = fresh(g, "r")
    let inner = cg(g, e.e, p, icx, skd)
    if e.t == "and" => "{ let " + r + " = " + inner + "; if " + r + " >= 0 => " + p + " else => -1 }"
    else => {
        // `!.` failing is the end of input that was expected
        let (pre, x) = at_pos(g, cx, p, skd)
        let failed = if cx.track && e.e.t == "any" => blk(pre + [fail_code(g, with_cx(cx, "tok", ""), x, "end of input")]) else => "-1"
        "{ let " + r + " = " + inner + "; if " + r + " >= 0 => " + failed + " else => " + p + " }"
    }
}

// A label names the nodes its expression gathered (`key:Key`), or makes
// one node of what it matched (`op:("+" / "-")`) and names that.
fn cg_label(g, e, p, cx, skd) = {
    if !cx.build => cg(g, e.e, p, cx, skd)
    else => {
        let (pre, x) = at_pos(g, cx, p, skd)
        let m = fresh(g, "m")
        let r = fresh(g, "r")
        let i = fresh(g, "i")
        let inner = cg(g, e.e, x, cx, true)
        let name = quoted_name(e.name)
        let done = if label_wraps(e.e, g.byname) =>
                "{ let mut " + i + " = " + m + "; while " + i + " < nk { let z = kd[" + i + "]; if typeof(z) != \"Tuple\" => { kd[" + i + "] = (" +
                    name + ", z) }; " + i + " = " + i + " + 1 }; " + r + " }"
            else => {
                // (an optional part that matched nothing makes no node)
                let n = fresh(g, "n")
                "{ if " + r + " > " + x + " || nk > " + m + " => { let " + n + " = (" + name + ", pg_lnode(" + name + ", " + x + ", " + r + ", pg_slice(kd, " + m + ", nk), " + show(list_labels(e.e, g.byname)) +
                    ")); nk = " + m + "; " + push_code(n) + " }; " + r + " }"
            }
        blk(pre + ["let " + m + " = nk", "let " + r + " = " + inner, "if " + r + " >= 0 => " + done + " else => -1"])
    }
}

// The operators of a precedence table, longest literal first so `<=`
// is tried before `<`: [(fixity, level, expr)].
fn prec_ops(e) = {
    let mut ops = []
    let mut lev = 0
    for l in e.levels {
        for op in l.ops { ops = ops + [(l.assoc, lev, op)] }
        lev = lev + 1
    }
    col.sort_by(ops, (o) => if o[2].t == "lit" => 0 - len(o[2].codes) else => 0)
}

// A precedence table as a function `(s, p, min_level)`: an operand (a
// prefix operator and its operand, or the atom), then, while an
// operator at `min_level` or tighter follows, a postfix operator, or an
// infix one and its right side — one level tighter for `left`, the same
// level for `right`. Building, it answers the node: the operand alone,
// or an operator's node with `op`, `fixity` and `left`/`right` or
// `operand`.
fn emit_prec(g, e, cx, kind) = {
    let name = fresh(g, if cx.track => "tp" else => "p") + (if cx.build => "b" else => "k") + g.uid
    let ocx = with_cx(cx, "build", false)
    let ops = prec_ops(e)
    let k = quoted_name(kind)
    let sk = (v) => if cx.ws && g.skip != () => skip_code(g, v) else => v
    let mut lines = ["fn " + name + "(s, p, ml) = {", "    let cs = s.cs"]
    if cx.build => { lines = lines + ["    let mut kd = []", "    let mut nk = 0"] }
    lines = lines + ["    let mut q = -1", "    let mut pl = -1", "    let mut pe = -1"]
    for (fix, lev, op) in ops {
        if fix == "prefix" => {
            let r = fresh(g, "r")
            lines = lines + ["    if pl < 0 => { let " + r + " = " + cg(g, op, "p", ocx, true) + "; if " + r + " >= 0 => { pl = " +
                to_string(lev) + "; pe = " + r + " } }"]
        }
    }
    if cx.build => {
        lines = lines + ["    if pl >= 0 => { let o = " + name + "(s, " + sk("pe") + ", pl); if o != () => { let nn = [" + k +
                ", p, o[2], [o], #{ \"op\": pg_text_of(cs, p, pe), \"fixity\": \"prefix\", \"operand\": 0 }]; " + push_code("nn") + "; q = o[2] } }",
            "    else => { q = " + cg(g, e.atom, "p", cx, true) + " }"]
    }
    else => {
        lines = lines + ["    if pl >= 0 => { q = " + name + "(s, " + sk("pe") + ", pl) }",
            "    else => { q = " + cg(g, e.atom, "p", cx, true) + " }"]
    }
    lines = lines + ["    if q >= 0 => { loop {", "        let x = " + sk("q")]
    for (fix, lev, op) in ops {
        if fix == "postfix" => {
            let r = fresh(g, "r")
            let made = if cx.build => "let a = pg_grp(kd, 0, nk, " + k + ", p); nk = 0; let nn = [" + k + ", p, " + r + ", [a], #{ \"op\": pg_text_of(cs, x, " + r +
                    "), \"fixity\": \"postfix\", \"operand\": 0 }]; " + push_code("nn") + "; "
                else => ""
            lines = lines + ["        if ml <= " + to_string(lev) + " => { let " + r + " = " + cg(g, op, "x", ocx, true) + "; if " + r +
                " >= 0 => { " + made + "q = " + r + "; continue } }"]
        }
    }
    for (fix, lev, op) in ops {
        if fix == "left" || fix == "right" => {
            let r = fresh(g, "r")
            let next = if fix == "left" => to_string(lev + 1) else => to_string(lev)
            let tail = if cx.build =>
                    "let o = " + name + "(s, " + sk(r) + ", " + next + "); if o != () => { let a = pg_grp(kd, 0, nk, " + k + ", p); nk = 0; let nn = [" + k +
                        ", p, o[2], [a, o], #{ \"op\": pg_text_of(cs, x, " + r + "), \"fixity\": \"infix\", \"left\": 0, \"right\": 1 }]; " + push_code("nn") +
                        "; q = o[2]; continue }"
                else => "let o = " + name + "(s, " + sk(r) + ", " + next + "); if o >= 0 => { q = o; continue }"
            lines = lines + ["        if ml <= " + to_string(lev) + " => { let " + r + " = " + cg(g, op, "x", ocx, true) + "; if " + r +
                " >= 0 => { " + tail + " } }"]
        }
    }
    lines = lines + ["        break", "    } }"]
    lines = lines + [if cx.build => "    if q < 0 => () else => pg_grp(kd, 0, nk, " + k + ", p)" else => "    q", "}"]
    emit(g, join(lines, "\n"))
    name
}

// ── rules as functions ─────────────────────────────────────────────────

fn quoted_name(name) = "\"" + name + "\""

// A rule tried twice at one position keeps its last answer: one entry a
// rule, which is the case that matters — an alternative that matched
// the rule and then failed, and the next one, which starts with it.
fn cache_fn(fname, inner, slot, build) = {
    let miss = if build => "let v = " + inner + "(s, p); pg_cput(s, " + to_string(slot) + ", [p, v]); v"
        else => "let v = " + inner + "(s, p); pg_cput(s, " + to_string(slot) + ", [p, v]); v"
    "fn " + fname + "(s, p) = { let e = pg_cget(s, " + to_string(slot) + "); if e != () && e[0] == p => e[1] else => { " + miss + " } }"
}

// A left-recursive rule grows a seed (Warth et al.): it fails at first,
// then runs its body again and again, each time letting the recursive
// call see the last answer, while the match grows. The seeds of the
// growths under way live in `s.seeds`; another head of the same cycle,
// met inside, grows its own from scratch each round, so it always sees
// the current seed.
fn lr_fn(fname, inner, slot, build) = {
    let sl = to_string(slot)
    if build => join(["fn " + fname + "(s, p) = {",
        "    let e = pg_seed_find(s, " + sl + ", p)",
        "    if e != () => e[2]",
        "    else => {",
        "        pg_seed_set(s, " + sl + ", p, ())",
        "        let mut best = ()",
        "        let mut bend = -1",
        "        loop {",
        "            let nd = " + inner + "(s, p)",
        "            if nd == () => break",
        "            if nd[2] <= bend => break",
        "            best = nd",
        "            bend = nd[2]",
        "            pg_seed_set(s, " + sl + ", p, nd)",
        "        }",
        "        pg_seed_drop(s, " + sl + ", p)",
        "        best",
        "    }",
        "}"], "\n")
    else => join(["fn " + fname + "(s, p) = {",
        "    let e = pg_seed_find(s, " + sl + ", p)",
        "    if e != () => e[2]",
        "    else => {",
        "        pg_seed_set(s, " + sl + ", p, -1)",
        "        let mut best = -1",
        "        loop {",
        "            let r = " + inner + "(s, p)",
        "            if r <= best => break",
        "            best = r",
        "            pg_seed_set(s, " + sl + ", p, r)",
        "        }",
        "        pg_seed_drop(s, " + sl + ", p)",
        "        best",
        "    }",
        "}"], "\n")
}

// One rule function, wrapped when the rule memoises: its text, with the
// precedence functions its body needed written just before it.
fn rule_fn(g, fname, slot, build, body_of) = {
    let name = cell.get(g.rule)
    let inner = if slot < 0 => fname else => fname + "__"
    cell.set(g.cur, inner)
    cell.set(g.extra, [])
    let body = body_of()
    let pre = cell.get(g.extra)
    let text = if slot < 0 => "fn " + fname + "(s, p) = " + body
        else => {
            let wrapper = if contains(g.lr.heads, name) => lr_fn(fname, inner, slot, build) else => cache_fn(fname, inner, slot, build)
            "fn " + inner + "(s, p) = " + body + "\n" + wrapper
        }
    declare(g, inner)
    declare(g, fname)
    join(pre + [text], "\n")
}

// A building body: gather the children, then make the node (or, for a
// silent rule, a node without a kind, whose children its caller takes).
fn build_body(g, r, code) = {
    let gathers = produces(g, r.e)
    let kids = if gathers => "(if nk == len(kd) => kd else => pg_slice(kd, 0, nk))" else => "[]"
    let kind = if r.silent => "()" else => quoted_name(r.name)
    let made = if !r.silent && has_labels(r.e, g.byname) => "pg_lnode(" + kind + ", p, r, " + kids + ", " + show(list_labels(r.e, g.byname)) + ")"
        else => "[" + kind + ", p, r, " + kids + ", ()]"
    let locals = if gathers => "let mut kd = []; let mut nk = 0; " else => ""
    "{ let cs = s.cs; " + locals + "let r = " + code + "; if r < 0 => () else => " + made + " }"
}

fn emit_rule(g, r, family) = {
    let name = r.name
    cell.set(g.rule, name)
    let fname = fnm(g, family, name)
    let build = family == "b"
    let track = family == "tk"
    let slot = match g.slots[name] { () => -1, i => i * 2 + (if build => 0 else => 1) }
    let d = r.desc ?? name
    if name == "skip" => {
        if family == "k" => {
            let cx = #{ "build": false, "ws": false, "track": false, "tok": "", "inner": true }
            let ks = fnm(g, "k", "skip")
            let k = rule_fn(g, ks, -1, false, () => "{ let cs = s.cs; " + cg(g, r.e, "p", cx, true) + " }")
            let sk = fnm(g, "sk", "")
            declare(g, sk)
            [k, "fn " + sk + "(s, p) = { let mut q = p; loop { let r = " + ks + "(s, q); if r <= q => break; q = r }; q }"]
        }
        else => {
            declare(g, fname)
            let ks = fnm(g, "k", "skip")
            if build => ["fn " + fname + "(s, p) = { let r = " + ks + "(s, p); if r < 0 => () else => [(), p, r, [], ()] }"]
            else => ["fn " + fname + "(s, p) = " + ks + "(s, p)"]
        }
    }
    else if r.token => {
        if build => {
            // a token's node is a leaf: what it matched, no children
            declare(g, fname)
            let kind = if r.silent => "()" else => quoted_name(name)
            ["fn " + fname + "(s, p) = { let r = " + fnm(g, "k", name) + "(s, p); if r < 0 => () else => [" + kind + ", p, r, [], ()] }"]
        }
        else => {
            let cx = #{ "build": false, "ws": false, "track": track, "tok": if track => "p" else => "", "inner": true }
            [rule_fn(g, fname, slot, false, () => {
                let body = cg(g, r.e, "p", cx, true)
                if track => "{ let cs = s.cs; let sv = cell.get(s.err); let r = pg_mute_end(s, sv, p, " + body + ", " + to_string(desc_idx(g, d)) +
                    ", true); if r < 0 => -1 else => pg_okk(s, p, r) }"
                else => "{ let cs = s.cs; " + body + " }"
            })]
        }
    }
    else => {
        let cx = #{ "build": build, "ws": true, "track": track, "tok": "", "inner": false }
        [rule_fn(g, fname, slot, build, () => {
            if build => {
                if r.e.t == "prec" && !r.silent => emit_prec(g, r.e, cx, name) + "(s, p, 0)"
                else => build_body(g, r, cg(g, r.e, "p", cx, true))
            }
            else if r.e.t == "prec" => {
                let call = emit_prec(g, r.e, cx, name) + "(s, p, 0)"
                if track && r.desc != () => "pg_mute_end(s, cell.get(s.err), p, " + call + ", " + to_string(desc_idx(g, d)) + ", false)" else => call
            }
            else => {
                // a described rule (`value "a value" = ...`), tracking,
                // says its description where it fails at its start, in
                // place of what its insides expected there
                let code = cg(g, r.e, "p", cx, true)
                if track && r.desc != () => "{ let cs = s.cs; pg_mute_end(s, cell.get(s.err), p, " + code + ", " + to_string(desc_idx(g, d)) + ", false) }"
                else => "{ let cs = s.cs; " + code + " }"
            }
        })]
    }
}

// The rules in an order that writes a rule after the ones it calls,
// where the grammar allows: callees first (depth-first, post-order), so
// most calls are direct and only a cycle's closing call is not.
fn rule_order(rules, byname) = {
    let mut order = []
    let mut seen = []
    let mut stack = []
    let starts = map(filter(rules, (r) => r.name == "skip"), (r) => r.name) + map(rules, (r) => r.name)
    for root in starts {
        if !contains(seen, root) => {
            seen = seen + [root]
            stack = [(root, 0)]
            while len(stack) > 0 {
                let (n, i) = stack[len(stack) - 1]
                let kids = dedupe(refs_in(byname[n].e))
                if i < len(kids) => {
                    stack = col.set(stack, len(stack) - 1, (n, i + 1))
                    let k = kids[i]
                    if !contains(seen, k) => {
                        seen = seen + [k]
                        stack = stack + [(k, 0)]
                    }
                }
                else => {
                    order = order + [n]
                    stack = take(stack, len(stack) - 1)
                }
            }
        }
    }
    order
}

// The whole parser as olang source: the runtime, then each rule's
// functions in call order (recognisers first, since builders call them),
// and the table of every rule's functions — b, k, tk, by rule index —
// which is also `s.t`.
fn generate(g) = {
    let rules = g.rules
    let order = rule_order(rules, g.byname)
    let mut fns = []
    for fam in ["k", "tk", "b"] {
        for name in order { fns = fns + emit_rule(g, g.byname[name], fam) }
    }
    let list = (fam) => join(map(rules, (r) => fnm(g, fam, r.name)), ", ")
    let table = "#{ \"t\": [" + list("b") + ", " + list("k") + ", " + list("tk") + "]" +
        ", \"sk\": " + (if g.skip == () => "()" else => fnm(g, "sk", "")) + ", \"fail\": pg_fail }"
    let maker = "make_parser" + g.uid
    join(["fn " + maker + "() = {", RUNTIME] + fns + [table, "}", maker + "()", ""], "\n")
}

// ── building a grammar ─────────────────────────────────────────────────

fn edit_distance(a, b) = {
    let x = str.chars(a)
    let y = str.chars(b)
    let mut prev = range(0, len(y) + 1)
    for i in 0..len(x) {
        let mut cur = [i + 1]
        for j in 0..len(y) {
            let cost = if x[i] == y[j] => 0 else => 1
            let best = min([prev[j + 1] + 1, cur[j] + 1, prev[j] + cost])
            cur = cur + [best]
        }
        prev = cur
    }
    prev[len(y)]
}

fn did_you_mean(name, names) = {
    let close = filter(names, (n) => {
        let d = edit_distance(str.to_lower(n), str.to_lower(name))
        d <= 2 && d < str.length(name)
    })
    if len(close) == 0 => "" else => "; did you mean `" + col.min_by(close, (n) => edit_distance(n, name)) + "`?"
}

let RESERVED = ["kind", "text", "span", "children"]

// Names, references, labels and actions that hold together, or the
// first that does not.
fn validate(rules, cs, actions, start) = {
    let names = map(rules, (r) => r.name)
    let byname = fold(rules, #{}, (m, r) => map_set(m, r.name, r))
    let mut seen = []
    let mut bad = ()
    for r in rules {
        if bad == () && contains(seen, r.name) => { bad = error_value(cs, r.at, "peg.grammar: the rule `" + r.name + "` is defined twice", [], (), ()) }
        seen = seen + [r.name]
        for e in all_exprs(r.e) {
            if bad == () && e.t == "ref" && byname[e.name] == () => {
                bad = error_value(cs, e.at, "peg.grammar: no rule is named `" + e.name + "`" + did_you_mean(e.name, names), [], (), ())
            }
            if bad == () && e.t == "label" && contains(RESERVED, e.name) => {
                bad = error_value(cs, e.at, "peg.grammar: a label cannot be named `" + e.name + "`: every node has its own " + e.name, [], (), ())
            }
            if bad == () && e.t == "label" && (r.token || r.name == "skip") => {
                bad = error_value(cs, e.at, "peg.grammar: `" + r.name + "` is a token, matched as one piece of text; a label inside it names nothing", [], (), ())
            }
        }
    }
    let labels = dedupe(flatten(map(rules, (r) => map(filter(all_exprs(r.e), (e) => e.t == "label"), (e) => e.name))))
    if bad == () => {
        for k in map_keys(actions) {
            if bad == () && !contains(names, k) && !contains(labels, k) => {
                bad = error_value(cs, 0, "peg.grammar: an action for `" + k + "`, which is no rule or label" + did_you_mean(k, names + labels), [], (), ())
            }
        }
    }
    if bad == () && start != () && byname[start] == () => {
        bad = error_value(cs, 0, "peg.grammar: the start rule `" + start + "` is not defined" + did_you_mean(start, names), [], (), ())
    }
    if bad == () && byname["skip"] != () && byname["skip"].token => {
        bad = error_value(cs, 0, "peg.grammar: `skip` is special and cannot be a token", [], (), ())
    }
    if bad != () => Err(bad) else => Ok(byname)
}

// A token small enough to write in place: characters only, no rule.
fn simple_token(r) = {
    let xs = all_exprs(r.e)
    r.token && !col.any(xs, (x) => x.t == "ref" || x.t == "label" || x.t == "prec") && len(xs) <= 16
}

fn inline_tokens(e, byname, depth) = {
    let t = e.t
    if t == "ref" => {
        let r = byname[e.name]
        if depth < 4 && simple_token(r) => inline_tokens(r.e, byname, depth + 1) else => e
    }
    else if t == "seq" || t == "alt" => map_set(e, "items", map(e.items, (x) => inline_tokens(x, byname, depth)))
    else if t == "star" || t == "plus" || t == "opt" || t == "and" || t == "not" || t == "label" => map_set(e, "e", inline_tokens(e.e, byname, depth))
    else => e
}

// A grammar from its text: Ok(grammar) or Err(error), the error with
// `show()` as a parse error has. `actions` maps a rule (or label) name to
// a function from its node to a value; `options`: `start` (the rule a
// parse begins with; the first rule otherwise), `recover` (sync strings:
// a failed parse skips past the next and goes on), `memo` ("auto",
// "all" or "none") and, for testing peg itself, `engine`: "native" (the
// default) or "olang", the reference engine that writes the grammar out
// as olang functions — slower, and kept to check the native one against.
share fn grammar(text, actions = #{}, options = #{}) = {
    let (rules, cs) = read_rules(text)?
    let byname = validate(rules, cs, actions, options.start)?
    let engine = options.engine ?? "native"
    if engine != "native" && engine != "olang" => raise("peg.grammar: the engine is \"native\" or \"olang\", not " + quote(to_string(engine)))
    let nul = nullables(rules)
    let fst = firsts(rules, nul)
    let graph = left_graph(rules, nul)
    let lr = left_recursion(rules, graph)
    let mode = options.memo ?? "auto"
    let memo = if mode == "none" => lr.heads
        else if mode == "all" => dedupe(lr.heads + map(filter(rules, (r) => !r.token && !contains(lr.rec, r.name) && r.name != "skip"), (r) => r.name))
        else => dedupe(lr.heads + memo_rules(rules, nul, graph, lr))
    // Inside tokens (and skip), a reference to a token made only of
    // characters is written in place, so a run like `Wc+` is one loop.
    let crules = map(rules, (r) => if r.token || r.name == "skip" => map_set(r, "e", inline_tokens(r.e, byname, 0)) else => r)
    let names = map(rules, (r) => r.name)
    let start = options.start ?? head(filter(names, (n) => n != "skip") + ["skip"])
    let base = #{ "rules": rules, "names": names, "byname": byname, "idx": fold(enumerate(names), #{}, (m, (i, n)) => map_set(m, n, i)),
        "start": start, "recover": options.recover, "actions": actions,
        "nul": nul, "fst": fst, "lr": lr, "graph": graph, "memo": memo, "text": text }
    if engine == "olang" => Ok(grammar_value(olang_engine(base, crules, nul, fst, lr, memo, byname)))
    else => {
        // The native matcher (src/stdlib/peg): the rules as data, the
        // ones that memoise in slot order, the heads of left recursion
        // and the actions; it answers a handle the parses run on.
        let native = __peg.compile(crules, memo, lr.heads, actions)
        Ok(grammar_value(#{ ...base, "native": native, "source": __peg.listing(native) }))
    }
}

// The reference engine: the grammar written out as olang source and
// compiled once with `meta.eval`.
fn olang_engine(base, crules, nul, fst, lr, memo, byname) = {
    let slots = fold(enumerate(memo), #{}, (m, (i, n)) => map_set(m, n, i))
    let sk = byname["skip"]
    let skipclass = if sk == () => ()
        else if (sk.e.t == "plus" || sk.e.t == "star") && sk.e.e.t == "class" => sk.e.e
        else => ()
    let cbyname = fold(crules, #{}, (m, r) => map_set(m, r.name, r))
    let g = #{ "rules": crules, "byname": cbyname, "nul": nul, "fst": fst, "lr": lr, "slots": slots,
        "skip": if sk == () => () else => sk.e, "skipclass": skipclass,
        "index": fold(enumerate(base.rules), #{}, (m, (i, r)) => map_set(m, r.name, i)),
        "ctr": cell.new(0), "extra": cell.new([]), "descs": cell.new([]), "rule": cell.new(""), "seen": cell.new([]),
        "cur": cell.new(""), "declared": cell.new(#{}), "uid": "_" + crypto.random_hex(4) }
    let end_idx = desc_idx(g, "end of input")
    let src = generate(g)
    let fns = match meta.eval(src) {
        Ok(f) => f,
        Err(e) => raise("peg: the parser generated for this grammar does not compile (a bug in peg): " + e)
    }
    #{ ...base, "native": (), "fns": fns, "descs": cell.get(g.descs), "end_idx": end_idx, "nslots": len(memo) * 2, "source": src }
}

// The grammar as a value: its methods close over the compiled parser.
// (Lambdas without defaults, so they compile: a parse is then compiled
// code calling compiled code, and its tree never crosses into the
// tree-walker and back.)
fn grammar_value(P) =
    #{ "parse": (input) => run_parse(P, input, #{}),
       "parse_with": (input, opts) => run_parse(P, input, opts),
       "accepts": (input) => accepts(P, input, #{}),
       "check": () => check_rules(P),
       "examples": (n, seed) => examples_of(P, n, seed),
       "rules": P.names, "start": P.start, "source": P.source, "$peg": P }

// ── parsing ────────────────────────────────────────────────────────────

// The parse state. The cells are made per parse, on the parsing thread;
// those a parse does not need are ().
type PegState = struct { cs: List, t: List, seeds: Any, cache: Any, err: Any, oks: Any }

fn new_state(P, cs, track) =
    PegState { cs: cs, t: P.fns.t,
        seeds: if len(P.lr.heads) == 0 => () else => cell.new([]),
        cache: if P.nslots == 0 => () else => cell.new(col.filled(P.nslots, ())),
        err: if track => cell.new([-1, []]) else => (),
        oks: if track => cell.new(col.filled(len(cs), -1)) else => () }

fn skip_at(P, s, p) = if P.fns.sk == () => p else => P.fns.sk(s, p)

// The start rule's node from `p` (skip before it), or ().
fn build_from(P, s, i, p) = P.fns.t[i](s, skip_at(P, s, p))

// The end of the start rule from `p`, noting failures, skip after it;
// or -1.
fn track_from(P, s, i, p) = {
    let r = P.fns.t[2 * len(P.names) + i](s, skip_at(P, s, p))
    if r < 0 => -1 else => skip_at(P, s, r)
}

fn plain_node(x) = if typeof(x) == "Tuple" => x[1] else => x

// The tree's root: the start rule's node, or one made for it when the
// start rule is silent (its node has no kind).
fn root_of(nd, name) = if nd[0] == () => [name, nd[1], nd[2], map(nd[3], plain_node), ()] else => nd

fn is_word_code(c) = is_id_char(c) || c >= 128

// What stands at a position, for "found": a word, or one character.
fn found_at(cs, p) = {
    if cs[p] < 0 => ()
    else if is_word_code(cs[p]) => {
        let mut q = p
        while is_word_code(cs[q]) && q - p < 24 { q = q + 1 }
        text_of(cs, p, q)
    }
    else => text_of(cs, p, p + 1)
}

// The last piece matched before a position, for "after".
fn after_at(cs, oks, p) = {
    let mut b = p
    while b > 0 && (cs[b - 1] == 32 || cs[b - 1] == 9 || cs[b - 1] == 10 || cs[b - 1] == 13) { b = b - 1 }
    if b <= 0 || b >= len(oks) || oks[b] < 0 => ()
    else => {
        let a = oks[b]
        if b - a > 24 => () else => text_of(cs, a, b)
    }
}

fn error_from(cs, far, expected, oks) = {
    let found = found_at(cs, far)
    let after = after_at(cs, oks, far)
    let saw = if found == () => "end of input" else => quote(found)
    let message = if len(expected) == 0 => "unexpected " + saw
        else => "expected " + or_list(expected) + (if after == () => "" else => " after " + quote(after)) + ", found " + saw
    error_value(cs, far, message, expected, found, after)
}

// Parse again from `p` with the tracking functions, which note every
// failure, and say the furthest one.
fn failure(P, cs, i, p) = {
    let n = len(cs) - 1
    let s = new_state(P, cs, true)
    let r = track_from(P, s, i, p)
    if r >= 0 && r < n => { P.fns.fail(s, r, P.end_idx) }
    let e = cell.get(s.err)
    let far = if e[0] >= 0 => e[0] else if r >= 0 => r else => p
    error_from(cs, far, dedupe(map(e[1], (k) => P.descs[k])), cell.get(s.oks))
}

fn run_parse(P, input, opts) = {
    let start = opts.start ?? P.start
    let i = P.idx[start]
    if i == () => raise("peg: no rule is named `" + start + "`" + did_you_mean(start, P.names))
    let recover = opts.recover ?? P.recover
    if P.native != () => {
        if recover != () => native_recovering(P, input, i, recover)
        else => match __peg.parse(P.native, input, i) {
            Ok(v) => Ok(v),
            Err(d) => Err(parse_error(d))
        }
    }
    else => {
        let cs = codes(input)
        let n = len(cs) - 1
        if recover != () => parse_recovering(P, input, cs, i, start, recover)
        else => {
            let s = new_state(P, cs, false)
            let nd = build_from(P, s, i, 0)
            if nd != () && skip_at(P, s, nd[2]) == n => Ok(finish(input, cs, root_of(nd, start), P.actions))
            else => Err(failure(P, cs, i, 0))
        }
    }
}

// An error value from the native matcher's data: the same fields, and
// `show()`.
fn parse_error(d) = {
    let shown = d.shown
    #{ "message": d.message, "span": d.span, "expected": d.expected, "found": d.found, "after": d.after,
       "show": () => shown }
}

fn native_recovering(P, input, i, sync) = {
    let syncs = if typeof(sync) == "List" => sync else => [sync]
    match __peg.recover(P.native, input, i, syncs) {
        Ok(tree) => Ok(tree),
        Err(got) => {
            let errors = map(got[1], parse_error)
            let shown = join(map(errors, (e) => e.show()), "\n")
            Err(#{ ...errors[0], "errors": errors, "tree": got[0], "show": () => shown })
        }
    }
}

// Whether the whole input matches, without building a tree.
fn accepts(P, input, opts) = {
    let start = opts.start ?? P.start
    let i = P.idx[start]
    if i == () => raise("peg: no rule is named `" + start + "`" + did_you_mean(start, P.names))
    if P.native != () => __peg.accepts(P.native, input, i)
    else => {
        let cs = codes(input)
        let s = new_state(P, cs, false)
        let r = P.fns.t[len(P.names) + i](s, skip_at(P, s, 0))
        r >= 0 && skip_at(P, s, r) == len(cs) - 1
    }
}

fn starts_at(cs, q, sy) = {
    let mut i = 0
    while i < len(sy) && cs[q + i] == sy[i] { i = i + 1 }
    i == len(sy)
}

// Where to go on after an error: past the first sync string at or
// after it, or -1.
fn sync_after(cs, from, syncs) = {
    let n = len(cs) - 1
    let mut q = from
    let mut found = -1
    while found < 0 && q < n {
        for sy in syncs {
            if found < 0 && len(sy) > 0 && starts_at(cs, q, sy) => { found = q + len(sy) }
        }
        q = q + 1
    }
    found
}

// Parse, and at each error skip past the next sync point and go on:
// every error in one pass, and a tree of what did parse.
fn parse_recovering(P, input, cs, i, start, sync) = {
    let n = len(cs) - 1
    let syncs = map(if typeof(sync) == "List" => sync else => [sync], (t) => take(codes(t), str.length(t)))
    let mut p = 0
    let mut kids = []
    let mut errors = []
    let mut going = true
    while going {
        let s = new_state(P, cs, false)
        let nd = build_from(P, s, i, p)
        let r = if nd == () => -1 else => skip_at(P, s, nd[2])
        if nd != () => { kids = kids + map(nd[3], plain_node) }
        if r == n => { going = false }
        else => {
            let e = failure(P, cs, i, p)
            errors = errors + [e]
            let at = sync_after(cs, e.span.start, syncs)
            if at < 0 || at >= n => { going = false }
            else => { p = if at <= p => p + 1 else => at }
        }
    }
    let tree = finish(input, cs, [start, 0, n, kids, ()], P.actions)
    if len(errors) == 0 => Ok(tree)
    else => {
        let shown = join(map(errors, (e) => e.show()), "\n")
        Err(#{ ...errors[0], "errors": errors, "tree": tree, "show": () => shown })
    }
}

// ── trees ──────────────────────────────────────────────────────────────

// Line and column of `a`, moving a cursor forward: the nodes are visited
// in order of their start, so the whole walk reads the input once.
fn advance(cs, cur, a) = {
    let c = cell.take(cur)
    let mut q = c[0]
    let mut line = c[1]
    let mut ls = c[2]
    if a < q => {
        q = 0
        line = 1
        ls = 0
    }
    while q < a {
        if cs[q] == 10 => {
            line = line + 1
            ls = q + 1
        }
        q = q + 1
    }
    cell.set(cur, [q, line, ls])
    (line, a - ls + 1)
}

fn conv(bs, cs, cur, nd, actions) = {
    let kind = nd[0]
    let a = nd[1]
    let b = nd[2]
    let (line, column) = advance(cs, cur, a)
    let mut kids = []
    for k in nd[3] { kids = kids + [conv(bs, cs, cur, if typeof(k) == "Tuple" => k[1] else => k, actions)] }
    let mut m = #{ "kind": kind, "text": input_text(bs, cs, a, b),
        "span": #{ "line": line, "column": column, "start": a, "end": b }, "children": kids }
    let f = nd[4]
    if f != () => {
        for (k, v) in entries(f) {
            let tv = typeof(v)
            m = map_set(m, k, if tv == "Int" => kids[v] else if tv == "List" => map(v, (j) => kids[j]) else => v)
        }
    }
    let act = actions[kind]
    if act == () => m else => act(m)
}

fn finish(input, cs, root, actions) = {
    let bs = if str.is_ascii(input) => bytes.from_string(input) else => ()
    conv(bs, cs, cell.new([0, 1, 0]), root, actions)
}

// Parse `input` with a grammar: `g.parse(input)`, or with options
// (`start`, `recover`) `g.parse_with(input, opts)`.
share fn parse(g, input, opts = #{}) = g.parse_with(input, opts)

// A tree as indented text, one node a line: its kind, the labels that
// name it, its text when it has no children, and the fields that are
// not nodes (a precedence node's `op`). For tests and for looking.
share fn dump(tree) = join(dump_lines(tree, "", ""), "\n")

fn is_node(v) = typeof(v) != "List" && typeof(v) != "String" && v != () && map_has_key(v, "kind") && map_has_key(v, "span")

fn dump_lines(nd, indent, label) = {
    if !is_node(nd) => [indent + label + show(nd)]
    else => {
        let extra = filter(map_keys(nd), (k) => !contains(RESERVED, k))
        let plain = filter(extra, (k) => typeof(nd[k]) == "String")
        let named = (kid) => filter(extra, (k) => {
            let v = nd[k]
            if typeof(v) == "List" => col.any(v, (x) => is_node(x) && x.span == kid.span && x.kind == kid.kind)
            else => is_node(v) && v.span == kid.span && v.kind == kid.kind
        })
        let notes = join(map(plain, (k) => " " + k + "=" + quote(nd[k])), "")
        let head = indent + label + nd.kind + notes + (if len(nd.children) == 0 => " " + quote(nd.text) else => "")
        let mut out = [head]
        for kid in nd.children {
            let names = if is_node(kid) => named(kid) else => []
            let lab = if len(names) == 0 => "" else => join(names, ",") + ": "
            out = out + dump_lines(kid, indent + "  ", lab)
        }
        out
    }
}

// ── checking a grammar ─────────────────────────────────────────────────

// An expression in the grammar notation, for messages.
fn show_expr(e) = {
    let t = e.t
    let wrap = (x) => if contains(["seq", "alt", "label", "star", "plus", "opt", "and", "not"], x.t) => "(" + show_expr(x) + ")" else => show_expr(x)
    if t == "lit" => quote(e.s) + (if e.ci => "i" else => "")
    else if t == "class" => e.src
    else if t == "any" => "."
    else if t == "ref" => e.name
    else if t == "seq" => join(map(e.items, (x) => if x.t == "alt" => "(" + show_expr(x) + ")" else => show_expr(x)), " ")
    else if t == "alt" => join(map(e.items, show_expr), " / ")
    else if t == "star" => wrap(e.e) + "*"
    else if t == "plus" => wrap(e.e) + "+"
    else if t == "opt" => wrap(e.e) + "?"
    else if t == "and" => "&" + wrap(e.e)
    else if t == "not" => "!" + wrap(e.e)
    else if t == "label" => e.name + ":" + wrap(e.e)
    else if t == "prec" => "precedence(" + show_expr(e.atom) + ") { ... }"
    else => t
}

// Does `e` always succeed (so nothing after it in a choice is tried)?
fn always(e, byname, depth) = {
    let t = e.t
    if t == "star" || t == "opt" => true
    else if t == "lit" => len(e.codes) == 0
    else if t == "seq" => col.all(e.items, (x) => always(x, byname, depth))
    else if t == "alt" => col.any(e.items, (x) => always(x, byname, depth))
    else if t == "label" => always(e.e, byname, depth)
    else if t == "ref" && depth < 8 => always(byname[e.name].e, byname, depth + 1)
    else => false
}

// The literal an expression starts with, when it must.
fn leading_lit(e) =
    if e.t == "lit" => e
    else if e.t == "seq" => leading_lit(e.items[0])
    else if e.t == "label" => leading_lit(e.e)
    else => ()

fn class_has(e, c) = {
    let inside = col.any(e.ranges, ((lo, hi)) => c >= lo && c <= hi)
    if e.neg => !inside else => inside
}

// Why the later alternative `b` can never match after `a`, or ().
fn shadow_reason(a, b, byname) = {
    let lb = leading_lit(b)
    if always(a, byname, 0) => "`" + show_expr(a) + "` before it always matches"
    else if show_expr(a) == show_expr(b) => "it is the same as an alternative before it"
    else if a.t == "lit" && !a.ci && lb != () && !lb.ci && len(lb.codes) >= len(a.codes) && take(lb.codes, len(a.codes)) == a.codes =>
        "`" + show_expr(a) + "` before it matches its start first"
    else if a.t == "class" && lb != () && len(lb.codes) > 0 && !lb.ci && class_has(a, lb.codes[0]) && b.t == "lit" && len(lb.codes) == 1 =>
        "`" + show_expr(a) + "` before it matches that character first"
    else => ()
}

// The rule names reachable from `from` through any reference.
fn reachable(byname, from) = {
    let mut seen = [from]
    let mut todo = [from]
    while len(todo) > 0 {
        let x = todo[0]
        todo = skip(todo, 1)
        let r = byname[x]
        if r != () => {
            for n in dedupe(refs_in(r.e)) {
                if !contains(seen, n) => {
                    seen = seen + [n]
                    todo = todo + [n]
                }
            }
        }
    }
    seen
}

fn finding(kind, rule, message) = #{ "kind": kind, "rule": rule, "message": message }

// Can a rule of a left-recursive cycle ever get a first match: has the
// cycle an alternative that does not begin by calling into it?
fn seedable(e, cyc, base, nul) = {
    let t = e.t
    if t == "ref" => if contains(cyc, e.name) => base[e.name] ?? false else => true
    else if t == "seq" => {
        let mut ok = true
        let mut going = true
        for x in e.items {
            if going => {
                if !seedable(x, cyc, base, nul) => { ok = false }
                if !nullable(x, nul) => { going = false }
            }
        }
        ok
    }
    else if t == "alt" => col.any(e.items, (x) => seedable(x, cyc, base, nul))
    else if t == "plus" || t == "label" => seedable(e.e, cyc, base, nul)
    else if t == "prec" => seedable(e.atom, cyc, base, nul)
    else => true
}

fn check_rules(P) = {
    let rules = P.rules
    let byname = P.byname
    let nul = P.nul
    let mut out = []
    // rules nothing uses
    let used = reachable(byname, P.start) + reachable(byname, "skip")
    for r in rules {
        if !contains(used, r.name) => {
            out = out + [finding("unused", r.name, "the rule `" + r.name + "` is not used: nothing reachable from `" + P.start + "` refers to it")]
        }
    }
    // alternatives an earlier one hides, and loops over what can match nothing
    for r in rules {
        for e in all_exprs(r.e) {
            if e.t == "alt" => {
                // each later alternative, with the first earlier one that hides it
                let mut j = 1
                while j < len(e.items) {
                    let mut i = 0
                    let mut why = ()
                    while why == () && i < j {
                        why = shadow_reason(e.items[i], e.items[j], byname)
                        i = i + 1
                    }
                    if why != () => {
                        out = out + [finding("shadowed", r.name, "in `" + r.name + "`, the alternative `" + show_expr(e.items[j]) + "` can never match: " + why)]
                    }
                    j = j + 1
                }
            }
            if (e.t == "star" || e.t == "plus") && nullable(e.e, nul) => {
                out = out + [finding("empty_loop", r.name, "in `" + r.name + "`, `" + show_expr(e) + "` repeats something that can match nothing; it stops after one empty match")]
            }
        }
    }
    // left recursion with no way in
    let rec = P.lr.rec
    let mut done = []
    for n in rec {
        if !contains(done, n) => {
            let cyc = filter(rec, (o) => o == n || (contains(reach(P.graph, n, []), o) && contains(reach(P.graph, o, []), n)))
            done = done + cyc
            let mut base = fold(cyc, #{}, (m, c) => map_set(m, c, false))
            let mut changed = true
            while changed {
                changed = false
                for c in cyc {
                    if !base[c] && seedable(byname[c].e, cyc, base, nul) => {
                        base = map_set(base, c, true)
                        changed = true
                    }
                }
            }
            if !col.any(cyc, (c) => base[c]) => {
                for c in cyc {
                    out = out + [finding("left_recursion", c, "`" + c + "` is left-recursive with no way to start: every alternative of " +
                        or_list(map(cyc, (x) => "`" + x + "`")) + " begins by calling the cycle again, so it can never match")]
                }
            }
        }
    }
    out
}

// Problems in a grammar: rules nothing uses, choice alternatives an
// earlier one hides, repetitions of what can match nothing, and left
// recursion that can never get started. `[#{ "kind", "rule",
// "message" }]`, empty when there are none. Takes a grammar or its text.
share fn check(grammar) = {
    let g = if typeof(grammar) == "String" => match grammar_of_text(grammar) { Ok(x) => x, Err(e) => raise(e.message) } else => grammar
    g.check()
}

fn grammar_of_text(text) = grammar(text)

// ── examples ───────────────────────────────────────────────────────────

// The shortest text each rule can match, by its length: what a generator
// falls back on when it has gone deep enough.
fn min_cost(e, cost) = {
    let t = e.t
    if t == "lit" => len(e.codes)
    else if t == "class" || t == "any" => 1
    else if t == "ref" => cost[e.name] ?? 1000000
    else if t == "seq" => fold(e.items, 0, (a, x) => a + min_cost(x, cost))
    else if t == "alt" => min(map(e.items, (x) => min_cost(x, cost)))
    else if t == "star" || t == "opt" || t == "and" || t == "not" => 0
    else if t == "plus" || t == "label" => min_cost(e.e, cost)
    else if t == "prec" => min_cost(e.atom, cost)
    else => 0
}

fn rule_costs(rules) = {
    let mut cost = fold(rules, #{}, (m, r) => map_set(m, r.name, 1000000))
    let mut changed = true
    while changed {
        changed = false
        for r in rules {
            let c = min_cost(r.e, cost)
            if c < cost[r.name] => {
                cost = map_set(cost, r.name, c)
                changed = true
            }
        }
    }
    cost
}

// A seeded generator (an LCG), so examples repeat for a seed.
fn rnd(st, n) = {
    let x = (cell.get(st) * 1103515245 + 12345) % 2147483648
    cell.set(st, x)
    if n <= 0 => 0 else => (x / 65536) % n
}

let PRINTABLE = range(32, 127)

fn gen_char_in(e, st) = {
    if !e.neg && len(e.ranges) > 0 => {
        let (lo, hi) = e.ranges[rnd(st, len(e.ranges))]
        let top = if hi > lo + 94 && lo < 127 => (if hi < 126 => hi else => 126) else => hi
        let c = lo + rnd(st, top - lo + 1)
        str.from_char_code([c])
    }
    else => {
        let ok = filter(PRINTABLE, (c) => class_has(e, c))
        if len(ok) == 0 => "" else => str.from_char_code([ok[rnd(st, len(ok))]])
    }
}

fn gen(P, e, depth, ws, st, cost) = {
    let t = e.t
    let deep = depth > 8
    if t == "lit" => e.s
    else if t == "class" => gen_char_in(e, st)
    else if t == "any" => str.from_char_code([PRINTABLE[rnd(st, len(PRINTABLE))]])
    else if t == "ref" => {
        let r = P.byname[e.name]
        gen(P, r.e, depth + 1, !r.token && r.name != "skip", st, cost)
    }
    else if t == "seq" => {
        let parts = filter(map(e.items, (x) => gen(P, x, depth, ws, st, cost)), (x) => x != "")
        join(parts, if ws => gen_skip(P, st, cost) else => "")
    }
    else if t == "alt" => {
        let pick = if deep => col.min_by(e.items, (x) => min_cost(x, cost)) else => e.items[rnd(st, len(e.items))]
        gen(P, pick, depth, ws, st, cost)
    }
    else if t == "star" || t == "plus" || t == "opt" => {
        let lo = if t == "plus" => 1 else => 0
        let hi = if t == "opt" => 1 else if deep => lo else => lo + 2
        let n = lo + rnd(st, hi - lo + 1)
        let parts = filter(map(range(0, n), (i) => gen(P, e.e, depth + 1, ws, st, cost)), (x) => x != "")
        join(parts, if ws => gen_skip(P, st, cost) else => "")
    }
    else if t == "label" => gen(P, e.e, depth, ws, st, cost)
    else if t == "prec" => {
        let infix = flatten(map(filter(e.levels, (l) => l.assoc == "left" || l.assoc == "right"), (l) => l.ops))
        let prefix = flatten(map(filter(e.levels, (l) => l.assoc == "prefix"), (l) => l.ops))
        let sep = if ws => gen_skip(P, st, cost) else => ""
        let operand = () => {
            let a = gen(P, e.atom, depth + 1, ws, st, cost)
            if len(prefix) > 0 && !deep && rnd(st, 4) == 0 => gen(P, prefix[rnd(st, len(prefix))], depth, ws, st, cost) + sep + a else => a
        }
        let n = if deep || len(infix) == 0 => 0 else => rnd(st, 3)
        let mut out = operand()
        for i in 0..n { out = out + sep + gen(P, infix[rnd(st, len(infix))], depth, ws, st, cost) + sep + operand() }
        out
    }
    else => ""
}

fn gen_skip(P, st, cost) = {
    let r = P.byname["skip"]
    if r == () => "" else => gen(P, r.e, 20, false, st, cost)
}

fn examples_of(P, n, seed) = {
    let st = cell.new(if seed < 0 => 0 - seed else => seed)
    let cost = rule_costs(P.rules)
    let root = P.byname[P.start]
    let mut out = []
    let mut tries = 0
    while len(out) < n && tries < n * 30 {
        let x = gen(P, root.e, 0, !root.token, st, cost)
        tries = tries + 1
        if !contains(out, x) && is_ok(run_parse(P, x, #{})) => { out = out + [x] }
    }
    out
}

// Up to `n` different texts the grammar accepts, generated from its
// rules with a seeded generator (the same seed, the same texts): for
// fuzzing what consumes the trees. Takes a grammar or its text.
share fn examples(grammar, n, seed = 1) = {
    let g = if typeof(grammar) == "String" => match grammar_of_text(grammar) { Ok(x) => x, Err(e) => raise(e.message) } else => grammar
    g.examples(n, seed)
}

// ── tests (the full suite is peg_test.ol) ──────────────────────────────

test "a grammar parses to a tree of kinds, texts and spans" {
    let g = unwrap(grammar(`
      list = "[" item ("," item)* "]"
      item = Num / Word
      Num  = [0-9]+
      Word = [a-z]+
      skip = [ ]+
    `))
    let t = unwrap(g.parse("[1, ab ,22]"))
    assert_eq(t.kind, "list")
    assert_eq(map(t.children, (i) => i.children[0].kind), ["Num", "Word", "Num"])
    assert_eq(t.children[2].children[0].span, #{ "line": 1, "column": 9, "start": 8, "end": 10 })
    assert_eq(dump(t), join(["list", "  item", "    Num \"1\"", "  item", "    Word \"ab\"", "  item", "    Num \"22\""], "\n"))
    assert_true(g.accepts(" [ 7 ] "))
    assert_false(g.accepts("[7,]"))
}

test "an error says what was expected, after what, and where" {
    let g = unwrap(grammar(`
      pair = key:Name ":" val:value
      value "a value" = Name / Num
      Name = [a-z]+
      Num  = [0-9]+
      skip = [ ]+
    `))
    match g.parse("is: ") {
        Ok(_) => raise("parsed"),
        Err(e) => {
            assert_eq(e.message, "expected a value after \":\", found end of input (1:5)")
            assert_eq(e.show(), "expected a value after \":\", found end of input (1:5)\n 1 | is: \n   |     ^")
            assert_eq(e.expected, ["a value"])
            assert_eq(e.found, ())
            assert_eq(e.span.start, 4)
        }
    }
}

test "left recursion, precedence and actions" {
    let lr = unwrap(grammar(`
      sum = sum "-" Num / Num
      Num = [0-9]+
    `, #{ "sum": (m) => if len(m.children) == 1 => m.children[0] else => m.children[0] - m.children[1],
          "Num": (m) => unwrap(str.parse_int(m.text)) }))
    assert_eq(unwrap(lr.parse("9-4-3")), 2)
    let pr = unwrap(grammar(`
      e = precedence(Num) { left "+" "-"; left "*"; right "^"; prefix "-" }
      Num = [0-9]+
    `, #{ "e": (m) => "(" + (if m.fixity == "infix" => m.left + m.op + m.right else => m.op + m.operand) + ")",
          "Num": (m) => m.text }))
    assert_eq(unwrap(pr.parse("1-2-3*4^5^6")), "((1-2)-(3*(4^(5^6))))")
    assert_eq(unwrap(pr.parse("-1^2")), "((-1)^2)")
}

test "grammar errors, check and examples" {
    match grammar("a = b") {
        Ok(_) => raise("accepted"),
        Err(e) => assert_eq(e.message, "peg.grammar: no rule is named `b` (1:5)")
    }
    assert_eq(map(check("a = \"x\" / \"xy\"\nb = \"z\""), (f) => f.kind), ["unused", "shadowed"])
    let g = unwrap(grammar("s = (\"a\" / \"b\")+ [0-9]?"))
    let xs = examples(g, 5, 2)
    assert_eq(len(xs), 5)
    assert_true(col.all(xs, (x) => g.accepts(x)))
    assert_eq(xs, examples(g, 5, 2))
}

test "recovery at sync points" {
    let g = unwrap(grammar(`
      file = stmt*
      stmt = Name "=" Num ";"
      Name = [a-z]+
      Num  = [0-9]+
      skip = [ \\n]+
    `, #{}, #{ "recover": [";"] }))
    match g.parse("a = 1; b = x; c = 3; = 4;") {
        Ok(_) => raise("parsed"),
        Err(e) => {
            assert_eq(len(e.errors), 2)
            assert_eq(len(e.tree.children), 2)
        }
    }
}
