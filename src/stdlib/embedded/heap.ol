// heap — a binary min-heap, written in olang: the `collections.heap`
// submodule. Reach it fully qualified with no `use` at all, or import
// the short name: `use collections { heap }`.
//
// The heap orders (priority, item) pairs by priority — an Int or Float —
// and each operation touches O(log n) elements. `item` may be any value;
// in data-oriented code it is usually an index into your own lists, which
// keeps the heap itself a flat machine of numbers.
//
// Representation: one flat list, `[size, p0, i0, p1, i1, ...]` — the pair
// count first, then priority/item pairs in binary-heap order (children of
// pair c at 2c+1 and 2c+2). One list rather than a struct of lists so the
// handle itself is the backing store: passed by move and written through
// the sole-owner fusion, every operation is O(log n) writes in place, no
// copies. The list may be longer than 1 + 2*size — popped slots are kept
// for reuse, so a heap that grows and shrinks allocates only on its
// high-water mark.
//
// Calling convention (all the bundled collections share it): operations
// take the handle first and return the new handle, and the caller rebinds
// the same name — `h = heap.push(h, prio, item)`. The rebind is what lets
// the runtime pass the handle by move and mutate in place. Holding an
// older handle is legal and gives an unchanged snapshot; the next write
// through either handle then pays a one-time copy.
//
// Reads (`size`, `top_prio`, `top_item`, `is_empty`) take the handle
// without rebinding — they write nothing.

// A new, empty heap.
share fn new() = [0]

// The number of pairs in the heap.
share fn size(h) = h[0]

// True when the heap holds nothing. `top_prio`/`top_item`/`pop` are
// undefined on an empty heap (they raise an index error).
share fn is_empty(h) = h[0] == 0

// The smallest priority in the heap, without removing it.
share fn top_prio(h) = h[1]

// The item paired with the smallest priority, without removing it.
share fn top_item(h) = h[2]

// The heap with (prio, item) added: `h = heap.push(h, prio, item)`.
share fn push(h, prio, item) = {
    let n = h[0]
    h = col.set(h, 0, n + 1)
    // Grow only past the high-water mark; otherwise write into a slot a
    // pop left behind.
    if len(h) < 1 + (n + 1) * 2 => { h = h + [prio, item] }
    else => {
        h = col.set(h, 1 + n * 2, prio)
        h = col.set(h, 2 + n * 2, item)
    }
    // Sift the new pair up: swap with the parent while it beats it.
    let mut c = n
    while c > 0 {
        let parent = (c - 1) / 2
        if h[1 + parent * 2] <= h[1 + c * 2] => { break }
        h = col.swap(h, 1 + parent * 2, 1 + c * 2)
        h = col.swap(h, 2 + parent * 2, 2 + c * 2)
        c = parent
    }
    h
}

// The heap with its smallest pair removed: read `top_prio`/`top_item`
// first, then `h = heap.pop(h)`.
share fn pop(h) = {
    let n = h[0] - 1
    h = col.set(h, 0, n)
    // The last pair moves to the root, then sifts down to its place.
    h = col.swap(h, 1, 1 + n * 2)
    h = col.swap(h, 2, 2 + n * 2)
    let mut c = 0
    while true {
        let l = c * 2 + 1
        let r = c * 2 + 2
        let mut best = c
        if l < n && h[1 + l * 2] < h[1 + best * 2] => { best = l }
        if r < n && h[1 + r * 2] < h[1 + best * 2] => { best = r }
        if best == c => { break }
        h = col.swap(h, 1 + best * 2, 1 + c * 2)
        h = col.swap(h, 2 + best * 2, 2 + c * 2)
        c = best
    }
    h
}

// A heap built from parallel lists of priorities and items — O(n) sifts
// against O(n log n) repeated pushes.
share fn from_lists(prios, items) = {
    let mut h = [len(prios)]
    for k in range(0, len(prios)) {
        h = h + [prios[k], items[k]]
    }
    // Heapify: sift every internal node down, last parent first.
    let n = len(prios)
    let mut start = n / 2 - 1
    while start >= 0 {
        let mut c = start
        while true {
            let l = c * 2 + 1
            let r = c * 2 + 2
            let mut best = c
            if l < n && h[1 + l * 2] < h[1 + best * 2] => { best = l }
            if r < n && h[1 + r * 2] < h[1 + best * 2] => { best = r }
            if best == c => { break }
            h = col.swap(h, 1 + best * 2, 1 + c * 2)
            h = col.swap(h, 2 + best * 2, 2 + c * 2)
            c = best
        }
        start = start - 1
    }
    h
}

test "push and pop order by priority" {
    let mut h = new()
    h = push(h, 5, "five")
    h = push(h, 1, "one")
    h = push(h, 3, "three")
    testing.assert_eq(size(h), 3)
    testing.assert_eq(top_prio(h), 1)
    testing.assert_eq(top_item(h), "one")
    h = pop(h)
    testing.assert_eq(top_prio(h), 3)
    h = pop(h)
    testing.assert_eq(top_prio(h), 5)
    h = pop(h)
    testing.assert_eq(is_empty(h), true)
}

test "duplicate priorities all surface" {
    let mut h = new()
    h = push(h, 2, "a")
    h = push(h, 2, "b")
    h = push(h, 1, "c")
    h = pop(h)
    testing.assert_eq(top_prio(h), 2)
    h = pop(h)
    testing.assert_eq(top_prio(h), 2)
}

test "from_lists heapifies" {
    let mut h = from_lists([9, 4, 7, 1, 8], [0, 1, 2, 3, 4])
    testing.assert_eq(top_prio(h), 1)
    testing.assert_eq(top_item(h), 3)
    h = pop(h)
    testing.assert_eq(top_prio(h), 4)
}
