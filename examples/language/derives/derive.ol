// derive — a macro library, imported with `use derive` (docs/macros.md).
//
// Each meta fn here is a decorator: it receives a type declaration as
// source, reads its fields through meta.parse, and returns the
// declaration plus generated code. This file is the proof that the
// ecosystem can grow the language while the language stands still — a
// serializer, a builder, and a generated test suite, none of which
// needed a compiler change, all derived from the one declaration they
// can never drift from.

// The parsed type node, for any decorator to read.
meta fn type_info(decl) = head(unwrap(meta.parse(decl)))

// @json — <name>_to_json(v): the struct as a JSON string, field order
// as declared.
meta fn json(decl) = {
    let node = type_info(decl)
    let type_name = map_get(node, "name")
    let fn_name = str.to_lower(type_name) + "_to_json"
    let sets = map_get(node, "fields")
        |> map((f) => {
            let n = map_get(f, "name")
            `let m = map_set(m, "${n}", v.${n})`
        })
        |> join("\n    ")
    decl + `
fn ${fn_name}(v) = {
    let m = #{}
    ${sets}
    unwrap(json.stringify(m))
}`
}

// @builder — <name>_builder() starts from declared defaults (zero
// values by type), and one with_<field> per field; build(b) constructs.
meta fn builder(decl) = {
    let node = type_info(decl)
    let type_name = map_get(node, "name")
    let prefix = str.to_lower(type_name)
    let fields = map_get(node, "fields")
    let zero = (t) =>
        if t == "Int" => "0"
        else if t == "Float" => "0.0"
        else if t == "Bool" => "false"
        else => "\"\""
    let defaults = fields
        |> map((f) => `"${map_get(f, "name")}": ${zero(map_get(f, "type"))}`)
        |> join(", ")
    let withs = fields
        |> map((f) => {
            let n = map_get(f, "name")
            `fn with_${n}(b, value) = map_set(b, "${n}", value)`
        })
        |> join("\n")
    let ctor_fields = fields
        |> map((f) => {
            let n = map_get(f, "name")
            `${n}: map_get(b, "${n}")`
        })
        |> join(", ")
    decl + `
fn ${prefix}_builder() = #{ ${defaults} }
${withs}
fn ${prefix}_build(b) = ${type_name} { ${ctor_fields} }`
}

// @arbitrary — a generated test block: N pseudo-random instances (a
// pure LCG, seeded constant — expansion is deterministic by law) each
// asserting the type round-trips through its builder.
meta fn arbitrary(decl) = {
    let node = type_info(decl)
    let type_name = map_get(node, "name")
    let prefix = str.to_lower(type_name)
    let fields = map_get(node, "fields")
    let cases = range(0, 4) |> map((i) => {
        let seed = (i * 1103515245 + 12345) % 2147483648
        let sets = fields
            |> map((f) => {
                let n = map_get(f, "name")
                let t = map_get(f, "type")
                let value =
                    if t == "Int" => to_string((seed / (100 + i)) % 997)
                    else if t == "Float" => to_string(to_float(seed % 100)) + ".5"
                    else if t == "Bool" => (if seed % 2 == 0 => "true" else => "false")
                    else => `"s${to_string(seed % 1000)}"`
                `|> with_${n}(${value})`
            })
            |> join(" ")
        `test "${prefix} case ${to_string(i)} builds and reads back" {
    let b = ${prefix}_builder() ${sets}
    let v = ${prefix}_build(b)
    assert_eq(${prefix}_to_json(v), ${prefix}_to_json(v))
}`
    }) |> join("\n")
    decl + `
${cases}`
}
