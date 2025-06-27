let n = range(100)

let v = random.choice(n)

for number in n { 
    n |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum() |> println()
    let data = 1..100 |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum()
    n |> map((x) => x * math.pow(x, random.choice(n))) |> filter((x) => x % 2 == 0) |> sum() |> println()
    let data = 1..100 |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum()
    n |> map((x) => x * math.pow(x, 2)) |> filter((x) => x % 2 == 0) |> sum() |> println()
    let data = 1..100 |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum()

}