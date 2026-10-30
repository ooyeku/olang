// peg_native_test — the native matcher against the reference engine:
// `olang test src/stdlib/embedded/peg_native_test.ol`.
//
// Every grammar is built twice, once on the native matcher (the
// default) and once on the olang engine (`#{ "engine": "olang" }`, the
// grammar written out as olang functions — the engine peg had before
// the native one, kept for this). Over JSON, CSV, an expression
// language, olang-track's query bar and a handful of grammars for the
// corners (labels, silent rules, left recursion, lookahead, Unicode,
// recovery), both must answer the same trees, the same values and the
// same errors, to the text of `show()`: on thousands of sentences from
// `peg.examples`, and on as many mutated — mostly invalid — inputs.

use peg

let JSON = `
  json    = value
  value   "a value" = object / array / String / Number / True / False / Null
  object  = "{" (member ("," member)*)? "}"
  member  = key:String ":" val:value
  array   = "[" (value ("," value)*)? "]"
  String  = "\\"" ([^"\\\\\\x00-\\x1f] / Escape)* "\\""
  Escape  = "\\\\" (["\\\\/bfnrt] / "u" Hex Hex Hex Hex)
  Hex     = [0-9a-fA-F]
  Number  = "-"? ("0" / [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
  True    = "true"
  False   = "false"
  Null    = "null"
  skip    = [ \\t\\n\\r]+
`

let JSON_ACTIONS = #{
    "json": (m) => m.children[0],
    "value": (m) => m.children[0],
    "object": (m) => fold(m.children, #{}, (acc, kv) => map_set(acc, kv[0], kv[1])),
    "member": (m) => (m.key, m.val),
    "array": (m) => m.children,
    "String": (m) => m.text,
    "Number": (m) => m.text,
    "True": (m) => true,
    "False": (m) => false,
    "Null": (m) => ()
}

let CSV = `
  file   = row (_Eol row)* _Eol?
  row    = field ("," field)*
  field  = Quoted / Bare
  Quoted = "\\"" ([^"] / "\\"\\"")* "\\""
  Bare   = [^,\\r\\n"]*
  _Eol   = "\\r"? "\\n"
`

let EXPR = `
  expr = precedence(atom) {
    left "||"
    left "&&"
    left "==" "!=" "<=" ">=" "<" ">"
    left "+" "-"
    left "*" "/" "%"
    right "^"
    prefix "-" "!"
    postfix "!"
  }
  atom  = call / Num / Name / "(" expr ")"
  call  = fn:Name "(" args:(expr ("," expr)*)? ")"
  Num   = [0-9]+ ("." [0-9]+)?
  Name  = [a-z_] [a-z0-9_]*
  skip  = [ \\t\\n]+
`

// An action for every kind, so a value shows the whole shape.
let EXPR_ACTIONS = #{
    "expr": (m) => "(" + (if m.fixity == "infix" => m.left + m.op + m.right
        else if m.fixity == "prefix" => m.op + m.operand else => m.operand + m.op) + ")",
    "atom": (m) => join(m.children, ""),
    "call": (m) => m.fn + "[" + (if m.args == () => "" else => join(m.args.children, ",")) + "]",
    "args": (m) => m,
    "Num": (m) => m.text,
    "Name": (m) => m.text
}

let QUERY = `
  query      = _item*
  _item      = pending / Pair / negated / group / Close / Or / _Colon / Quoted / Bare
  pending    = key:PendKey val:_value?
  negated    = Not group
  group      = "(" _inner* shut:Close?
  _inner     = pending_in / Pair / negated / group / Or / _Colon / Quoted / Bare
  pending_in = key:PendKey val:_word?
  _word      = Quoted / Bare
  _value     = Quoted / Open / Close / Bare
  _Colon     = ":" !Wc
  Or         = "or"i !Wc
  Not        = "not"i !Wc &(Sp* "(")
  PendKey    = (!(":" !Wc) Wc)+ ":" !Wc
  Pair       = [^ \\t\\u{a0}"():]+ ":" Wc+
  Bare       = Wc+
  Quoted     = "\\"" [^"]* "\\""?
  Open       = "("
  Close      = ")"
  Wc         = [^ \\t\\u{a0}"()]
  Sp         = [ \\t\\u{a0}]
  skip       = [ \\t\\u{a0}]+
`

// The corners: labels on rules, groups and repetitions, through silent
// rules; lookahead; case-insensitive literals; Unicode classes; `.`.
let DECL = `
  decl  = name:Name ":" type:Name ("=" init:Num)? attrs:attr* pair:(k:Name "=" v:Num)? _rest
  _rest = ("," more:Name)* ";"?
  attr  = "@" Name / "@" "(" Name ")"
  Name  = !Kw [a-zα-ω_]+
  Kw    = ("let"i / "if"i) ![a-z]
  Num   = [0-9]+ / "0x" [0-9a-f]+
  skip  = [ \\t]+ / "#" (!"\\n" .)*
`

