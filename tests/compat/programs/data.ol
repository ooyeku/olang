// compat: collections and data — lists, maps, tuples, col, math, vec,
// bytes, compress, value, bigint, the persistent collections, cell.

let xs = [5, 3, 8, 1]
println(len(xs))
println(first(xs))
println(last(xs))
println(first([]))
println(xs[1])
println(xs + [9])
println(sort(xs))
println(reverse(xs))
println(col.sum(xs))
println(col.max(xs))
println(col.take(xs, 2))
println(col.zip([1, 2], ["a", "b"]))
println(col.unique([1, 1, 2, 3, 3]))
println(col.frequencies(["a", "b", "a"]))
println(col.partition(xs, (v) => v > 4))
println(col.sort_by(["ccc", "a", "bb"], (s) => len(s)))
println(col.flatten([[1], [2, 3]]))
println(col.window([1, 2, 3, 4], 2))
println(any(xs, (v) => v > 7))
println(all(xs, (v) => v > 0))

let m = #{ "a": 1, "b": 2 }
println(map_get(m, "a"))
println(map_set(m, "c", 3))
println(#{ ...m, "a": 10 })
println(map_keys(m))
println(map_values(m))
println(map_has_key(m, "b"))
println(map_set_in(#{}, ["x", "y"], 1))
println(m)

let t = (1, "two", 3.0)
println(t[1])
println(len(t))

println(math.PI)
println(math.sqrt(16.0))
println(vec.dot([1.0, 2.0], [3.0, 4.0]))
println(vec.add([1.0, 2.0], [3.0, 4.0]))

let b = bytes.from_string("olang")
println(bytes.len(b))
println(bytes.to_hex(b))
println(bytes.from_hex("6f6c616e67") |> unwrap |> bytes.to_string)
println(compress.gunzip(compress.gzip(b)) |> unwrap |> bytes.to_string)
println(value.decode(value.encode(#{ "k": [1, (2, 3)] })))

println(bigint.of(10) * bigint.of(10))

let pm = pmap.set(pmap.of(#{ "a": 1 }), "b", 2)
println(pmap.get(pm, "b"))
println(pmap.len(pm))
println(pmap.to_map(pm))
let pv = pvec.push(pvec.of([1, 2]), 3)
println(pvec.to_list(pv))
println(pvec.get(pv, 0))
let ps = psorted.of(#{ "b": 2, "a": 1, "c": 3 })
println(psorted.keys(ps))
println(psorted.first(ps))

let c = cell.new(0)
for i in range(0, 5) { cell.update(c, (v) => v + i) }
println(cell.get(c))
cell.set(c, 100)
println(cell.get(c))
