let n = range(10000)

let v = random.choice(n)

for number in n { 
    n |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum() |> println()
    let data = 1..100 |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum()
    n |> map((x) => x * math.pow(x, random.choice(n))) |> filter((x) => x % 2 == 0) |> sum() |> println()
    let data = 1..100 |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum()
    n |> map((x) => x * math.pow(x, 2)) |> filter((x) => x % 2 == 0) |> sum() |> println()
    let data = 1..100 |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum()
    n |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum() |> println()
    let data = 1..100 |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum()
    n |> map((x) => x * math.pow(x, v)) |> filter((x) => x % 2 == 0) |> sum() |> println()

    // Ultra-complex mathematical computation with trigonometry, logarithms, and advanced transformations
    let complex_result = 1..1000 
        |> map((x) => math.sin(x / 100.0) * math.cos(x / 50.0))
        |> map((x) => math.pow(math.abs(x), 1.5) + math.ln(math.abs(x) + 1.0))
        |> filter((x) => x > 0.1)
        |> map((x) => x * math.tan(x / 10.0) + math.sqrt(math.abs(x)))
        |> filter((x) => math.floor(x * 1000.0) % 7 == 0)
        |> map((x) => math.pow(x, 2.0) + math.exp(x / 100.0))
        |> filter((x) => x < 1000.0)
        |> map((x) => math.asin(math.min(x / 1000.0, 1.0)) * math.acos(math.min(x / 500.0, 1.0)))
        |> reduce(0.0, (acc, x) => acc + x * math.sinh(x / 1000.0))
    
    complex_result |> println()

    // Nested pipeline with prime-like filtering and advanced number theory operations
    let prime_inspired = 2..5000
        |> filter((x) => x % 2 != 0 || x == 2)
        |> map((x) => math.pow(x, 1.0 / 3.0))
        |> filter((x) => math.floor(x * x * x) % 13 == 0)
        |> map((x) => math.log10(x + 1.0) * math.sin(x * math.PI / 180.0))
        |> filter((x) => math.abs(x) > 0.001)
        |> map((x) => math.pow(2.0, x) - math.pow(math.E, x / 10.0))
        |> filter((x) => x > 0.0)
        |> map((x) => math.atan2(x, math.sqrt(x + 1.0)) + math.log2(x + 1.0))
        |> sum()
    
    prime_inspired |> println()

    // Fractal-inspired recursive computation with multiple transformations
    let fractal_calc = 1..2000
        |> map((x) => x / 10.0)
        |> map((x) => math.sin(x) + math.cos(x * 2.0) + math.sin(x * 4.0) / 2.0)
        |> filter((x) => math.abs(x) > 0.5)
        |> map((x) => math.pow(x, 3.0) - 3.0 * x * math.pow(x, 1.0) + 1.0)
        |> filter((x) => math.abs(x) < 10.0)
        |> map((x) => math.sinh(x / 10.0) + math.cosh(math.abs(x) / 10.0))
        |> map((x) => x * math.pow(math.abs(x) + 1.0, math.min(3.0, math.abs(x) / 5.0)))
        |> filter((x) => math.abs(x) < 1000000.0)
        |> map((x) => math.cbrt(math.abs(x)) * math.sign(x))
        |> reduce(1.0, (acc, x) => acc * math.tanh(x / 100.0) + 0.1)
    
    fractal_calc |> println()
}