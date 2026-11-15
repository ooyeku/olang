// compat: the error model — Result and ?, the helpers, the boundaries
// (attempt, a spawned task), raise, and the three named sentinels.
// Error *text* is not contract, so only raise's message is printed.

fn safe_div(a, b) = if b == 0 => Err("division by zero") else => Ok(a / b)
println(`${is_ok(safe_div(10, 2))} ${is_err(safe_div(1, 0))}`)
println(unwrap(safe_div(8, 2)))
println(unwrap_or(safe_div(1, 0), -1))
println(unwrap_or_else(safe_div(1, 0), (e) => 0 - len(e)))
println(result_map(safe_div(6, 2), (v) => v * 10))

fn parse_both(a, b) = {
    let x = str.parse_int(a)?
    let y = str.parse_int(b)?
    Ok(x + y)
}
println(parse_both("2", "40"))
println(is_err(parse_both("2", "oops")))

println(attempt(() => 41 + 1))
println(is_err(attempt(() => map_get(5, "k"))))
fn widget(k) = if k == "button" => { kind: k } else => raise("widget: no kind " + k)
println(widget("button"))
println(attempt(() => widget("dial")))

// a return or ? inside attempt passes through as control flow
fn early() = { attempt(() => 1); return 2 }
println(early())

// a spawned task's failure is Err from task.join
fn risky(n) = if n > 2 => 1 + "not a number" else => n * 10
let jobs = [spawn risky(1), spawn risky(9)]
println(jobs |> map((j) => match task.join(j) { Ok(v) => v, Err(_) => -1 }))
let raised = spawn raise("from a task")
// (stability.md: task.join hands raise's message back as given; OL-627 tracks
// the "Runtime error: " prefix it carries today, so only the end is pinned)
println(match task.join(raised) { Err(e) => str.ends_with(e, "from a task"), _ => false })

// the sentinels are contract
let c = chan.new()
chan.close(c)
println(chan.recv(c))
let waiting = chan.new()
println(chan.recv_timeout(waiting, 10))

// a missing key is (), never an error
println(map_get(#{ "a": 1 }, "b"))
println(#{ "a": 1 }.b ?? "fallback")
