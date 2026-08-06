// colx — a slice of the `col` collections module, written in olang and
// bundled into the binary as a builtin package. It demonstrates that pure
// (non-FFI) stdlib modules can be self-hosted: every function here is a
// fold/map composition over the top-level builtins.
//
// The native Rust `col` module remains the default (it is faster); this is
// the differential-tested olang mirror.

// Deduplicate, preserving first-seen order.
share fn unique(xs) = xs |> fold([], (acc, x) =>
    if contains(acc, x) => acc else => concat(acc, [x]))

// Split into (matching, non-matching) by a predicate.
share fn partition(xs, pred) = {
    let yes = xs |> filter(pred)
    let no = xs |> filter((x) => if pred(x) => false else => true)
    (yes, no)
}

// Sum of f(x) over the list.
share fn sum_by(xs, f) = xs |> fold(0, (acc, x) => acc + f(x))

// Do all elements satisfy the predicate?
share fn all(xs, pred) = xs |> fold(true, (acc, x) => acc && pred(x))

// Does any element satisfy the predicate?
share fn any(xs, pred) = xs |> fold(false, (acc, x) => acc || pred(x))

// Count occurrences of each key produced by key_fn -> map.
share fn count_by(xs, key_fn) = xs |> fold(#{}, (acc, x) => {
    let k = key_fn(x)
    map_set(acc, k, if map_has_key(acc, k) => map_get(acc, k) + 1 else => 1)
})

// Longest prefix whose elements all satisfy the predicate. The "still
// taking" flag is threaded through the fold accumulator (a (list, bool)
// tuple) rather than mutated in the closure — a lambda cannot write back to
// a variable it captured, so purely-functional state threading is required.
share fn take_while(xs, pred) = {
    let final = xs |> fold(([], true), (acc, x) => {
        let (taken, taking) = acc
        if taking && pred(x) => (concat(taken, [x]), true)
        else => (taken, false)
    })
    let (result, _) = final
    result
}
