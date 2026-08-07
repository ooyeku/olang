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

// Suffix after the take_while prefix.
share fn drop_while(xs, pred) = {
    let final = xs |> fold(([], true), (acc, x) => {
        let (kept, dropping) = acc
        if dropping && pred(x) => (kept, true)
        else => (concat(kept, [x]), false)
    })
    let (result, _) = final
    result
}

// Map then flatten one level; a non-list result is kept as a single element.
share fn flat_map(xs, f) = xs |> fold([], (acc, x) => {
    let mapped = f(x)
    if typeof(mapped) == "List" => concat(acc, mapped)
    else => concat(acc, [mapped])
})

// Count occurrences of each distinct value (count_by with identity).
share fn frequencies(xs) = count_by(xs, (x) => x)

// The last element (errors on an empty list, like indexing past the end).
share fn last(xs) = xs[len(xs) - 1]

// The element with the smallest key; ties keep the first seen.
share fn min_by(xs, key_fn) = xs |> fold(head(xs), (best, x) =>
    if key_fn(x) < key_fn(best) => x else => best)

// The element with the largest key; ties keep the first seen.
share fn max_by(xs, key_fn) = xs |> fold(head(xs), (best, x) =>
    if key_fn(x) > key_fn(best) => x else => best)

// Stable sort by a key. Insertion sort (O(n^2)) — the native col.sort_by is
// the fast path; this is the readable olang mirror. Inserts before the first
// element with a strictly greater key, so equal keys keep their order.
fn insert_by_key(sorted, x, key_fn) = {
    let kx = key_fn(x)
    let final = sorted |> fold(([], false), (acc, y) => {
        let (out, placed) = acc
        if placed => (concat(out, [y]), true)
        else => if key_fn(y) > kx => (concat(out, [x, y]), true)
                else => (concat(out, [y]), false)
    })
    let (out, placed) = final
    if placed => out else => concat(out, [x])
}

share fn sort_by(xs, key_fn) = xs |> fold([], (sorted, x) =>
    insert_by_key(sorted, x, key_fn))

// Sliding windows of a fixed size: window([1,2,3,4], 2) = [[1,2],[2,3],[3,4]].
share fn window(xs, size) = {
    let n = len(xs)
    if size > n => []
    else => range(0, n - size + 1) |> map((i) => take(skip(xs, i), size))
}

// Combine parallel elements with f; truncates to the shorter list.
share fn zip_with(a, b, f) = zip(a, b) |> map((pair) => {
    let (x, y) = pair
    f(x, y)
})