let LR = `
  e = e "+" t / e "-" t / t
  t = t "*" f / f
  f = "(" e ")" / n:Num / "-" f
  Num = Num [0-9] / [0-9]
  skip = [ ]+
`

let STMTS = `
  file = stmt*
  stmt = name:Name "=" val:value ";"
  value "a value" = Num / Name / "[" (value ("," value)*)? "]"
  Name = [a-z]+
  Num "a number" = [0-9]+
  skip = [ \\n]+
`

let JSON_ALPHA = ["{", "}", "[", "]", ",", ":", "\"", "\\", " ", "\n", "1", "0", "-", ".", "e", "t", "n", "u", "a", "x"]
let CSV_ALPHA = [",", "\"", "\n", "\r", "a", " ", "1"]
let EXPR_ALPHA = ["+", "-", "*", "/", "^", "!", "(", ")", ",", " ", "1", "x", "<", "=", "&", "|", "f"]
let QUERY_ALPHA = [":", "(", ")", "\"", " ", "\t", "-", ",", "a", "or", "not", "\u{a0}", "é"]
let DECL_ALPHA = [":", "=", "@", "(", ")", ",", ";", " ", "#", "\n", "x", "λ", "0x", "let", "1"]
let LR_ALPHA = ["+", "-", "*", "(", ")", " ", "1", "9"]
let STMTS_ALPHA = ["=", ";", "[", "]", ",", " ", "\n", "a", "1"]

fn lcg(seed) = (seed * 1103515245 + 12345) % 2147483648

// `n` variants of `text`: a character deleted, one inserted from the
// grammar's alphabet, one replaced, the text cut short, two pieces
// swapped — mostly invalid, sometimes not.
fn mutations(text, alpha, n, seed0) = {
    let chars = str.chars(text)
    let k = len(chars)
    let mut seed = seed0
    let mut out = []
    for i in 0..n {
        seed = lcg(seed)
        let at = if k == 0 => 0 else => (seed / 7) % (k + 1)
        let pick = alpha[(seed / 13) % len(alpha)]
        let way = seed % 5
        let before = join(take(chars, at), "")
        let after = join(skip(chars, at), "")
        let rest = join(skip(chars, at + 1), "")
        out = out + [
            if way == 0 => before + rest
            else if way == 1 => before + pick + after
            else if way == 2 => before + pick + rest
            else if way == 3 => before
            else => after + before
        ]
    }
    out
}

