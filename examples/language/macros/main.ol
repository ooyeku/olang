// macros — extending olang in olang (docs/macros.md).
//
// A `meta fn` runs at expansion time: it receives the source text of its
// arguments and returns source text, which the real parser validates and
// splices in place of the `@` site before the interpreter or any tier
// sees the program. Everything here is userland — none of these are
// language features, and that is the point. The language stays frozen;
// the ecosystem grows it.
//
//   olang main.ol            run it
//   olang expand main.ol     see the program the runtime actually gets

// ── control flow the language "doesn't have" ────────────────────────
meta fn unless(cond, body) = `if !(${cond}) => ${body} else => ()`

// ── comptime: evaluate at expansion, ship a literal ─────────────────
// The whole of Zig-style comptime in one line: run the expression in
// the pure expansion sandbox, splice its result as source.
meta fn bake(expr) = meta.lit(unwrap(meta.eval(expr)))

// ── zero-cost debugging: print source and value, pass it through ────
meta fn dbg(expr) = `{ let v = ${expr}; println("${expr} = " + show(v)); v }`

// ── a derive: serialization from a type's own field list ────────────
// The macro parses the declaration it decorates (the Open AST), reads
// the field names, and emits the declaration plus a to_json function.
meta fn json(decl) = {
    let t = head(unwrap(meta.parse(decl)))
    let tname = str.to_lower(map_get(t, "name"))
    let fields = map_get(t, "fields") |> map((f) => map_get(f, "name"))
    let pairs = fields |> map((f) => `"${f}": v.${f}`) |> join(", ")
    `${decl}
fn ${tname}_to_json(v) = unwrap(json.stringify(#{ ${pairs} }))`
}

// ── the program ─────────────────────────────────────────────────────

println("═══ macros: the language, extended from userland ═══")

// @bake computes the table once, at expansion. `olang expand` shows the
// literal list in the source; the runtime never runs the map.
let squares = @bake(range(0, 10) |> map((n) => n * n))
println(`baked squares: ${squares}`)

let x = 3
@unless(x > 5, println(`${x} is small (said via @unless)`))

let answer = @dbg(6 * 7)
println(`the answer is ${answer}`)

@json
type Reading = struct { station: String, kwh: Float }

let r = Reading { station: "north", kwh: 21.5 }
println(`as json: ${reading_to_json(r)}`)

// ── self-checks ─────────────────────────────────────────────────────
test "baked values equal their runtime computation" {
    assert_eq(@bake(range(0, 10) |> map((n) => n * n)),
              range(0, 10) |> map((n) => n * n))
    assert_eq(@bake(2 + 3), 5)
}

test "unless runs the body exactly when the condition is false" {
    let mut hits = 0
    @unless(true, { hits = hits + 100 })
    @unless(false, { hits = hits + 1 })
    assert_eq(hits, 1)
}

test "the derive serializes every declared field" {
    let j = reading_to_json(Reading { station: "s", kwh: 1.5 })
    assert_eq(j, "{\"kwh\":1.5,\"station\":\"s\"}")
}
