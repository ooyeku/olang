// instrument — zero-cost instrumentation as an imported macro library.
//
// Each macro rewrites code at load time, so the instrumentation is in
// the program itself: no wrapper closures at runtime, no engine, and
// `olang expand main.ol` shows exactly what was added. Imported with
// `use instrument`; only the meta fns travel — this module contributes
// nothing to the program at runtime.

// ── @memo: transparent memoization for a one-parameter function ──────
// The original body is kept under a fresh name; the public name becomes
// a cache lookup around it. Recursive calls in the body still name the
// public function, so recursion is memoized too — that is the entire
// trick, and it is what turns fib from exponential to linear. A
// companion `<name>_cache_size()` is generated so a test can pin the
// cache's growth rather than trusting the speedup.
meta fn memo(decl) = {
    let node = head(unwrap(meta.parse(decl)))
    let name = map_get(node, "name")
    let p0 = head(map_get(node, "params"))
    let impl_name = meta.fresh(name)
    let cache = meta.fresh("cache")
    let renamed = str.replace(decl, `fn ${name}(`, `fn ${impl_name}(`)
    renamed + `
let ${cache} = cell.new(#{})
fn ${name}(${p0}) = {
    let key = to_string(${p0})
    let seen = cell.get(${cache})
    if map_has_key(seen, key) => map_get(seen, key)
    else => {
        let result = ${impl_name}(${p0})
        cell.update(${cache}, (m) => map_set(m, key, result))
        result
    }
}
fn ${name}_cache_size() = map_len(cell.get(${cache}))`
}

// ── @trace: log every call with its arguments and result ─────────────
meta fn trace(decl) = {
    let node = head(unwrap(meta.parse(decl)))
    let name = map_get(node, "name")
    let params = map_get(node, "params")
    let plist = params |> join(", ")
    let shown = params |> map((p) => `show(${p})`) |> join(` + ", " + `)
    let impl_name = meta.fresh(name)
    let renamed = str.replace(decl, `fn ${name}(`, `fn ${impl_name}(`)
    renamed + `
fn ${name}(${plist}) = {
    println("→ ${name}(" + ${shown} + ")")
    let result = ${impl_name}(${plist})
    println("← ${name} = " + show(result))
    result
}`
}

// ── @timed: wall-clock any expression, labeled ───────────────────────
meta fn timed(label, e) = {
    let t0 = meta.fresh("t0")
    let r = meta.fresh("r")
    `{
    let ${t0} = time.monotonic_ms()
    let ${r} = ${e}
    println(${label} + ": " + to_string(time.monotonic_ms() - ${t0}) + "ms")
    ${r}
}`
}

// ── @dbg: print an expression's own source next to its value ─────────
meta fn dbg(e) = {
    let v = meta.fresh("v")
    `{
    let ${v} = ${e}
    println("dbg: " + ${meta.lit(e)} + " = " + show(${v}))
    ${v}
}`
}
