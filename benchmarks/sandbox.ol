// Sandboxed code runs faster: a price lookup that asks a service when the
// program may reach the network and computes the price when it may not,
// called three million times under `--deny net`, written three ways:
//
// - folded: `caps.allowed("net")`, a name the compiler reads. The grant is
//   fixed for the run, so the check is a constant, the network branch is
//   compiled away, and the function goes native;
// - checked: the same check through a variable, `caps.allowed(NET)`, so it
//   is decided at run time, as a sandbox usually decides, and the network
//   call it keeps holds the function back from native code;
// - plain: the price alone, no network branch (the floor).
//
//     olang --deny net benchmarks/sandbox.ol
//
// docs/performance.md ("Sandboxed code runs faster") tabulates it: the
// best of five runs of each, in milliseconds. Every way sums the same.

let NET = "net"
let CALLS = 3000000

fn folded(i) = if caps.allowed("net") => match http.get("http://127.0.0.1:9/p/" + to_string(i)) { Ok(r) => r.status * 1.0, Err(e) => 0.0 }
    else => to_float(i % 97) * 1.07 + to_float(i % 13) * 0.5
fn checked(i) = if caps.allowed(NET) => match http.get("http://127.0.0.1:9/p/" + to_string(i)) { Ok(r) => r.status * 1.0, Err(e) => 0.0 }
    else => to_float(i % 97) * 1.07 + to_float(i % 13) * 0.5
fn plain(i) = to_float(i % 97) * 1.07 + to_float(i % 13) * 0.5

fn total_folded(n) = {
    let mut s = 0.0
    for i in range(n) { s = s + folded(i) }
    s
}
fn total_checked(n) = {
    let mut s = 0.0
    for i in range(n) { s = s + checked(i) }
    s
}
fn total_plain(n) = {
    let mut s = 0.0
    for i in range(n) { s = s + plain(i) }
    s
}

/// The best of five runs of `f`, in milliseconds, and what it summed.
fn best(f) = fold(range(5), (1000000000, 0.0), (acc, k) => {
    let t0 = time.now_ms()
    let s = f(CALLS)
    (min(acc[0], time.now_ms() - t0), s)
})

if caps.allowed("net") => println("run it under --deny net: the sandbox is what it measures")
else => {
    let (f_ms, f_sum) = best(total_folded)
    let (c_ms, c_sum) = best(total_checked)
    let (p_ms, p_sum) = best(total_plain)
    if f_sum != c_sum || c_sum != p_sum => println(`the sums differ: ${f_sum} ${c_sum} ${p_sum}`)
    else => {
        println(`folded   ${f_ms} ms`)
        println(`checked  ${c_ms} ms  (${str.fixed(to_float(c_ms) / to_float(f_ms), 2)}x the folded)`)
        println(`plain    ${p_ms} ms`)
    }
}
