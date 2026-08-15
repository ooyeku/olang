// metatool — olang reading olang, over `meta.parse`.
//
// The `meta` module hands a parsed program back as ordinary olang values:
// a list of statement maps, each tagged with a "kind", that you walk with
// the same map/filter/fold you use on any data. Linters, codemods, and
// import extractors become olang scripts instead of compiler changes —
// the "open code" pillar of the openness campaign.
//
// This tool parses a sample program (kept inline so the example is
// self-contained) and reports two things a real linter would: its
// imports, and every bare `unwrap(...)` call, grouped by the function it
// lives in. `otc deps` — "list a file's imports" — is the first four
// lines of it.

let sample = "
use geometry { area, circle }
use fmt

share fn describe(shape) = {
    let a = area(shape)
    println(str.fmt(\"area is {}\", a))
    unwrap(risky())
}

fn total(shapes) = shapes
    |> map((s) => unwrap(area_of(s)))
    |> fold(0.0, (acc, x) => acc + x)

fn safe(x) = if x > 0 => Ok(x) else => Err(\"negative\")
"

let program = unwrap(meta.parse(sample))

// ── Imports: the whole of `otc deps`, in olang ──────────────────────
println("── imports ──")
for node in program |> filter((n) => map_get(n, "kind") == "use") {
    println("  " + map_get(node, "path") + " " + show(map_get(node, "items")))
}

// ── A lint: count bare unwrap() call sites, walking the tree ────────
// Every node is a map tagged with "kind"; children are nested maps or
// lists of maps. One recursive walk counts `unwrap` call targets.
fn count_unwraps(node) = {
    if typeof(node) == "Map" => {
        let here = if map_get(node, "kind") == "call" && map_get(node, "target") == "unwrap"
            => 1 else => 0
        here + (entries(node) |> fold(0, (acc, kv) => acc + count_unwraps(kv[1])))
    }
    else => if typeof(node) == "List"
        => node |> fold(0, (acc, child) => acc + count_unwraps(child))
        else => 0
}

println("── bare unwrap() call sites, per function ──")
let fns = program |> filter((n) => map_get(n, "kind") == "fn")
for f in fns {
    let n = count_unwraps(map_get(f, "body"))
    let shared = if map_get(f, "shared") => "share " else => ""
    println("  " + shared + "fn " + map_get(f, "name") + ": " + show(n) + " unwrap(s)")
}

let total_unwraps = fns |> fold(0, (acc, f) => acc + count_unwraps(map_get(f, "body")))
println("── " + show(len(program)) + " top-level declarations, " +
    show(total_unwraps) + " bare unwraps ──")

test "meta.parse exposes the program as walkable data" {
    let p = unwrap(meta.parse("use x { a }\nfn f() = unwrap(g())\n"))
    testing.assert_eq(len(p), 2)
    testing.assert_eq(map_get(p[0], "kind"), "use")
    testing.assert_eq(map_get(p[0], "path"), "x")
    testing.assert_eq(map_get(p[1], "kind"), "fn")
    testing.assert_eq(map_get(p[1], "name"), "f")
    // a syntax error is a normal Err, not a crash
    testing.assert_true(is_err(meta.parse("fn (")))
}
