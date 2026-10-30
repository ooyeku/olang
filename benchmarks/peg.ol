// The `peg` budget (plans/PEG.md, G2): a JSON grammar against the
// built-in `json.parse`, and a query bar like olang-track's, on the
// same machine in the same run. `olang benchmarks/peg.ol`.
//
// Four numbers for JSON over one ~150 KB document: `json.parse` (native
// Rust), `accepts` (the grammar recognising the text, no tree), `parse`
// (the tree of nodes with kinds, texts, spans and labels) and `parse`
// with actions (the same values `json.parse` gives, checked equal).
// The query bar is olang-track's grammar over four typical queries, as
// microseconds a query; olang-track's own hand-written parser is in its
// repository (packages/olang-track-query) — its figure on the machine
// that wrote this file is quoted below for comparison, with how it was
// measured.
//
//   json.parse          1.6 ms
//   peg accepts         36 ms      (22×)
//   peg tree            163 ms     (100×)
//   peg values          184 ms     (115×, equal to json.parse's)
//   query, peg tree     83 µs      olang-track's parse: 51 µs (1.6×)
//   query, peg tree + olang-track's meaning of it: 146 µs (2.9×)
//
// (Apple M-series, olang 1.0.0-rc.5, best of several runs.)

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

// The best of `reps` runs of `f`, in milliseconds.
fn best_ms(f, reps) = {
    let mut best = 1000000
    for i in 0..reps {
        let t = time.monotonic_ms()
        f()
        let d = time.monotonic_ms() - t
        if d < best => { best = d }
    }
    best
}

fn times(a, b) = if b == 0 => "-" else => str.fixed(to_float(a) / to_float(b), 1) + "×"

let J = unwrap(peg.grammar(JSON))
let JV = unwrap(peg.grammar(JSON, ACTIONS))
let record = `{"id": 12345, "name": "widget \"pro\"", "tags": ["a", "b", "c"], "price": 12.5e2, "ok": true, "none": null, "nested": {"x": [1, 2, 3, {"y": -4.25}]}}`
let doc = "[" + join(map(range(0, 1000), (i) => record), ",\n") + "]"
if unwrap(JV.parse(doc)) != unwrap(json.parse(doc)) => raise("the grammar's values differ from json.parse's")

let native = to_float(best_ms(() => { for i in 0..10 { json.parse(doc) } }, 5)) / 10.0
let accept = best_ms(() => J.accepts(doc), 3)
let tree = best_ms(() => J.parse(doc), 3)
let values = best_ms(() => JV.parse(doc), 3)
println(`JSON, ${len(doc)} characters`)
println(`  json.parse   ${str.fixed(native, 1)} ms`)
println(`  peg accepts  ${accept} ms  (${times(accept, native)})`)
println(`  peg tree     ${tree} ms  (${times(tree, native)})`)
println(`  peg values   ${values} ms  (${times(values, native)})`)

let Q = unwrap(peg.grammar(QUERY))
let per_query = (f) => to_float(best_ms(() => { for i in 0..200 { for q in QUERIES { f(q) } } }, 5)) * 1000.0 / 800.0
println("query bar, µs a query")
println(`  peg accepts  ${str.fixed(per_query((q) => Q.accepts(q)), 1)}`)
println(`  peg tree     ${str.fixed(per_query((q) => Q.parse(q)), 1)}`)
