let data = 1..10


let n = 1..100000

for numbers in n {
    let v = random.choice(n)
    data |> map((x) => x * math.pow(x, v)) |> sum() |> println()

    type Person = struct { age: Int }
    let p = Person { age: v }
    println("I am " + p.age + " years old")
}