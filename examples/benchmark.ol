let start = dates.timestamp(dates.now())
let n = range(10000)

// Note: Comparison operators in lambda expressions require block syntax: { expr } instead of expr
let data = n |> map((val) => val * math.pow(val, 2)) |> filter((val) => { val % 2 == 0 }) |> sum()

println(data)

// Highly optimized pipeline with fusion
let result = n 
    |> filter((val) => { val > 50 })
    |> map((val) => val * val) 
    |> take(100)
    |> println()

let r = range(100)          
for i in r |> map((val) => val * val) {
    let take_count = random.randint(0, 100)
    let result = r |> map((val) => (val * val) / 0.2) |> reverse() |> take(take_count) |> sum()
    let z = len(r) - take_count 
    let advaned_comp = i |> math.pow(z) |> math.sqrt() |> math.sqrt() |> math.sin() |> math.cos() |> math.round()

}

let end = dates.timestamp(dates.now())
println(`time: ${end - start}`)