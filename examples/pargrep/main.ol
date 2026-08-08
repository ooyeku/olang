// pargrep — parallel code search on spawned worker threads.
//
//   olang main.ol <pattern> [dir] [workers]
//   olang main.ol "share fn" ../..  8
//
// The coordinator walks the tree, deals the files into chunks, spawns one
// worker thread per chunk, awaits them all, and merges. It also runs the
// same search sequentially and prints both timings — the speedup is the
// point of the exercise. A worker that fails (e.g. a bad pattern) is
// handled per-task with try/catch, so one bad worker cannot kill the run.

use lib.search { search_chunk }

let args = unwrap(os.args())
let pattern = if len(args) > 1 => args[1] else => "share fn"
let root = if len(args) > 2 => args[2] else => ".."
let workers = if len(args) > 3 => unwrap(str.parse_int(args[3])) else => 6

// ── collect the searchable files ──
fn is_text(path) =
    ends_with(path, ".ol") || ends_with(path, ".md") || ends_with(path, ".rs")
        || ends_with(path, ".toml")

let files = unwrap(fs.walk(root)) |> filter(is_text)

// ── sequential baseline ──
let t0 = time.monotonic_ms()
let seq_hits = search_chunk(files, pattern)
let seq_ms = time.monotonic_ms() - t0

// ── parallel: deal files into chunks, one spawned worker each ──
let t1 = time.monotonic_ms()
let chunk_size = (len(files) + workers - 1) / workers
let jobs = chunk(files, chunk_size)
    |> map((paths) => spawn search_chunk(paths, pattern))
let outcomes = jobs |> map((job) => try { await job } catch (e) { [] })
let par_hits = flatten(outcomes)
let par_ms = time.monotonic_ms() - t1

// ── report ──
let total = par_hits |> fold(0, (acc, h) => acc + h.count)
println(str.fmt("pargrep: /{}/ under {} — {} files, {} workers", pattern, root, len(files), len(jobs)))
println("")
let top = take(par_hits |> col.sort_by((h) => 0 - h.count), 10)
for h in top {
    println(str.fmt("  {}  {} hit{}", str.pad_end(h.file, 52, " "), h.count,
        if h.count == 1 => "" else => "s"))
}
if len(par_hits) > 10 => { println(str.fmt("  ... and {} more files", len(par_hits) - 10)) }
println("")
println(str.fmt("  total: {} hits in {} files", total, len(par_hits)))
println(str.fmt("  sequential: {}ms   parallel: {}ms", seq_ms, par_ms))

// ── self-check: parallel and sequential must agree exactly ──
test "parallel search agrees with sequential" {
    let s = seq_hits |> fold(0, (acc, h) => acc + h.count)
    assert_eq(total, s, "hit totals must match")
    assert_eq(len(par_hits), len(seq_hits), "file counts must match")
    assert_true(total > 0, "the tree contains the default pattern")
}
