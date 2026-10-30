// peg_test — the `peg` module's suite: `olang test src/stdlib/embedded/peg_test.ol`.
//
// JSON, CSV and an expression language, left recursion, precedence and
// associativity, labels and classes, errors as golden strings, recovery,
// `peg.check`, fuzzing with `peg.examples`, and olang-track's query
// language giving the same answers as its hand-written parser on a
// corpus. The faster checks of the same ground live in peg.ol itself.

use peg

// ── grammars ───────────────────────────────────────────────────────────

// JSON. A grammar in a template string is written with its backslashes
// doubled: a template reads `\\` as one backslash and leaves the rest.
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

fn json_number(t) =
    if str.contains(t, ".") || str.contains(t, "e") || str.contains(t, "E") => unwrap(str.parse_float(t))
    else => unwrap(str.parse_int(t))

let JSON_ACTIONS = #{
    "json": (m) => m.children[0],
    "value": (m) => m.children[0],
    "object": (m) => fold(m.children, #{}, (acc, kv) => map_set(acc, kv[0], kv[1])),
    "member": (m) => (m.key, m.val),
    "array": (m) => m.children,
    "String": (m) => unwrap(json.parse(m.text)),
    "Number": (m) => json_number(m.text),
    "True": (m) => true,
    "False": (m) => false,
    "Null": (m) => ()
}

// CSV (RFC 4180's shape): quoted fields may hold commas, newlines and
// doubled quotes; a field may be empty.
let CSV = `
  file   = row (_Eol row)* _Eol?
  row    = field ("," field)*
  field  = Quoted / Bare
  Quoted = "\\"" ([^"] / "\\"\\"")* "\\""
  Bare   = [^,\\r\\n"]*
  _Eol   = "\\r"? "\\n"
`

let CSV_ACTIONS = #{
    "file": (m) => filter(m.children, (r) => r != [""]),
    "row": (m) => m.children,
    "field": (m) => m.children[0],
    "Quoted": (m) => str.replace(str.substring(m.text, 1, str.length(m.text) - 1), "\"\"", "\""),
    "Bare": (m) => m.text
}

// An expression language: a precedence table, calls, and names.
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

fn ipow(a, b) = if b <= 0 => 1 else => a * ipow(a, b - 1)
fn fact(n) = if n <= 1 => 1 else => n * fact(n - 1)

fn eval_op(op, l, r) =
    if op == "+" => l + r
    else if op == "-" => l - r
    else if op == "*" => l * r
    else if op == "/" => l / r
    else if op == "%" => l % r
    else if op == "^" => ipow(l, r)
    else if op == "<" => l < r
    else if op == ">" => l > r
    else if op == "<=" => l <= r
    else if op == ">=" => l >= r
    else if op == "==" => l == r
    else if op == "!=" => l != r
    else if op == "&&" => l && r
    else if op == "||" => l || r
    else => raise("no op " + op)

let EXPR_ACTIONS = #{
    "expr": (m) =>
        if m.fixity == "infix" => eval_op(m.op, m.left, m.right)
        else if m.fixity == "prefix" => (if m.op == "-" => 0 - m.operand else => !m.operand)
        else => fact(m.operand),
    "atom": (m) => m.children[0],
    "call": (m) => if m.fn == "max" => max(m.children[1].children) else => len(m.children[1].children),
    "args": (m) => m,
    "Num": (m) => unwrap(str.parse_int(m.text)),
    "Name": (m) => m.text
}

fn grammar_of(text) = match peg.grammar(text) { Ok(g) => g, Err(e) => raise(e.show()) }
fn grammar_with(text, actions) = match peg.grammar(text, actions) { Ok(g) => g, Err(e) => raise(e.show()) }

fn tree(g, input) = match g.parse(input) { Ok(t) => peg.dump(t), Err(e) => "ERROR " + e.show() }
fn value(g, input) = match g.parse(input) { Ok(v) => v, Err(e) => raise(e.show()) }
fn shown(g, input) = match g.parse(input) { Ok(t) => "parsed: " + peg.dump(t), Err(e) => e.show() }

// ── JSON ───────────────────────────────────────────────────────────────

test "JSON values agree with json.parse" {
    let J = grammar_with(JSON, JSON_ACTIONS)
    for doc in [
        "0", "-12", "3.25", "1e3", "-2.5E-2", "true", "false", "null", "\"\"", "\"a\\\"b\\\\c\\n\"",
        "[]", "{}", "[1, [2, [3, []]]]", "{\"a\": {\"b\": {\"c\": [true, null]}}}",
        " \n\t[ 1 ,2,\n 3 ] \n", "{\"k\": \"v\", \"n\": -0.5, \"l\": [\"x\", {\"y\": 1}]}",
        "\"caf\u{e9} \u{1F600}\""
    ] {
        assert_eq(value(J, doc), unwrap(json.parse(doc)), doc)
    }
}

