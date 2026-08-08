let n = range(100)

let v = random.choice(n)

for i in range(500) {
    let result1 = n |> map((x) => x * math.pow(x, v)) |> filter((x) => { x % 2 == 0 }) |> sum()
    println(result1)

    let result2 = n |> map((x) => x * math.pow(x, 2)) |> filter((x) => { x % 2 == 0 }) |> sum()
    println(result2)

    // Simplified mathematical computation
    let simple_result = 1..100
        |> map((x) => x * 2)
        |> filter((x) => { x > 10 })
        |> sum()

    println(simple_result)

    // Simplified prime-like operation
    let simple_prime = 2..50
        |> filter((x) => { x % 2 != 0 || x == 2 })
        |> map((x) => x * x)
        |> sum()

    println(simple_prime)

    // Simplified computation
    let simple_calc = 1..50
        |> map((x) => x * 2)
        |> filter((x) => { x > 20 })
        |> sum()

    println(simple_calc)
}
