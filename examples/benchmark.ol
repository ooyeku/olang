let n = range(1000)

// Note: Comparison operators in lambda expressions require block syntax: { expr } instead of expr
let data = n |> map((val) => val * math.pow(val, 2)) |> filter((val) => { val % 2 == 0 }) |> sum()

println(data)

// Highly optimized pipeline with fusion
let result = n 
    |> filter((val) => { val > 50 })
    |> map((val) => val * val) 
    |> take(10)
    |> println()