test "JSON tree: labels, kinds and spans" {
    let J = grammar_of(JSON)
    assert_eq(tree(J, "{\"a\": [1, true]}"), join([
        "json",
        "  value",
        "    object",
        "      member",
        "        key: String \"\\\"a\\\"\"",
        "        val: value",
        "          array",
        "            value",
        "              Number \"1\"",
        "            value",
        "              True \"true\""], "\n"))
    let t = unwrap(J.parse("[\n  1,\n  {\"k\": 22}\n]"))
    let obj = t.children[0].children[0].children[1].children[0]
    assert_eq(obj.kind, "object")
    assert_eq(obj.span, #{ "line": 3, "column": 3, "start": 9, "end": 18 })
    assert_eq(obj.children[0].val.text, "22")
    assert_eq(obj.children[0].val.span.column, 9)
}

test "JSON errors say what was expected, where, and after what" {
    let J = grammar_of(JSON)
    assert_eq(shown(J, "{\"a\": [1, 2 3]}"), join([
        "expected \",\" or \"]\" after \"2\", found \"3\" (1:13)",
        " 1 | {\"a\": [1, 2 3]}",
        "   |             ^"], "\n"))
    assert_eq(shown(J, "{\"a\": }"), join([
        "expected a value after \":\", found \"}\" (1:7)",
        " 1 | {\"a\": }",
        "   |       ^"], "\n"))
    assert_eq(shown(J, "[1,\n\t2,\n"), join([
        "expected a value after \",\", found end of input (3:1)",
        " 3 | ",
        "   | ^"], "\n"))
    assert_eq(shown(J, "[1] x"), join([
        "expected end of input after \"]\", found \"x\" (1:5)",
        " 1 | [1] x",
        "   |     ^"], "\n"))
    // the caret keeps a tab where the line has one
    assert_eq(shown(J, "[\t1 2]"), join([
        "expected \",\" or \"]\" after \"1\", found \"2\" (1:5)",
        " 1 | [\t1 2]",
        "   |  \t  ^"], "\n"))
    let e = match J.parse("{\"a\" 1}") { Err(x) => x, Ok(_) => raise("parsed") }
    assert_eq(e.message, "expected \":\" after \"\\\"a\\\"\", found \"1\" (1:6)")
    assert_eq(e.expected, ["\":\""])
    assert_eq(e.found, "1")
    assert_eq(e.after, "\"a\"")
    assert_eq(e.span, #{ "line": 1, "column": 6, "start": 5, "end": 6 })
    let unterminated = match J.parse("\"abc") { Err(x) => x, Ok(_) => raise("parsed") }
    assert_eq(unterminated.found, ())
    assert_true(contains(unterminated.expected, "\"\\\"\""))
}

test "JSON: a large document, in order" {
    let J = grammar_with(JSON, JSON_ACTIONS)
    let rec = "{\"id\": 7, \"tags\": [\"a\", \"b\"], \"price\": 12.5, \"ok\": true, \"none\": null}"
    let doc = "[" + join(map(range(0, 300), (i) => rec), ",\n") + "]"
    let v = value(J, doc)
    assert_eq(len(v), 300)
    assert_eq(v, unwrap(json.parse(doc)))
}

test "JSON fuzz: generated documents parse here and in json.parse alike" {
    let J = grammar_with(JSON, JSON_ACTIONS)
    let docs = peg.examples(grammar_of(JSON), 40, 11)
    assert_eq(len(docs), 40)
    for d in docs {
        match json.parse(d) {
            Ok(expected) => assert_eq(value(J, d), expected, d),
            // a generated exponent can be past a float's range, which
            // json.parse refuses and the grammar does not know about
            Err(e) => assert_true(str.contains(e, "range") || str.contains(e, "number"), e)
        }
    }
}

// ── CSV ────────────────────────────────────────────────────────────────

test "CSV: quoted fields, empty fields, CRLF" {
    let C = grammar_with(CSV, CSV_ACTIONS)
    assert_eq(value(C, "a,b,c\n1,2,3\n"), [["a", "b", "c"], ["1", "2", "3"]])
    assert_eq(value(C, "name,note\r\nann,\"says \"\"hi\"\", then, leaves\"\r\nbo,\r\n"),
        [["name", "note"], ["ann", "says \"hi\", then, leaves"], ["bo", ""]])
    assert_eq(value(C, "x,\"line one\nline two\",z"), [["x", "line one\nline two", "z"]])
    assert_eq(value(C, ",,"), [["", "", ""]])
    assert_eq(shown(C, "a,\"open"), join([
        "expected [^\"], \"\\\"\\\"\" or \"\\\"\", found end of input (1:8)",
        " 1 | a,\"open",
        "   |        ^"], "\n"))
}

// ── expressions, precedence and associativity ──────────────────────────

test "precedence: levels, left and right associativity, prefix and postfix" {
    let E = grammar_of(EXPR)
    assert_eq(tree(E, "1 - 2 - 3"), join([
        "expr op=\"-\" fixity=\"infix\"",
        "  left: expr op=\"-\" fixity=\"infix\"",
        "    left: atom",
        "      Num \"1\"",
        "    right: atom",
        "      Num \"2\"",
        "  right: atom",
        "    Num \"3\""], "\n"))
    assert_eq(tree(E, "2 ^ 3 ^ 2"), join([
        "expr op=\"^\" fixity=\"infix\"",
        "  left: atom",
        "    Num \"2\"",
        "  right: expr op=\"^\" fixity=\"infix\"",
        "    left: atom",
        "      Num \"3\"",
        "    right: atom",
        "      Num \"2\""], "\n"))
    let V = grammar_with(EXPR, EXPR_ACTIONS)
    assert_eq(value(V, "1 + 2 * 3"), 7)
    assert_eq(value(V, "(1 + 2) * 3"), 9)
    assert_eq(value(V, "10 - 4 - 3"), 3)
    assert_eq(value(V, "100 / 10 / 5"), 2)
    assert_eq(value(V, "2 ^ 3 ^ 2"), 512)
    assert_eq(value(V, "-2 ^ 2"), 4)
    assert_eq(value(V, "-(2 ^ 2)"), -4)
    assert_eq(value(V, "3! + 1"), 7)
    assert_eq(value(V, "1 + 2 < 4 && 2 * 2 == 4"), true)
    assert_eq(value(V, "!(1 < 2) || 1 >= 1"), true)
    assert_eq(value(V, "max(3, 9, 4) * 2"), 18)
    assert_eq(value(V, "1 <= 1"), true)
}

test "precedence errors" {
    let E = grammar_of(EXPR)
    assert_eq(shown(E, "1 + * 2"), join([
        "expected \"-\", \"!\", Name, Num or \"(\" after \"+\", found \"*\" (1:5)",
        " 1 | 1 + * 2",
        "   |     ^"], "\n"))
    assert_eq(shown(E, "(1 + 2"), join([
        "expected \"!\", \"*\", \"/\", \"%\", \"^\", \"||\", \"&&\", \"==\", \"!=\", \"<=\", \">=\", \"<\", \">\", \"+\", \"-\" or \")\" after \"2\", found end of input (1:7)",
        " 1 | (1 + 2",
        "   |       ^"], "\n"))
}

// ── left recursion ─────────────────────────────────────────────────────

test "direct left recursion builds a left-leaning tree" {
    let G = grammar_of(`
      sum  = sum op:"+" num / sum op:"-" num / num
      num  = Digits
      Digits = [0-9]+
      skip = [ ]+
    `)
    assert_eq(tree(G, "1 - 2 + 3"), join([
        "sum",
        "  sum",
        "    sum",
        "      num",
        "        Digits \"1\"",
        "    op: op \"-\"",
        "    num",
        "      Digits \"2\"",
        "  op: op \"+\"",
        "  num",
        "    Digits \"3\""], "\n"))
    let V = grammar_with(`
      sum  = sum "-" num / num
      num  = [0-9]+
    `, #{ "sum": (m) => if len(m.children) == 1 => m.children[0] else => m.children[0] - m.children[1],
          "num": (m) => unwrap(str.parse_int(m.text)) })
    assert_eq(value(V, "10-3-2"), 5)
}

test "indirect and mutual left recursion" {
    // a → b → a
    let G = grammar_of(`
      a = b "x" / "a"
      b = a "y"
    `)
    assert_true(G.accepts("a"))
    assert_true(G.accepts("ayx"))
    assert_true(G.accepts("ayxyx"))
    assert_false(G.accepts("ay"))
    assert_false(G.accepts("ayxy"))
    // two heads on one cycle, each also directly left-recursive
    let H = grammar_of(`
      e = e "+" t / t
      t = t "*" f / f
      f = "(" e ")" / [0-9]
    `)
    assert_true(H.accepts("1+2*3+(4*5+6)*7"))
    assert_false(H.accepts("1+*2"))
    let V = grammar_with(`
      e = e "+" t / t
      t = t "*" f / f
      f = "(" e ")" / [0-9]
    `, #{ "e": (m) => if len(m.children) == 1 => m.children[0] else => m.children[0] + m.children[1],
          "t": (m) => if len(m.children) == 1 => m.children[0] else => m.children[0] * m.children[1],
          "f": (m) => if len(m.children) == 1 => m.children[0] else => unwrap(str.parse_int(m.text)) })
    assert_eq(value(V, "1+2*3+(4*5+6)*7"), 189)
    // left recursion inside a token
    let T = grammar_of(`
      Num = Num [0-9] / [0-9]
    `)
    assert_eq(tree(T, "2026"), "Num \"2026\"")
}

