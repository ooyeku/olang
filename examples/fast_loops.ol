

share fn fast_loop() = {
    let data = 1..100


let n = 1..100

for numbers in n {
    let v = random.choice(n)
    data |> map((x) => x * math.cos(x)) |> map((x) => x * v) |> sum() |> println()
}
}

fast_loop()

fn main() = {
    fast_loop()
}

main()