// The record pipeline an application's query layer runs: filter -> map
// -> sort over 2,000 record maps, three map_gets and a string compare per
// row, a three-key map built per kept row. The shape roadmap W15 item 3
// measures (10-15 ms here against 0.5 ms in Node); `olang bench
// benchmarks/records.ol --save` pins it, and `--in-task` runs it as an
// http worker would.
let n = 2000
let issues = map(range(0, n), (i) => #{
    "id": i, "title": "Issue " + to_string(i), "state": if i % 3 == 0 => "closed" else => "open",
    "priority": i % 5, "assignee": "user" + to_string(i % 17), "labels": ["a", "b"],
    "description": "- [ ] one\n- [x] two\n", "updated": 1700000000 + i * 37 })
fn run(rows) = {
    let open = filter(rows, (r) => r.state == "open")
    let shaped = map(open, (r) => #{ "id": r.id, "title": r.title,
        "score": r.priority * 10 + str.length(r.assignee) })
    col.sort_by(shaped, (r) => r.score)
}
let t0 = time.monotonic_ms()
let mut out = []
for i in range(0, 20) { out = run(issues) }
let ms = (time.monotonic_ms() - t0) / 20
println("run over " + to_string(n) + ": " + to_string(ms) + " ms (" + to_string(len(out)) + ")")