test "left recursion that cannot start fails and is reported" {
    let G = grammar_of(`
      a = a "x"
    `)
    assert_false(G.accepts("x"))
    assert_eq(map(G.check(), (f) => f.kind), ["left_recursion"])
}

// ── labels, silent rules, classes, lookahead ───────────────────────────

test "labels: single, repeated, on groups, through silent rules" {
    let G = grammar_of(`
      decl  = name:Name ":" type:Name ("=" init:Num)? attrs:attr* pair:(k:Name "=" v:Num)?
      attr  = "@" Name
      Name  = [a-z]+
      Num   = [0-9]+
      skip  = [ ]+
    `)
    let t = unwrap(G.parse("x : int = 4 @a @b y = 7"))
    assert_eq(t.name.text, "x")
    assert_eq(t.type.text, "int")
    assert_eq(t.init.text, "4")
    assert_eq(map(t.attrs, (a) => a.text), ["@a", "@b"])
    assert_eq(t.pair.kind, "pair")
    assert_eq(t.pair.k.text, "y")
    assert_eq(t.pair.v.text, "7")
    let u = unwrap(G.parse("x : int"))
    assert_eq(u.init, ())
    assert_eq(u.attrs, [])
    assert_eq(u.pair, ())
    // a silent rule's labels belong to the rule that uses it
    let S = grammar_of(`
      call  = callee:Name _args
      _args = "(" args:Name* ")"
      Name  = [a-z]+
      skip  = [ ]+
    `)
    let c = unwrap(S.parse("f(a b c)"))
    assert_eq(c.callee.text, "f")
    assert_eq(map(c.args, (a) => a.text), ["a", "b", "c"])
    assert_eq(map(c.children, (k) => k.kind), ["Name", "Name", "Name", "Name"])
}

