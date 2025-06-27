let n = range(1000000)

let data = n |> map((x) => x * math.pow(x, 2)) |> filter((x) => x % 2 == 0) |> sum()

println(data)

// Highly optimized pipeline with fusion
let result = n 
    |> filter((x) => x > 1000)
    |> map((x) => x * x) 
    |> take(10)
    |> println()
