// Import from the `geometry` package by name — resolved through olang.toml.
use geometry { point, distance, circle, rectangle, triangle, area, describe }

println("═══ using the geometry package ═══")

// Points and distance
let a = point(0.0, 0.0)
let b = point(3.0, 4.0)
println(`distance (0,0)->(3,4): ${distance(a, b)}`)

// Shapes and areas
let shapes = [circle(2.0), rectangle(3.0, 5.0), triangle(6.0, 4.0)]
for shape in shapes {
    println("  " + describe(shape))
}

// Compose package functions with local pipelines
let total = shapes |> map((s) => area(s)) |> fold(0.0, (acc, x) => acc + x)
println(`total area: ${total}`)

println("═══ done ═══")
