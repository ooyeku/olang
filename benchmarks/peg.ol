// The `peg` budget (plans/PEG.md, G2; plans/PEG-NATIVE.md): a JSON
// grammar against the built-in `json.parse`, and olang-track's query
// bar against its hand-written parser, on the same machine in the same
// run. `olang benchmarks/peg.ol`.
//
// JSON is one ~150 KB document (1,000 records, 52,000 tree nodes):
// `json.parse` (native Rust); `accepts` (the grammar recognising the
// text, no tree); `parse` (the tree, every node a map with its kind,
// text, span, children and labels); `parse` with actions (the values
// `json.parse` gives, checked equal). The query bar is olang-track's
// grammar over four typical queries, in microseconds a query. Each is
// measured on the native matcher and on the olang engine it replaced
// (`#{ "engine": "olang" }`), and set against its budget.
//
// olang-track's hand-written parser (tokenizer and the meaning of the
// clauses, packages/olang-track-query) is in its own repository; its
// figure, measured on the machine that wrote this file in the same
// harness, is HAND below.
//
//                    olang engine (rc.6)    native (rc.6)     budget
//   json.parse        1.6 ms                 1.6 ms
//   peg accepts       31 ms   (19×)          1.2 ms  (0.7×)    2×
//   peg tree          158 ms  (99×)          18 ms   (11.5×)   3× (missed)
//   peg values        178 ms  (112×)         44 ms   (27×)
//   query, accepts    27 µs                  1.3 µs
//   query, tree       77 µs   (1.5×)         8.5 µs  (0.17×)   1× HAND
//   query, tree + olang-track's meaning of it (peg_test.ol's port):
//                     139 µs  (2.8×)         51 µs   (1.0×)
//   HAND (olang-track's own parse): 50 µs
//
// (Apple M-series, olang 1.0.0-rc.6 release build, best of several
// runs; a tree's time counts freeing it.) The tree misses its budget:
// the matching is about a millisecond, and the rest is making 52,000
// nodes as olang maps — about fifteen allocations a node: a map and its
// span with eight String keys, the text, the children — and freeing
// them. plans/PEG-NATIVE.md has the profile.

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

fn number(t) =
    if str.contains(t, ".") || str.contains(t, "e") || str.contains(t, "E") => unwrap(str.parse_float(t))
    else => unwrap(str.parse_int(t))

let ACTIONS = #{
    "json": (m) => m.children[0],
    "value": (m) => m.children[0],
    "object": (m) => fold(m.children, #{}, (acc, kv) => map_set(acc, kv[0], kv[1])),
    "member": (m) => (m.key, m.val),
    "array": (m) => m.children,
    "String": (m) => unwrap(json.parse(m.text)),
    "Number": (m) => number(m.text),
    "True": (m) => true,
    "False": (m) => false,
    "Null": (m) => ()
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

let QUERIES = ["project: OL is: overdue sort: -updated",
    "state: open,doing tag: -ui due: <+7d \"release notes\" -draft sort: due top: 20",
    "(state: open tag: polish) OR state: doing sort: points",
    "owner: ann spent: >1h30m points: >=3 NOT (tag: server OR is: late) kanban board"]

// The best of `reps` runs of `f`, in milliseconds a call of it, where
// a run calls it `inner` times. (`time.monotonic()` is milliseconds
// with a fraction. A value a call answered is freed during the next, so
// a run of more than one call counts freeing it too.)
fn best_ms(f, inner, reps) = {
    let mut best = 1000000.0
    for i in 0..reps {
        let t = time.monotonic()
        for j in 0..inner { f() }
        let d = (time.monotonic() - t) / to_float(inner)
        if d < best => { best = d }
    }
    best
}

let HAND = 50.0

fn times(a, b) = if b == 0.0 => "-" else => str.fixed(a / b, 1) + "×"
fn ms(x) = if x < 10.0 => str.fixed(x, 1) else => str.fixed(x, 0)
fn row(name, x, base, budget) =
    println(`  ${name}${ms(x)} ms  (${times(x, base)}${if budget == "" => "" else => ", budget " + budget})`)

let record = `{"id": 12345, "name": "widget \"pro\"", "tags": ["a", "b", "c"], "price": 12.5e2, "ok": true, "none": null, "nested": {"x": [1, 2, 3, {"y": -4.25}]}}`
let doc = "[" + join(map(range(0, 1000), (i) => record), ",\n") + "]"
let native = best_ms(() => json.parse(doc), 10, 5)
println(`JSON, ${len(doc)} characters`)
println(`  json.parse            ${ms(native)} ms`)
for engine in ["native", "olang"] {
    let J = unwrap(peg.grammar(JSON, #{}, #{ "engine": engine }))
    let JV = unwrap(peg.grammar(JSON, ACTIONS, #{ "engine": engine }))
    if unwrap(JV.parse(doc)) != unwrap(json.parse(doc)) => raise("the grammar's values differ from json.parse's")
    // (two calls a run at least, so the tree freed is in the time)
    let n = if engine == "native" => 4 else => 2
    let label = if engine == "native" => "" else => " (olang engine)"
    row(`peg accepts${label}  `, best_ms(() => J.accepts(doc), n, 5), native, if engine == "native" => "2×" else => "")
    row(`peg tree${label}     `, best_ms(() => J.parse(doc), n, 5), native, if engine == "native" => "3×" else => "")
    row(`peg values${label}   `, best_ms(() => JV.parse(doc), n, 5), native, "")
}

println(`query bar, µs a query (olang-track's hand-written parser: ${str.fixed(HAND, 0)} µs, measured beside it)`)
for engine in ["native", "olang"] {
    let Q = unwrap(peg.grammar(QUERY, #{}, #{ "engine": engine }))
    let per_query = (f) => best_ms(() => { for q in QUERIES { f(q) } }, 200, 5) * 1000.0 / 4.0
    let label = if engine == "native" => "" else => " (olang engine)"
    let tree = per_query((q) => Q.parse(q))
    println(`  peg accepts${label}  ${str.fixed(per_query((q) => Q.accepts(q)), 1)}`)
    println(`  peg tree${label}     ${str.fixed(tree, 1)}  (${times(tree, HAND)} the hand-written parser${if engine == "native" => ", budget 1×" else => ""})`)
}
