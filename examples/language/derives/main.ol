// derives — declaration-driven code generation, from an imported macro
// library (docs/macros.md). One `type` declaration; the serializer, the
// builder, and a generated test suite all derive from it and can never
// drift from it. Delete a field and every derived artifact follows.
//
//   olang main.ol          run the demo
//   olang test .           run the tests @arbitrary generated
//   olang expand main.ol   see exactly what the derives produced

use derive

@arbitrary @builder @json
type Contact = struct { name: String, email: String, age: Int }

// The builder the decorator generated, used like hand-written code.
let ada = contact_builder()
    |> with_name("Ada Lovelace")
    |> with_email("ada@example.com")
    |> with_age(36)
    |> contact_build

println("── one contact, three derived artifacts ──")
println(`struct:  ${show(ada.name)} <${ada.email}>, ${ada.age}`)
println(`json:    ${contact_to_json(ada)}`)

// Decorator order reads outward: @json (nearest) runs first on the bare
// declaration, @builder wraps that, @arbitrary wraps the whole — so its
// generated tests land after every function they call.

// A second type shares the same derives — the library is generic over
// whatever declaration it is handed.
@json
type Reading = struct { station: String, celsius: Float }

let r = Reading { station: "north", celsius: 21.5 }
println(`reading: ${reading_to_json(r)}`)

test "the serializer tracks the declaration" {
    let j = contact_to_json(ada)
    assert_true(str.contains(j, "\"name\""))
    assert_true(str.contains(j, "\"age\""))
    assert_true(str.contains(j, "ada@example.com"))
}