fn both(text, actions, options) = (
    unwrap(peg.grammar(text, actions, options)),
    unwrap(peg.grammar(text, actions, #{ ...options, "engine": "olang" })))

fn is_node(v) = typeof(v) == "Map" && map_has_key(v, "kind") && map_has_key(v, "span")

fn same_error(a, b, x) = {
    assert_eq(a.message, b.message, x)
    assert_eq(a.show(), b.show(), x)
    assert_eq(a.span, b.span, x)
    assert_eq(a.expected, b.expected, x)
    assert_eq(a.found, b.found, x)
    assert_eq(a.after, b.after, x)
}

// The same answer from both engines, to the last field. 1 when the
// input parsed, 0 when it did not.
fn agree(N, O, x, opts) = {
    let a = peg.parse(N, x, opts)
    let b = peg.parse(O, x, opts)
    match (a, b) {
        (Ok(t), Ok(u)) => {
            assert_eq(t, u, x)
            if is_node(t) => assert_eq(peg.dump(t), peg.dump(u), x)
            1
        },
        (Err(e), Err(f)) => {
            same_error(e, f, x)
            if map_has_key(e, "errors") || map_has_key(f, "errors") => {
                assert_eq(len(e.errors), len(f.errors), x)
                for i in 0..len(e.errors) { same_error(e.errors[i], f.errors[i], x) }
                assert_eq(e.tree, f.tree, x)
            }
            0
        },
        _ => raise("the engines disagree on whether this parses: " + show(x) + " native " + show(is_ok(a)) + ", olang " + show(is_ok(b)))
    }
}

// Both engines over `inputs`, parse and accepts: (inputs, parsed).
fn compare(N, O, inputs, opts) = {
    let mut ok = 0
    for x in inputs {
        ok = ok + agree(N, O, x, opts)
        assert_eq(N.accepts(x), O.accepts(x), x)
    }
    (len(inputs), ok)
}

// A corpus: `n` examples per seed and `m` mutations of each.
fn corpus(g, seeds, n, alpha, m) = {
    let mut out = []
    for s in seeds {
        for x in peg.examples(g, n, s) {
            out = out + [x] + mutations(x, alpha, m, s * 31 + len(x))
        }
    }
    out
}

fn report(name, counts) = println(name + ": " + to_string(counts[0]) + " inputs, " + to_string(counts[1]) + " parsed, the rest errors — the same from both engines")

test "differential: JSON trees, values and errors" {
    let (N, O) = both(JSON, #{}, #{})
    let fixed = ["0", "-12", "3.25", "1e3", "\"a\\\"b\"", "[]", "{}", "[1, [2, [3, []]]]",
        "{\"a\": [1, 2 3]}", "{\"a\": }", "[1,\n\t2,\n", "[1] x", "[\t1 2]", "{\"a\" 1}", "\"abc",
        "\"caf\u{e9} \u{1F600}\"", "", "   ", "[\"\\u12G4\"]", "{\"k\":\"v\",}", "nul", "-", "01"]
    let inputs = fixed + corpus(N, [1, 2, 3, 4, 5], 120, JSON_ALPHA, 3)
    let counts = compare(N, O, inputs, #{})
    report("JSON", counts)
    assert_true(counts[0] > 1500)
    let (NV, OV) = both(JSON, JSON_ACTIONS, #{})
    compare(NV, OV, take(inputs, 600), #{})
    // another start rule, and memo all / none
    compare(N, O, take(inputs, 300), #{ "start": "value" })
    let (NA, OA) = both(JSON, #{}, #{ "memo": "all" })
    compare(NA, OA, take(inputs, 300), #{})
    let (NN, ON) = both(JSON, #{}, #{ "memo": "none" })
    compare(NN, ON, take(inputs, 300), #{})
}

test "differential: CSV" {
    let (N, O) = both(CSV, #{}, #{})
    let fixed = ["a,b,c\n1,2,3\n", "name,note\r\nann,\"says \"\"hi\"\"\"\r\n", "x,\"line\nline\",z", ",,", "a,\"open", "\"a\"b", "\r"]
    let counts = compare(N, O, fixed + corpus(N, [1, 2, 3], 100, CSV_ALPHA, 3), #{})
    report("CSV", counts)
    assert_true(counts[0] > 900)
}

test "differential: expressions, precedence, left recursion" {
    let (N, O) = both(EXPR, #{}, #{})
    let fixed = ["1 - 2 - 3", "2 ^ 3 ^ 2", "-2 ^ 2", "3! + 1", "max(3, 9, 4) * 2", "1 + * 2", "(1 + 2",
        "f()", "f(1,)", "!(1 < 2) || 1 >= 1", "1 <= 1", "--1", "1!!", "a(b(c(d)))", ""]
    let inputs = fixed + corpus(N, [1, 2, 3, 4], 150, EXPR_ALPHA, 3)
    let counts = compare(N, O, inputs, #{})
    report("expressions", counts)
    assert_true(counts[0] > 2000)
    let (NV, OV) = both(EXPR, EXPR_ACTIONS, #{})
    compare(NV, OV, take(inputs, 500), #{})
    let (NL, OL) = both(LR, #{}, #{})
    let lr = ["1+2*3-(4*5+6)*7", "1+*2", "12-3", "-1*-2", "(1", "1+(2", "99 * (8 - 7) - 6"]
    report("left recursion", compare(NL, OL, lr + corpus(NL, [1, 2, 3], 100, LR_ALPHA, 3), #{}))
}

test "differential: olang-track's query bar" {
    let (N, O) = both(QUERY, #{}, #{})
    let PIECES = ["state:", "tag:", "due:", "sort:", "top:", "nope:", "open", "-done", "ui,server", "<2026-09-05",
        ">-3d", "OR", "or", "(", ")", "((", "))", "\"", "\"quoted phrase\"", "\"open", "-", ",", ":", "::", "word",
        "\\", "🙂", "state:open", "group: state", "  ", "\t", "not", "NOT (", "a:b:", "x:", "\u{a0}"]
    let mut queries = []
    let mut seed = 7
    for k in 0..1200 {
        let mut parts = []
        for j in 0..(1 + k % 11) {
            seed = lcg(seed)
            parts = parts + [PIECES[seed % len(PIECES)]]
        }
        queries = queries + [join(parts, if seed % 3 == 0 => "" else => " ")]
    }
    let counts = compare(N, O, queries + corpus(N, [1, 2], 100, QUERY_ALPHA, 2), #{})
    report("query bar", counts)
    assert_true(counts[0] > 1500)
}

test "differential: labels, silent rules, lookahead, Unicode, comments in skip" {
    let (N, O) = both(DECL, #{}, #{})
    let fixed = ["x : int", "x : int = 4 @a @b y = 7", "x:int=0xff @(z), q, r;", "let : x", "λx : t # note\n",
        "x : int = @", "IF : x", "x : y , ;", "αβ : γ"]
    report("labels and lookahead", compare(N, O, fixed + corpus(N, [1, 2, 3], 120, DECL_ALPHA, 3), #{}))
}

test "differential: recovery" {
    let (N, O) = both(STMTS, #{}, #{ "recover": [";"] })
    let fixed = ["a = 1;\nb = ;\nc = 3;\nd 4;\ne = 5;", "a = [1, 2;", "= = =", "a = 1; b = [x, ; c = 2;", ""]
    let counts = compare(N, O, fixed + corpus(N, [1, 2, 3], 100, STMTS_ALPHA, 4), #{})
    report("recovery", counts)
    // recovery asked of one parse only, on a grammar without it
    let (P, Q) = both(STMTS, #{}, #{})
    compare(P, Q, ["x = ; y = ; z = 1;", "x = 1;"], #{ "recover": [";", "\n"] })
}
