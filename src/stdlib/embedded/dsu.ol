// dsu — disjoint sets (union–find), written in olang and bundled into
// the binary. Automatically available: `dsu.new(n)` works with no `use`.
//
// The structure answers one question fast — "are a and b in the same
// group?" — while groups merge over time. It is the backbone of Kruskal's
// spanning trees, connectivity checks, clustering, and deduplication.
//
// Representation: one flat list, `[n, parent0..parentN-1, rank0..rankN-1]`
// — the element count, each element's parent (itself for a root), then
// each root's rank (an upper bound on its tree height). Union by rank
// keeps trees shallow; `union` additionally compresses the two walked
// paths, so chains flatten as the structure is used. `find` itself writes
// nothing — it is a read, safe to call without rebinding — which keeps
// the read/write split of the calling convention honest: only operations
// you rebind through (`d = dsu.union(d, a, b)`) mutate.
//
// With union by rank alone, operations are O(log n); the compression in
// `union` brings sequences of mixed operations close to constant per op.

// A new structure of `n` elements, each in its own group.
share fn new(n) = {
    let mut d = col.filled(1 + n * 2, 0)
    d = col.set(d, 0, n)
    for i in range(0, n) {
        d = col.set(d, 1 + i, i)
    }
    d
}

// The number of elements (not groups) in the structure.
share fn size(d) = d[0]

// The root representative of `x`'s group. A read: follows parent links
// without rewriting them.
share fn find(d, x) = {
    let mut c = x
    while d[1 + c] != c {
        c = d[1 + c]
    }
    c
}

// True when `a` and `b` are in the same group.
share fn connected(d, a, b) = find(d, a) == find(d, b)

// The structure with `a`'s and `b`'s groups merged:
// `d = dsu.union(d, a, b)`. Compresses both walked paths and links by
// rank. Uniting elements already in one group returns the structure
// unchanged (aside from compression).
share fn union(d, a, b) = {
    let ra = find(d, a)
    let rb = find(d, b)
    // Compress: point every node on both walked paths straight at its
    // root, so the next find is a hop.
    let mut c = a
    while d[1 + c] != c {
        let next = d[1 + c]
        d = col.set(d, 1 + c, ra)
        c = next
    }
    c = b
    while d[1 + c] != c {
        let next = d[1 + c]
        d = col.set(d, 1 + c, rb)
        c = next
    }
    if ra == rb => { d }
    else => {
        let n = d[0]
        let rank_a = d[1 + n + ra]
        let rank_b = d[1 + n + rb]
        // The shallower tree hangs under the deeper root; equal ranks
        // pick `ra` and grow its rank by one.
        if rank_a < rank_b => {
            d = col.set(d, 1 + ra, rb)
            d
        }
        else => {
            d = col.set(d, 1 + rb, ra)
            if rank_a == rank_b => { d = col.set(d, 1 + n + ra, rank_a + 1) }
            d
        }
    }
}

// The number of distinct groups.
share fn groups(d) = {
    let mut count = 0
    for i in range(0, d[0]) {
        if d[1 + i] == i => { count = count + 1 }
    }
    count
}

test "fresh elements are their own groups" {
    let d = new(4)
    testing.assert_eq(groups(d), 4)
    testing.assert_eq(connected(d, 0, 3), false)
    testing.assert_eq(find(d, 2), 2)
}

test "union connects transitively" {
    let mut d = new(6)
    d = union(d, 0, 1)
    d = union(d, 1, 2)
    d = union(d, 4, 5)
    testing.assert_eq(connected(d, 0, 2), true)
    testing.assert_eq(connected(d, 0, 4), false)
    testing.assert_eq(groups(d), 3)
}

test "self-union changes nothing" {
    let mut d = new(3)
    d = union(d, 1, 1)
    testing.assert_eq(groups(d), 3)
}