test "classes: ranges, negation, escapes, shorthands, Unicode; literals ignoring case" {
    let G = grammar_of(`
      all    = Greek " " Emoji " " Esc " " Neg " " Short " " Kw
      Greek  = [α-ω]+
      Emoji  = [\\u{1F600}-\\u{1F64F}]
      Esc    = [\\]\\\\\\-\\t]+
      Neg    = [^a-z ]+
      Short  = \\d+ \\w \\s? [\\x41-\\x43]
      Kw     = "select"i
    `)
    assert_true(G.accepts("λόγος 😀 ]\\-\t 123ABC 42z A SeLeCt") == false)
    assert_true(G.accepts("λλω 😀 ]\\- 1A2B 42_ A SeLeCt"))
    assert_true(G.accepts("αβ 🙏 \\ !! 7x\tC SELECT"))
    assert_false(G.accepts("αβ 🙏 \\ !! 7x\tD SELECT"))
    assert_false(G.accepts("ab 😀 ] X 1aA select"))
    let t = unwrap(G.parse("αβ 🙏 \\ !! 7x\tC SELECT"))
    // positions count code points, as str.substring does
    assert_eq(t.children[1].span, #{ "line": 1, "column": 4, "start": 3, "end": 4 })
    assert_eq(t.children[5].text, "SELECT")
}

test "lookahead: keywords are not names, names are not keywords" {
    let G = grammar_of(`
      stmt    = If cond:Name Then body:Name / assign
      assign  = target:Name "=" Name
      If      = "if" !IdChar
      Then    = "then" !IdChar
      Name    = !Keyword [a-z]+
      Keyword = ("if" / "then") !IdChar
      IdChar  = [a-z0-9_]
      skip    = [ ]+
    `)
    assert_eq(unwrap(G.parse("if x then y")).cond.text, "x")
    assert_eq(unwrap(G.parse("iffy = thenx")).children[0].target.text, "iffy")
    assert_false(G.accepts("if = y"))
    assert_eq(unwrap(G.parse("x = y")).kind, "stmt")
    let A = grammar_of(`
      word = &[a-z] Letters !"!"
      Letters = [a-z]+
    `)
    assert_true(A.accepts("abc"))
    assert_false(A.accepts("abc!"))
}

test "spans over several lines" {
    let G = grammar_of(`
      lines = Item+
      Item  = [a-z]+ "\\n"?
    `)
    let t = unwrap(G.parse("ab\ncde\nf"))
    assert_eq(len(t.children), 3)
    assert_false(G.accepts("ab\n\ncde"))
    let H = grammar_of(`
      doc  = word+
      word = W
      W    = [a-zé]+
      skip = [ \\n]+
    `)
    let d = unwrap(H.parse("one\n  two\n\nthé four"))
    assert_eq(map(d.children, (w) => (w.text, w.span.line, w.span.column, w.span.start)),
        [("one", 1, 1, 0), ("two", 2, 3, 6), ("thé", 4, 1, 11), ("four", 4, 5, 15)])
}

// ── actions ────────────────────────────────────────────────────────────

test "actions turn matches into values, labels included" {
    let G = grammar_with(`
      list  = "[" items:item ("," items:item)* "]"
      item  = n:Num / s:Str
      Num   = [0-9]+
      Str   = [a-z]+
      skip  = [ ]+
    `, #{ "list": (m) => m.items,
          "item": (m) => if m.n != () => m.n * 10 else => str.to_upper(m.s.text),
          "Num": (m) => unwrap(str.parse_int(m.text)) })
    assert_eq(value(G, "[1, ab, 3]"), [10, "AB", 30])
    // an action for a label's own node
    let L = grammar_with(`
      pair = left:Name sep:("=" / ":") right:Name
      Name = [a-z]+
    `, #{ "sep": (m) => if m.text == "=" => "eq" else => "colon", "pair": (m) => m.sep })
    assert_eq(value(L, "a:b"), "colon")
}

test "an action for a name the grammar does not have is refused" {
    match peg.grammar("a = Num\nNum = [0-9]+", #{ "Numb": (m) => 1 }) {
        Ok(_) => raise("accepted"),
        Err(e) => assert_eq(e.message, "peg.grammar: an action for `Numb`, which is no rule or label; did you mean `Num`? (1:1)")
    }
}

// ── grammar errors ─────────────────────────────────────────────────────

test "grammar text errors point at the grammar" {
    let err = (text) => match peg.grammar(text) { Ok(_) => "accepted", Err(e) => e.show() }
    assert_eq(err("a = b\nb = (\"x\""), join([
        "peg.grammar: expected `)` to close the group (2:9)",
        " 2 | b = (\"x\"",
        "   |         ^"], "\n"))
    assert_eq(err("value = objekt\nobject = \"{\" \"}\""), join([
        "peg.grammar: no rule is named `objekt`; did you mean `object`? (1:9)",
        " 1 | value = objekt",
        "   |         ^"], "\n"))
    assert_eq(err("a = [z-a]"), join([
        "peg.grammar: a range runs backwards: z-a (1:5)",
        " 1 | a = [z-a]",
        "   |     ^"], "\n"))
    assert_eq(err("a = \"x\"\na = \"y\""), join([
        "peg.grammar: the rule `a` is defined twice (2:1)",
        " 2 | a = \"y\"",
        "   | ^"], "\n"))
    assert_eq(err("A = k:[a-z]"), join([
        "peg.grammar: `A` is a token, matched as one piece of text; a label inside it names nothing (1:5)",
        " 1 | A = k:[a-z]",
        "   |     ^"], "\n"))
    assert_eq(err("a = x:\"1\" text:\"2\""), join([
        "peg.grammar: a label cannot be named `text`: every node has its own text (1:11)",
        " 1 | a = x:\"1\" text:\"2\"",
        "   |           ^"], "\n"))
    assert_eq(err("e = precedence(a) { middle \"+\" }\na = \"1\""), join([
        "peg.grammar: expected left, right, prefix or postfix to start a level (1:21)",
        " 1 | e = precedence(a) { middle \"+\" }",
        "   |                     ^"], "\n"))
    assert_eq(err(""), join([
        "peg.grammar: the grammar has no rules (1:1)",
        " 1 | ",
        "   | ^"], "\n"))
}

// ── recovery ───────────────────────────────────────────────────────────

let STMTS = `
  file = stmt*
  stmt = name:Name "=" val:Num ";"
  Name = [a-z]+
  Num "a number" = [0-9]+
  skip = [ \\n]+
`

test "recovery: every error in one pass, and the tree of what parsed" {
    let G = unwrap(peg.grammar(STMTS, #{}, #{ "recover": [";"] }))
    match G.parse("a = 1;\nb = ;\nc = 3;\nd 4;\ne = 5;") {
        Ok(_) => raise("parsed"),
        Err(e) => {
            assert_eq(map(e.errors, (x) => x.message), [
                "expected a number after \"=\", found \";\" (2:5)",
                "expected \"=\" after \"d\", found \"4\" (4:3)"])
            assert_eq(e.message, e.errors[0].message)
            assert_eq(map(e.tree.children, (s) => s.name.text + "=" + s.val.text), ["a=1", "c=3", "e=5"])
            assert_eq(e.show(), join([
                "expected a number after \"=\", found \";\" (2:5)",
                " 2 | b = ;",
                "   |     ^",
                "expected \"=\" after \"d\", found \"4\" (4:3)",
                " 4 | d 4;",
                "   |   ^"], "\n"))
        }
    }
    // without errors a recovering parse is an ordinary one
    assert_eq(len(unwrap(G.parse("a = 1; b = 2;")).children), 2)
    // recovery asked of one parse only
    let P = grammar_of(STMTS)
    assert_eq(len(unwrap_err(peg.parse(P, "x = ; y = ; z = 1;", #{ "recover": [";"] })).errors), 2)
    assert_true(is_err(P.parse("x = ; y = 1;")))
}

fn unwrap_err(r) = match r { Err(e) => e, Ok(_) => raise("expected an error") }

// ── peg.check ──────────────────────────────────────────────────────────

test "check: unused rules, shadowed alternatives, empty loops, hopeless left recursion" {
    let found = peg.check(`
      top   = cmp / word / list
      cmp   = "<" / "<=" / [a-z] / "q" / "<"
      word  = [a-z]+ / "x"
      list  = item*
      item  = "-"?
      loop  = loop "!"
      lost  = "never"
    `)
    assert_eq(map(found, (f) => f.message), [
        "the rule `loop` is not used: nothing reachable from `top` refers to it",
        "the rule `lost` is not used: nothing reachable from `top` refers to it",
        "in `cmp`, the alternative `\"<=\"` can never match: `\"<\"` before it matches its start first",
        "in `cmp`, the alternative `\"q\"` can never match: `[a-z]` before it matches that character first",
        "in `cmp`, the alternative `\"<\"` can never match: it is the same as an alternative before it",
        "in `list`, `item*` repeats something that can match nothing; it stops after one empty match",
        "`loop` is left-recursive with no way to start: every alternative of `loop` begins by calling the cycle again, so it can never match"])
    assert_eq(map(found, (f) => f.kind), ["unused", "unused", "shadowed", "shadowed", "shadowed", "empty_loop", "left_recursion"])
    assert_eq(peg.check(JSON), [])
    assert_eq(peg.check(EXPR), [])
}

// ── peg.examples ───────────────────────────────────────────────────────

test "examples: valid, different, and the same for a seed" {
    let E = grammar_of(EXPR)
    let a = peg.examples(E, 25, 3)
    let b = peg.examples(E, 25, 3)
    assert_eq(a, b)
    assert_eq(len(a), 25)
    assert_eq(len(col.unique(a)), 25)
    for x in a { assert_true(E.accepts(x), x) }
    assert_ne(peg.examples(E, 25, 4), a)
    // fuzzing what consumes the trees: every example evaluates
    let V = grammar_with(EXPR, #{ "Num": (m) => 1 })
    for x in a { assert_true(is_ok(V.parse(x)), x) }
    let C = grammar_with(CSV, CSV_ACTIONS)
    for x in peg.examples(grammar_of(CSV), 20, 5) { assert_eq(typeof(value(C, x)), "List") }
}

// ── memoisation ────────────────────────────────────────────────────────

test "a rule tried twice at one place is not parsed twice" {
    // without the cache this grammar is exponential in the nesting
    let text = `
      e = t "+" e / t "-" e / t
      t = f "*" t / f "/" t / f
      f = "(" e ")" / [0-9]
    `
    let G = grammar_of(text)
    let deep = str.repeat("(", 18) + "1" + str.repeat(")", 18)
    let t0 = time.monotonic_ms()
    assert_true(G.accepts(deep + "+" + deep))
    assert_true(time.monotonic_ms() - t0 < 2000)
    let N = unwrap(peg.grammar(text, #{}, #{ "memo": "none" }))
    assert_eq(tree(N, "1+(2*3)"), tree(G, "1+(2*3)"))
}

// ── threads ────────────────────────────────────────────────────────────

test "a grammar built on one thread parses on another" {
    let J = grammar_with(JSON, JSON_ACTIONS)
    let job = spawn value(J, "[1, {\"a\": 2}]")
    assert_eq(task.join(job), Ok([1, #{ "a": 2 }]))
    let rs = par_map(["1", "[2]", "{\"x\": 3}"], (d) => value(J, d))
    assert_eq(rs, [1, [2], #{ "x": 3 }])
}

// ── olang-track's query language ───────────────────────────────────────
//
// The query bar of olang-track (packages/olang-track-query) as a peg
// grammar. What a parse means — fields, values, relative dates, effort,
// errors — is olang-track's own rules (lib/lang.ol, lib/values.ol),
// ported here over the tree instead of its token loop, so the two can be
// compared answer for answer. The goldens below are the canonical form of
// what olang-track's hand-written `parse` answers, hashed: written by
// running that parser over the same queries.
//
// One known difference: inside parentheses, a field with no value before
// a `(` — `(a: (b))` — takes the `(` as its value in the hand-written
// parser, whose paren counting still sees it; the grammar reads it as a
// nested group. Two of the 400 generated queries meet it; they are ()
// in SOUP_GOLDEN.

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

let QG = grammar_of(QUERY)

let OPS = [">=", "<=", ">", "<"]
let SORTABLE = ["enum", "text", "date", "number", "effort"]

fn shift_day(today, n) = match dates.parse(today) { Ok(d) => to_string(d + n), Err(e) => "" }

fn valid_day(s) = {
    let digits = (a, b) => match str.parse_int(str.substring(s, a, b)) { Ok(n) => n, Err(e) => 0 - 1 }
    str.length(s) == 10 && str.substring(s, 4, 5) == "-" && str.substring(s, 7, 8) == "-"
        && digits(0, 4) >= 1 && digits(5, 7) >= 1 && digits(5, 7) <= 12
        && digits(8, 10) >= 1 && digits(8, 10) <= 31
}

fn norm_date(v, today) = {
    let t = str.to_lower(str.trim(v))
    if t == "today" => today
    else if t == "tomorrow" => shift_day(today, 1)
    else if t == "yesterday" => shift_day(today, 0 - 1)
    else if (str.ends_with(t, "d") || str.ends_with(t, "w")) && str.length(t) >= 2
        && (str.starts_with(t, "-") || str.starts_with(t, "+")) =>
        match str.parse_int(str.substring(t, 1, str.length(t) - 1)) {
            Ok(n) => {
                let days = if str.ends_with(t, "w") => n * 7 else => n
                shift_day(today, if str.starts_with(t, "-") => 0 - days else => days)
            },
            Err(e) => t
        }
    else => t
}

fn norm_effort(v) = {
    let s = str.replace(str.to_lower(str.trim(to_string(v))), " ", "")
    let mut minutes = 0
    let mut num = ""
    let mut ok = s != ""
    let mut i = 0
    while i < str.length(s) && ok {
        let c = str.substring(s, i, i + 1)
        if str.contains("0123456789", c) => { num = num + c }
        else if (c == "h" || c == "m") && num != "" => {
            match str.parse_int(num) { Ok(n) => { minutes = minutes + (if c == "h" => n * 60 else => n) }, Err(e) => { ok = false } }
            num = ""
        }
        else => { ok = false }
        i = i + 1
    }
    let total = minutes + (if num == "" => 0 else => match str.parse_int(num) { Ok(n) => n, Err(e) => 0 })
    if !ok => () else => total
}

fn field_of(schema, name) = map_get(schema.fields, name)
fn kind_of(schema, name) = { let f = field_of(schema, name); if f == () => "" else => to_string(f.kind) }
fn today_of(schema) = { let t = schema.today; if t == () => "" else => to_string(t) }
fn resolve(schema, name) = #{ "name": name, "kind": kind_of(schema, name) }

fn sort_keys(schema) = {
    let extras = schema.sort
    let named = if extras == () => [] else => map_keys(extras)
    named + filter(map_keys(schema.fields), (n) => contains(SORTABLE, kind_of(schema, n)) && !contains(named, n))
}

fn default_sort(schema) = {
    let d = schema.default_sort
    let s = if d == () => "-updated" else => to_string(d)
    if str.starts_with(s, "-") => #{ "key": str.substring(s, 1, str.length(s)), "desc": true }
    else => #{ "key": s, "desc": false }
}

fn empty_parse(schema) = {
    let ds = default_sort(schema)
    #{ "clauses": [], "words": [], "not_words": [], "tree": [],
       "sort_key": ds.key, "sort_desc": ds.desc, "top": schema.top ?? 200, "group": "", "errors": [] }
}

fn group_node(parsed) =
    if len(parsed.clauses) == 0 && len(parsed.words) == 0 && len(parsed.not_words) == 0 => ()
    else => #{ "op": "and", "clauses": parsed.clauses, "words": parsed.words, "not_words": parsed.not_words }

fn close_group(parsed) = {
    let node = group_node(parsed)
    parsed
    |> map_set("tree", if node == () => parsed.tree else => parsed.tree + [node])
    |> map_set("clauses", []) |> map_set("words", []) |> map_set("not_words", [])
}

fn tree_of(parsed) = {
    let groups = close_group(parsed).tree
    if len(groups) == 0 => #{ "op": "and", "clauses": [], "words": [], "not_words": [] }
    else if len(groups) == 1 => groups[0]
    else => #{ "op": "or", "kids": groups }
}

fn with_error(parsed, message) = map_set(parsed, "errors", parsed.errors + [message])

fn op_of(value) = { let hit = filter(OPS, (o) => str.starts_with(value, o)); if len(hit) == 0 => "" else => first(hit) }

fn norm_cmp(kind, v, today) =
    if kind == "date" => { let d = norm_date(v, today); if is_ok(dates.parse(d)) => d else => () }
    else if kind == "effort" => norm_effort(v)
    else if kind == "number" => match str.parse_float(v) { Ok(x) => x, Err(e) => () }
    else => ()

fn value_text(v, kind, today) =
    if kind == "date" => norm_date(v, today)
    else if kind == "effort" => match norm_effort(v) { () => v, n => to_string(n) }
    else => v

fn split_values(v, kind, today) = {
    let mut include = []
    let mut exclude = []
    for part in str.split(str.to_lower(v), ",") {
        let p0 = str.trim(part)
        if str.starts_with(p0, "-") && !(kind == "date" && str.ends_with(p0, "d")) => {
            let name = str.substring(p0, 1, str.length(p0))
            if name != "" => { exclude = exclude + [value_text(name, kind, today)] }
        }
        else if p0 != "" => { include = include + [value_text(p0, kind, today)] }
    }
    #{ "include": include, "exclude": exclude }
}

fn add_clause(schema, parsed, field, value) = {
    let kind = kind_of(schema, field)
    let op = op_of(value)
    if field == "sort" => {
        let parts = filter(map(str.split(value, ","), (p) => str.trim(p)), (p) => p != "")
        let specs = map(parts, (p) => {
            let desc = str.starts_with(p, "-")
            #{ "key": str.to_lower(if desc => str.substring(p, 1, str.length(p)) else => p), "desc": desc }
        })
        let bad = filter(specs, (sp) => !contains(sort_keys(schema), sp.key))
        if len(specs) == 0 => with_error(parsed, "sort: missing value")
        else if len(bad) > 0 => with_error(parsed, "unknown sort key: " + first(bad).key)
        else => parsed |> map_set("sort_key", first(specs).key) |> map_set("sort_desc", first(specs).desc) |> map_set("sorts", specs)
    }
    else if field == "top" => match str.parse_int(value) {
        Ok(n) => map_set(parsed, "top", clamp(n, 1, 1000)),
        Err(e) => with_error(parsed, "top wants a number, got: " + value)
    }
    else if field == "group" => {
        let key = str.to_lower(value)
        if contains(schema.groups ?? [], key) => map_set(parsed, "group", key) else => with_error(parsed, "cannot group by: " + key)
    }
    else if kind == "" => with_error(parsed, "unknown field: " + field)
    else if str.contains(value, "..") && contains(["date", "number", "effort"], kind)
        && !str.starts_with(value, "..") && !str.ends_with(value, "..") => {
        let at = str.index_of(value, "..")
        let lo = str.substring(value, 0, at)
        let hi = str.substring(value, at + 2, str.length(value))
        add_clause(schema, add_clause(schema, parsed, field, ">=" + lo), field, "<=" + hi)
    }
    else if op != "" => {
        let raw = str.substring(value, str.length(op), str.length(value))
        if !contains(["date", "number", "effort"], kind) => with_error(parsed, field + ": no comparisons on this field")
        else => {
            let want = norm_cmp(kind, raw, today_of(schema))
            if want == () => with_error(parsed, field + ": cannot read " + raw)
            else => map_set(parsed, "clauses", parsed.clauses + [#{ "field": field, "op": op, "cmp": want, "at": resolve(schema, field) }])
        }
    }
    else => {
        let vs = split_values(value, kind, today_of(schema))
        let bad_days = if kind != "date" => [] else => filter(vs.include + vs.exclude, (d) => !valid_day(d))
        let allowed = { let f = field_of(schema, field); let a = f.values; if typeof(a) == "List" => map(a, (x) => str.to_lower(to_string(x))) else => [] }
        let bad_values = if !(kind == "enum" || kind == "flags") || len(allowed) == 0 => []
            else => filter(vs.include + vs.exclude, (x) => !contains(allowed, x))
        if len(vs.include) == 0 && len(vs.exclude) == 0 => with_error(parsed, field + ": empty value")
        else if len(bad_days) > 0 => with_error(parsed, field + ": not a date: " + first(bad_days))
        else if len(bad_values) > 0 => with_error(parsed, field + ": no such value: " + first(bad_values) + " (one of " + join(allowed, ", ") + ")")
        else => map_set(parsed, "clauses", parsed.clauses + [map_set(map_set(vs, "field", field), "at", resolve(schema, field))])
    }
}

fn add_word(parsed, w) = {
    let t = str.trim(w)
    if t == "" => parsed
    else if str.starts_with(t, "-") && str.length(t) > 1 =>
        map_set(parsed, "not_words", parsed.not_words + [str.to_lower(str.substring(t, 1, str.length(t)))])
    else => map_set(parsed, "words", parsed.words + [str.to_lower(t)])
}

// A token's text as olang-track's tokenizer gives it: a quoted one
// without its quotes, a bare one trimmed.
fn tok_text(nd) =
    if nd.kind == "Quoted" => {
        let t = nd.text
        let end = if str.length(t) >= 2 && str.ends_with(t, "\"") => str.length(t) - 1 else => str.length(t)
        str.substring(t, 1, end)
    }
    else => str.trim(nd.text)

fn pending_field(nd) = str.to_lower(str.substring(nd.key.text, 0, str.length(nd.key.text) - 1))

fn sub_clause(schema, parsed, grp, negate) = {
    let inner = filter(grp.children, (k) => k.kind != "Close" && k.kind != "Not")
    let sub = sem(inner, schema)
    let tail = if grp.shut == () => ["unmatched ("] else => []
    parsed
    |> map_set("clauses", parsed.clauses + [#{ "sub": tree_of(sub), "negate": negate }])
    |> map_set("errors", parsed.errors + sub.errors + tail)
}

// The items of a query (or of a group), in order, as parse_tokens
// reads its tokens.
fn sem(items, schema) = {
    let mut parsed = empty_parse(schema)
    let mut missing = ""
    for it in items {
        let k = it.kind
        if k == "pending" || k == "pending_in" => {
            let field = pending_field(it)
            if it.val == () => { missing = field }
            else => {
                let v = it.val
                // a value the tokenizer would have trimmed away is no token
                if v.kind == "Bare" && str.trim(v.text) == "" => { missing = field }
                else => { parsed = add_clause(schema, parsed, field, tok_text(v)) }
            }
        }
        else if k == "Pair" => {
            let at = str.index_of(it.text, ":")
            parsed = add_clause(schema, parsed, str.to_lower(str.substring(it.text, 0, at)), str.substring(it.text, at + 1, str.length(it.text)))
        }
        else if k == "negated" => { parsed = sub_clause(schema, parsed, it.children[1], true) }
        else if k == "group" => { parsed = sub_clause(schema, parsed, it, false) }
        else if k == "Close" => { parsed = with_error(parsed, "unmatched )") }
        else if k == "Or" => { parsed = close_group(parsed) }
        else if k == "Quoted" || k == "Bare" => { parsed = add_word(parsed, tok_text(it)) }
    }
    if missing != "" => with_error(parsed, missing + ": missing value") else => parsed
}

// The query, parsed against a schema, as olang-track's `parse` answers.
fn track_parse(q, schema) = sem(unwrap(QG.parse(q)).children, schema)

// A parse with what cannot be compared (the resolved readers) left out.
fn canon(parsed) = {
    let strip = (c) => if map_has_key(c, "sub") => #{ "sub": canon_node(c.sub), "negate": c.negate } else => map_remove(c, "at")
    #{ "tree": canon_node(tree_of(parsed)), "sort_key": parsed.sort_key, "sort_desc": parsed.sort_desc,
       "sorts": parsed.sorts, "top": parsed.top, "group": parsed.group, "errors": parsed.errors }
}

fn canon_node(nd) =
    if nd.op == "or" => #{ "op": "or", "kids": map(nd.kids, canon_node) }
    else => #{ "op": "and", "words": nd.words, "not_words": nd.not_words,
        "clauses": map(nd.clauses, (c) => if map_has_key(c, "sub") => #{ "sub": canon_node(c.sub), "negate": c.negate } else => map_remove(c, "at")) }


let TRACK_SCHEMA = #{
    "fields": #{
        "state": #{ "kind": "enum", "values": ["open", "doing", "done"] },
        "tag": #{ "kind": "list", "values": ["ui", "server"] },
        "owner": #{ "kind": "text" },
        "due": #{ "kind": "date" },
        "spent": #{ "kind": "effort" },
        "points": #{ "kind": "number" },
        "is": #{ "kind": "flags", "values": ["late", "big"] }
    },
    "sort": #{ "num": () },
    "groups": ["state", "owner"],
    "default_sort": "-updated", "top": 200, "today": "2026-09-10"
}
let PIECES = ["state:", "tag:", "owner:", "due:", "spent:", "points:", "is:", "sort:", "top:", "group:", "nope:",
    "open", "done", "-done", "ui", "ui,server", "-ui", "late", "num", "-num", "updated", "state", "owner",
    "<", ">", "<=", ">=", "<2026-09-05", ">-3d", "+7d", "today", "tomorrow", "2026-13-45", "soon", "1h30m", "90",
    "5", "-5", "0", "1000000", "OR", "or", "(", ")", "((", "))", "\"", "\"quoted phrase\"", "\"open", "-", ",",
    ":", "::", "word", "-word", "\\", "🙂", "state:open", "top:abc", "sort:", "group: state", "  ", "\t", "not", "NOT (", "a:b:", "x:"]
fn lcg(seed) = (seed * 1103515245 + 12345) % 2147483648
fn soup(seed_in, n) = {
    let mut seed = seed_in
    let mut out = []
    let mut i = 0
    while i < n {
        seed = lcg(seed)
        out = out + [PIECES[seed % len(PIECES)]]
        i = i + 1
    }
    join(out, if seed % 3 == 0 => "" else => " ")
}

fn qhash(p) = str.substring(crypto.sha256(show(canon(p))), 0, 10)

let SOUP_GOLDEN = [
    "de69b6ceab", "8e25056991", "5d452b05a9", "311c3d5723", "1d44172ffc", "e5ef33c6fe", "8d483ca00a", "dc67f2f28e",
    "bb3df3c101", "742b1429ee", "20451e4abd", "dc494c41df", "a93ddcea7f", "2107f86078", "8422532b4a", "f180829115",
    "0eee17b375", "cd0d9b4631", "31b3faebfb", "c3e4e1f6e3", "a03ab50781", "6bc3da7b11", "940ed83c6e", "ec32541457",
    "b5a4950a33", "4b3b647a47", "51bf343599", "88af2bbb3e", "eade1afbce", "e7c8950da3", "33a0e4cb08", "ab9429bcc6",
    "3ff8c578c3", "7413727f40", "ad6ec90e39", "e2366b1d67", "8e25056991", "22c387a9da", "e307235e3c", "693b1b114a",
    "18ff642941", "c1db6f7389", "104718da8d", "67401ae74f", "a194334c8c", "b3d75ab4b5", "eabb207ce7", "9f4acca83f",
    "4183362a04", "51335ea4f9", "e9da16f005", "c025be32f3", "54642b0ad8", "d53dab37a3", "d5cd707438", "91e133f804",
    "a380fec807", "cde7bf5237", "f8d5f3e965", "f522318a1e", "53295e8d4a", "c9f02c7b68", "67036663e2", "6bace88e34",
    "82c745e325", "7edcf174f6", "db8e23a264", "19461ed434", "7fa38b4f6a", "4e2b075713", "34a23e5053", "a5846e8acd",
    "a0c87d3186", "0964969a3e", "89dc869884", "263c0e66ca", "e64f3d9917", "193d8f1366", "6314d44ac1", "43f39ac6ce",
    "c4e37326ec", "9b9b334f4e", "fe46d58a1b", "b4bd25d81a", "90b66d2576", "3ea86288a0", "71c9624e94", "bd7c741559",
    "5207557b38", "a908edb55d", "d3e35a1eb1", "ae12c2e38e", "fad1275847", "e67426b0ca", "376fbc310d", "e242f9c70f",
    "f0d89e3b49", "6b3b04b48f", "388e30cefe", "13cf692905", "128627c9da", "ff0d4da1d6", "d0d717ffc1", "759f144e95",
    "ffe4b02807", "fcaf5af4eb", "d5593af4a6", "4ca1e7348e", "ea9ccdb636", "90f54b1a2e", "3d590663c8", "cd1d3eb9ff",
    "2712096d41", "371846ece5", "148ef018b9", "a3978dfaea", "28e49479ec", "aca3c68c3f", "e6e4773203", "bc2c8d690d",
    "5520c8bceb", "c3060f4bee", "42873f1966", "9da9c2cb38", "5c389eb406", "843e75fe4f", "12ad9be677", "3b8aa10481",
    "15f88cab72", "d0e74bca6d", "4a5c4bd364", "12fe3bba7f", "722e383701", "484c69b83b", "51d8039120", "1dcb2879c5",
    "c4f54e08b4", "01818cc968", "e11e4cc868", "8b423d52c6", "4da3fc8794", "68d684a349", "e2afc2f54f", "e8b4a70545",
    "c3533e166e", "d4ce6d8768", "458852205a", "754d115e26", "efc82a16ee", "d4bcf2ba12", "d2e814b654", "1242c34573",
    "4d3f8ce11f", "66a8d42198", "a4079dcbe9", "3d3494a4e0", "8f0021b347", "d28a299cf5", "c1c0373830", "227b704cac",
    "d2b8e39480", "1481b45368", "7e05b6002a", "47e20cb49d", "cfd87b0c9c", "e5f9c17eaf", "934e6d0d13", "9f09211b8f",
    "5b45f7b7a0", "6f447cd332", "b6a903ed51", "59c2959a37", "a0c87d3186", "1e51ebf15d", "423b861b68", "a0d0067b41",
    "1c136b0850", "f9a29a3bee", "3667bb154a", (), "0d45141a97", "28ed64bea1", "a25a3fb6fd", "9c5dfd9803",
    "2c5adf5fde", "355fed2025", "1e715425e7", "a938bc2a95", "686c5cedf8", "8eda63969f", "83d898e4a2", "5c6e93b5e5",
    "13b474b5d3", "17e0220828", "0c11b643b9", "0796b0bfee", "2501065df1", "0e83cb358c", "dcdecfad90", "159c7736cc",
    "49b6c3a345", "59191f98e1", "f4c55e6c15", "85a631334e", "b68fd85ec8", "cea07ef3cb", "1e244e9fe1", "d38f6db325",
    "20f7a5ef8c", "cd58c5580c", "e525f3bfe6", "f2e98b9e22", "d03ab5bfd9", "43560f9855", "d92fa22b10", "2eb4765635",
    "96331ec7e1", "ab58cfc40c", "6828392b31", "93ffc81f12", "98369808ae", "caa82f8ba9", "915f5d3504", "a449b71b81",
    "e75a7d3bff", "d85850c54f", "c8eb35e3be", "3d12100d10", "d6df4544f3", "7fc7c54db8", "cbaaf57156", "1715b47ba2",
    "fd6aca5edc", "2932cb5eb4", "ff0d4da1d6", "ff0d4da1d6", "b03ec69eea", "769a342aba", "e71f0183ad", "95c85b4d8e",
    "068c2b7cc5", "99d442f7d1", "3d825e013e", "2a2075b627", "158c5377c7", "6e7a5ceadb", "232f8a5b8d", "e1fe61959a",
    "b8bcffa114", "c1a6f34136", "ce1c3a5ef1", "81d9791918", "cec6832b6e", "c4de3dcabd", "bb3892a0a7", "0e06c0307b",
    "44798ba4b8", "49ac2f0fb7", "ce471f77b9", "13dcbea140", "f184a3cbf4", "cd46bcae8b", "8e60eb5ec2", "c61f83dcd2",
    "a54f669130", "aa62b36fd7", "5d84df03cd", "0c6505e5af", "a960a41efb", "65e0850e9a", "b613214787", "017cc5bdbb",
    "183a518d65", "48bef4a6cb", "ea955a6f4f", "24097aa0ae", "6411d2c9dd", "5e0c45f39b", "f3dcfa4c65", "6eb4394ed4",
    "9b4e42980a", "22c95326a6", "0fe5c1f59f", "9ea609bd05", "7939748bb1", "4f32a027a8", "d95996f7ab", "3d590663c8",
    "dcdecfad90", "19ffb5126f", "6688defcaf", "6236b138ac", "f37ff578dc", "80e286bf48", "6bbb4983ed", "6e5c91998e",
    "07612ce0b4", "49951d76a2", "6bb71aae81", "d04a319caf", "c54dfec2d3", "a188f663ac", "39e2202095", "fa1cf57c99",
    "63cb1a291d", "84de23dade", "bfbd84faac", "54ccca8140", "d2b95fbd44", "44f33d333e", "cd63fd3574", "7e98b8624e",
    "3bb53957ed", "6af5f6f796", "c1e2caa91c", "43da48fee7", "b199de9791", "81f06f97ab", "ba8f253389", "d42fa9f54b",
    "de368ee2f3", "d3117cbc79", "efbc662a12", "221f76ab0f", "76ef84fa26", "7f2ee30d0e", "a3db492415", "babd70f920",
    "99e2bec36a", "b3548d1d96", "f4b8eaf891", "c5302c2732", "4fed6685ca", "f9f039b00d", "79653df473", "299fe45bd2",
    "fa1b852930", "cfac9c9c1f", "7af3eb510d", "98350c8159", "442a9e29e0", "d09d691a32", "b29777aa48", "8e107608bb",
    "fbff7417dc", "3cb43d20ac", "e40b9f41d1", "dd3fdd25d4", "50b7c3651f", "90e084d176", "289d75d9bf", "41c99f12b7",
    "cec6832b6e", "16600e17d8", "6a3f8b485e", "c41707ac93", "362577fd57", "0086bfee50", "1921433562", "ed8c06af00",
    "dcdecfad90", "98ed6a763e", "2c9b7f82ce", "7cb8c05fdb", "2116ecdbf4", "136bff41c6", "6ee0548af3", "756d1dbae0",
    "e9e04f2cad", "597a05ef25", "e0d14ff5a7", "f91539a0db", "891380d124", "e433564c72", "7379c0f9a1", "0900c74f57",
    "b77442634a", "8b18b389eb", "dcdecfad90", "73255dcc96", "92d0ec8ca3", "53cce5a338", "494bff8788", "761045e7a0",
    (), "ac45c7a0cd", "6b1388a6d0", "36411879a0", "9b9b334f4e", "5828be90d3", "df9dd30304", "b77d3928d5",
    "65370404b5", "e56b4a8e41", "c62e7227e7", "d5f58bbfe4", "dcdecfad90", "a69ecee8c8", "6623efeb5f", "e464fff010",
]

// (query, hash of olang-track's answer, its errors)
let CURATED = [
    ("state: open sort: num", "35711042f4", []),
    ("state: open,done sort: num", "40866bb0c2", []),
    ("state: -done sort: num", "4d6db5ceed", []),
    ("tag: -ui", "111d125a32", []),
    ("board", "4ed3b4c672", []),
    ("\"the router\"", "69a0d48fc9", []),
    ("ui -dark", "1c64d7438d", []),
    ("state:open sort:num", "35711042f4", []),
    ("due: <today sort: num", "84ea07a3b4", []),
    ("due: <+7d", "3910d8bfc8", []),
    ("due: >=2026-09-05", "ba1000fca1", []),
    ("spent: >1h", "4014118adb", []),
    ("points: <=1", "2438174680", []),
    ("is: late", "c011ad6e4f", []),
    ("sort: -spent top: 1", "b1e5ee3428", []),
    ("", "dcdecfad90", []),
    ("wizard: x state: >open due: <soon state:", "c1e8daecab", ["unknown field: wizard", "state: no comparisons on this field", "due: cannot read soon", "state: missing value"]),
    ("top: 2", "8f828441d6", []),
    ("state: done OR points: >=8 sort: num", "b62fbfc5a9", []),
    ("(state: open tag: polish) OR state: doing sort: num", "781747c6cc", []),
    ("state: open (tag: polish OR tag: ui) sort: num", "8e70f4cdd6", []),
    ("(state: open OR wizard: x", "d43eaa8cb9", ["unknown field: wizard", "unmatched ("]),
    ("state: open)", "0c1e6f7c61", ["unmatched )"]),
    ("group: state", "49951d76a2", []),
    ("state: open group: Owner", "325c9e489e", []),
    ("group: points", "59f78bfc19", ["cannot group by: points"]),
    ("due: 2026-09-01..2026-09-12 sort: num", "ed9cda2b6b", []),
    ("points: 1..3", "af40b31812", []),
    ("due: ..2026-09-12", "143555b626", ["due: not a date: ..2026-09-12"]),
    ("due: >-2w", "687abe73d2", []),
    ("sort: state,-num", "ac42cbfbc1", []),
    ("sort: state,nope", "160ef723ab", ["unknown sort key: nope"]),
    ("NOT (state: open)", "e6c89f61f6", []),
    ("not (state: open OR state: done)", "fc385d9805", []),
    ("not", "ff0d4da1d6", []),
    ("state: opne", "6322d3af5b", ["state: no such value: opne (one of open, doing, done)"]),
    ("is: open", "e316a89299", ["is: no such value: open (one of late, big)"]),
    ("spent: 1h30m", "c00fe3c126", []),
    ("spent: -1h30m state: -done", "0d637397bf", []),
    ("state: open\tpoints: >1", "8ff125a57c", []),
    ("spent: >99999999999999999999h", "0b97535388", ["spent: cannot read 99999999999999999999h"]),
    ("owner: \"Ann Lee\" kanban", "b9904f5624", []),
    ("a:b:c d::e : f", "f4cd8fce4c", ["unknown field: a", "unknown field: d"]),
    ("((state: open)", "911ad5d101", ["unmatched ("]),
    ("tag:ui,-server -\"x y\" or", "c42e9d98a3", []),
    ("due: tomorrow is: -big", "5f0778152b", []),
]

test "olang-track queries: the same answers as its parser, curated" {
    for (q, h, errors) in CURATED {
        let p = track_parse(q, TRACK_SCHEMA)
        assert_eq(p.errors, errors, q)
        assert_eq(qhash(p), h, q)
    }
}

test "olang-track queries: the same answers as its parser, 400 generated" {
    let mut checked = 0
    for k in 0..len(SOUP_GOLDEN) {
        let want = SOUP_GOLDEN[k]
        if want != () => {
            let q = soup(k * 7919 + 1, 1 + k % 9)
            assert_eq(qhash(track_parse(q, TRACK_SCHEMA)), want, q)
            checked = checked + 1
        }
    }
    assert_eq(checked, 398)
}

test "olang-track queries: the grammar takes any text, and its errors are its parser's" {
    for k in 0..200 {
        let q = soup(k * 104729 + 7, 1 + k % 12)
        assert_true(QG.accepts(q), q)
    }
    let p = track_parse("project: OL is: overdue sort: -updated", #{ "fields": #{ "project": #{ "kind": "text" },
        "is": #{ "kind": "flags", "values": ["overdue"] } }, "sort": #{ "updated": () } })
    assert_eq(p.errors, [])
    assert_eq(p.sort_key, "updated")
    assert_eq(p.sort_desc, true)
    assert_eq(map(p.clauses, (c) => (c.field, c.include)), [("project", ["ol"]), ("is", ["overdue"])])
}
