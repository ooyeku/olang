// compat: the embedded modules and packages — colx, mathx, cli,
// collections — which follow the append-mostly rule.
use colx
use mathx
use cli
use collections { heap, deque }

println(colx.sum_by([1, 2, 3], (v) => v * 10))
println(colx.window([1, 2, 3], 2))
println(mathx.factorial(10))
println(mathx.gcd(84, 36))

let spec = {
    name: "greet",
    flags: [
        { name: "loud", short: "l", type: "bool" },
        { name: "count", short: "n", type: "int", default: 1 }
    ],
    args: [ { name: "who", required: true } ]
}
let parsed = unwrap(cli.parse(spec, ["-n", "3", "--loud", "Ada"]))
println(`${parsed.who} ${parsed.count} ${parsed.loud}`)
println(is_err(cli.parse(spec, [])))

let h = heap.push(heap.push(heap.new(), 3, "job-a"), 1, "job-b")
println(heap.top_item(h))
println(heap.size(h))
let d = deque.push_back(deque.push_front(deque.new(), 1), 2)
println(deque.to_list(d))
