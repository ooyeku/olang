// alg — the algorithms companion to the bundled data structures,
// written in olang: the `collections.alg` submodule. Fully qualified
// with no `use`, or `use collections { alg }` for the short name.
//
// Five families: stable sorting (plain and by key), the binary-search
// family over sorted lists, selection without sorting, graph traversal
// over flat adjacency (BFS, DFS order, topological sort), and Dijkstra
// shortest paths — the composed showcase, running `heap` over a flat
// weighted graph.
//
// The performance discipline throughout: flat lists, index arithmetic,
// loops and tail recursion for kernels, and keys rather than
// comparators. `sort_by_key` calls its key function once per element —
// a single map — and then sorts on the extracted keys with native
// comparisons; a comparator would be called O(n log n) times, which is
// the one API shape that cannot be fast. Sorted-ness, where required,
// is the caller's contract (the usual bisect rule).
//
// Graphs are flat CSR (compressed sparse row) handles built once from
// an edge list — `[n, m, offset0..offsetN, target0..targetM-1]`, node
// `u`'s neighbors at targets[offsets[u] .. offsets[u+1]] — so traversal
// is index chasing over two arrays, no per-node allocation. The
// weighted variant appends a parallel weight array.

// The frontier queue and the priority queue come from the sibling
// bundled modules — collections composing collections, all in olang.
use deque
use heap

// Defined ahead of every user: a module function's closure holds what
// existed when it was defined, and the global `min` is the list-taking
// builtin, not a pair comparison.
fn smaller(a, b) = if a < b => a else => b

// ── sorting ───────────────────────────────────────────────────────────

// The list, sorted ascending and stable. Elements must be mutually
// comparable (all numbers, or all strings) — the comparison itself
// raises otherwise, naming the offending pair. Iterative bottom-up
// merge sort: O(n log n) always, no recursion bookkeeping, one aux
// buffer reused across passes.
share fn sort(xs) = {
    let n = len(xs)
    if n < 2 => { xs }
    else => {
        let mut a = xs
        let mut b = col.filled(n, xs[0])
        let mut width = 1
        while width < n {
            let mut lo = 0
            while lo < n {
                let mid = smaller(lo + width, n)
                let hi = smaller(lo + width * 2, n)
                // Merge a[lo..mid] and a[mid..hi] into b[lo..hi],
                // left-first on ties: stability.
                let mut i = lo
                let mut j = mid
                let mut out = lo
                while i < mid && j < hi {
                    if a[j] < a[i] => {
                        b = col.set(b, out, a[j])
                        j = j + 1
                    }
                    else => {
                        b = col.set(b, out, a[i])
                        i = i + 1
                    }
                    out = out + 1
                }
                while i < mid {
                    b = col.set(b, out, a[i])
                    i = i + 1
                    out = out + 1
                }
                while j < hi {
                    b = col.set(b, out, a[j])
                    j = j + 1
                    out = out + 1
                }
                lo = hi
            }
            // The merged buffer becomes the source; the old source is
            // the next pass's scratch.
            let t = a
            a = b
            b = t
            width = width * 2
        }
        a
    }
}

// The list sorted ascending and stable by `key_fn(x)` — called once per
// element. Sorts (key, index) pairs, then gathers the elements.
share fn sort_by_key(xs, key_fn) = {
    let n = len(xs)
    if n < 2 => { xs }
    else => {
        // Pair each key with its index; the index breaks ties in
        // original order, which is what makes the result stable even
        // though the pair sort needn't be.
        let keys = map(xs, key_fn)
        let mut order = col.filled(n, 0)
        for i in range(0, n) {
            order = col.set(order, i, i)
        }
        order = sort_indices(keys, order)
        let mut out = col.filled(n, xs[0])
        for i in range(0, n) {
            out = col.set(out, i, xs[order[i]])
        }
        out
    }
}

// ── binary search over sorted lists ───────────────────────────────────

// The first index whose element is >= x — the insertion point that
// keeps `xs` sorted. `len(xs)` when every element is smaller.
share fn lower_bound(xs, x) = {
    let mut lo = 0
    let mut hi = len(xs)
    while lo < hi {
        let mid = (lo + hi) / 2
        if xs[mid] < x => { lo = mid + 1 }
        else => { hi = mid }
    }
    lo
}

// The first index whose element is > x. With `lower_bound`, brackets
// the run of elements equal to x.
share fn upper_bound(xs, x) = {
    let mut lo = 0
    let mut hi = len(xs)
    while lo < hi {
        let mid = (lo + hi) / 2
        if xs[mid] <= x => { lo = mid + 1 }
        else => { hi = mid }
    }
    lo
}

// The index of `x` in sorted `xs`, or `()` when absent — Unit is the
// absence value, the same contract as `str.index_of`.
share fn bin_search(xs, x) = {
    let at = lower_bound(xs, x)
    if at < len(xs) && xs[at] == x => { at } else => { () }
}

// ── selection ─────────────────────────────────────────────────────────

// The k-th smallest element (0-based) without sorting: quickselect over
// a working copy, expected O(n). The classic answer to medians and
// percentiles when a full sort is more than the question needs.
share fn select_kth(xs, k) = {
    let mut a = xs
    let mut lo = 0
    let mut hi = len(a) - 1
    while lo < hi {
        // Median-of-three pivot, moved to the end.
        let mid = (lo + hi) / 2
        if a[mid] < a[lo] => { a = col.swap(a, lo, mid) }
        if a[hi] < a[lo] => { a = col.swap(a, lo, hi) }
        if a[hi] < a[mid] => { a = col.swap(a, mid, hi) }
        a = col.swap(a, mid, hi)
        let pivot = a[hi]
        let mut store = lo
        for i in range(lo, hi) {
            if a[i] < pivot => {
                a = col.swap(a, i, store)
                store = store + 1
            }
        }
        a = col.swap(a, store, hi)
        if k < store => { hi = store - 1 }
        else => {
            if k > store => { lo = store + 1 }
            else => {
                lo = store
                hi = store
            }
        }
    }
    a[k]
}

// ── graphs: flat CSR adjacency ────────────────────────────────────────

// A directed graph of `n` nodes from an edge list `[[u, v], ...]`, as a
// flat CSR handle. Build once, traverse many times.
share fn graph(n, edges) = {
    let m = len(edges)
    // Counting sort of edges by source: offsets by prefix sum, then a
    // placement pass.
    let mut g = col.filled(2 + n + 1 + m, 0)
    g = col.set(g, 0, n)
    g = col.set(g, 1, m)
    for e in edges {
        g = col.set(g, 2 + e[0] + 1, g[2 + e[0] + 1] + 1)
    }
    for u in range(0, n) {
        g = col.set(g, 2 + u + 1, g[2 + u + 1] + g[2 + u])
    }
    let mut cursor = col.filled(n, 0)
    for u in range(0, n) {
        cursor = col.set(cursor, u, g[2 + u])
    }
    for e in edges {
        g = col.set(g, 2 + n + 1 + cursor[e[0]], e[1])
        cursor = col.set(cursor, e[0], cursor[e[0]] + 1)
    }
    g
}

// A weighted directed graph from `[[u, v, w], ...]` — the CSR handle
// with a parallel weight array appended. Weights are numbers.
share fn wgraph(n, edges) = {
    let m = len(edges)
    let mut g = col.filled(2 + n + 1 + m * 2, 0)
    g = col.set(g, 0, n)
    g = col.set(g, 1, m)
    for e in edges {
        g = col.set(g, 2 + e[0] + 1, g[2 + e[0] + 1] + 1)
    }
    for u in range(0, n) {
        g = col.set(g, 2 + u + 1, g[2 + u + 1] + g[2 + u])
    }
    let mut cursor = col.filled(n, 0)
    for u in range(0, n) {
        cursor = col.set(cursor, u, g[2 + u])
    }
    for e in edges {
        let at = cursor[e[0]]
        g = col.set(g, 2 + n + 1 + at, e[1])
        g = col.set(g, 2 + n + 1 + m + at, e[2])
        cursor = col.set(cursor, e[0], at + 1)
    }
    g
}

// Hop distances from `src`: a list with -1 for unreachable nodes.
// Breadth-first over the CSR arrays, the frontier in a ring-buffer
// deque.
share fn bfs(g, src) = {
    let n = g[0]
    let mut dist = col.filled(n, -1)
    dist = col.set(dist, src, 0)
    let mut q = deque.new()
    q = deque.push_back(q, src)
    while !deque.is_empty(q) {
        let u = deque.front(q)
        q = deque.pop_front(q)
        for e in range(g[2 + u], g[2 + u + 1]) {
            let v = g[2 + g[0] + 1 + e]
            if dist[v] == -1 => {
                dist = col.set(dist, v, dist[u] + 1)
                q = deque.push_back(q, v)
            }
        }
    }
    dist
}

// The nodes in a topological order, as `Ok(order)` — or `Err("cycle")`
// when the graph has one, which is data about the input, not a bug.
// Kahn's algorithm: repeatedly emit a node with no remaining incoming
// edges.
share fn topo_sort(g) = {
    let n = g[0]
    let mut indegree = col.filled(n, 0)
    for e in range(0, g[1]) {
        let v = g[2 + n + 1 + e]
        indegree = col.set(indegree, v, indegree[v] + 1)
    }
    let mut q = deque.new()
    for u in range(0, n) {
        if indegree[u] == 0 => { q = deque.push_back(q, u) }
    }
    let mut order = []
    while !deque.is_empty(q) {
        let u = deque.front(q)
        q = deque.pop_front(q)
        order = order + [u]
        for e in range(g[2 + u], g[2 + u + 1]) {
            let v = g[2 + n + 1 + e]
            indegree = col.set(indegree, v, indegree[v] - 1)
            if indegree[v] == 0 => { q = deque.push_back(q, v) }
        }
    }
    if len(order) == n => { Ok(order) } else => { Err("cycle") }
}

// Shortest-path distances from `src` over a weighted graph (`wgraph`
// handle, non-negative weights): a list with -1 for unreachable nodes.
// Dijkstra with a `heap` of (distance, node) pairs and lazy deletion —
// a popped pair older than the node's settled distance is skipped.
share fn dijkstra(g, src) = {
    let n = g[0]
    let m = g[1]
    let mut dist = col.filled(n, -1)
    dist = col.set(dist, src, 0)
    let mut h = heap.new()
    h = heap.push(h, 0, src)
    while !heap.is_empty(h) {
        let d = heap.top_prio(h)
        let u = heap.top_item(h)
        h = heap.pop(h)
        // Lazy deletion: a stale entry describes a longer route to a
        // node already settled by a shorter one.
        if dist[u] == d => {
            for e in range(g[2 + u], g[2 + u + 1]) {
                let v = g[2 + n + 1 + e]
                let next = d + g[2 + n + 1 + m + e]
                if dist[v] == -1 || next < dist[v] => {
                    dist = col.set(dist, v, next)
                    h = heap.push(h, next, v)
                }
            }
        }
    }
    dist
}

// ── internals ─────────────────────────────────────────────────────────

// Stable index sort: `order` (a permutation of 0..n) sorted so keys are
// ascending, equal keys in ascending index order. The same bottom-up
// merge as `sort`, comparing through the key list.
fn sort_indices(keys, order) = {
    let n = len(order)
    let mut a = order
    let mut b = col.filled(n, 0)
    let mut width = 1
    while width < n {
        let mut lo = 0
        while lo < n {
            let mid = smaller(lo + width, n)
            let hi = smaller(lo + width * 2, n)
            let mut i = lo
            let mut j = mid
            let mut out = lo
            while i < mid && j < hi {
                // Strictly-less takes the right element; ties keep left
                // first, and left indexes are smaller: stability.
                if keys[a[j]] < keys[a[i]] => {
                    b = col.set(b, out, a[j])
                    j = j + 1
                }
                else => {
                    b = col.set(b, out, a[i])
                    i = i + 1
                }
                out = out + 1
            }
            while i < mid {
                b = col.set(b, out, a[i])
                i = i + 1
                out = out + 1
            }
            while j < hi {
                b = col.set(b, out, a[j])
                j = j + 1
                out = out + 1
            }
            lo = hi
        }
        let t = a
        a = b
        b = t
        width = width * 2
    }
    a
}

test "sort is ordered and stable by value" {
    testing.assert_eq(sort([3, 1, 2]), [1, 2, 3])
    testing.assert_eq(sort([]), [])
    testing.assert_eq(sort([5]), [5])
    testing.assert_eq(sort(["pear", "apple", "fig"]), ["apple", "fig", "pear"])
}

test "sort_by_key is stable" {
    let xs = ["bb", "a", "cc", "dd", "e"]
    // Equal lengths keep their original order.
    testing.assert_eq(sort_by_key(xs, (s) => len(s)), ["a", "e", "bb", "cc", "dd"])
}

test "bounds bracket equal runs" {
    let xs = [1, 3, 3, 3, 7]
    testing.assert_eq(lower_bound(xs, 3), 1)
    testing.assert_eq(upper_bound(xs, 3), 4)
    testing.assert_eq(lower_bound(xs, 0), 0)
    testing.assert_eq(upper_bound(xs, 9), 5)
    testing.assert_eq(bin_search(xs, 7), 4)
    testing.assert_eq(bin_search(xs, 4), ())
}

test "select_kth matches sorting" {
    let xs = [9, 1, 8, 2, 7, 3]
    let sorted = sort(xs)
    for k in range(0, len(xs)) {
        testing.assert_eq(select_kth(xs, k), sorted[k])
    }
}

test "bfs measures hops" {
    // 0 -> 1 -> 2, 0 -> 2, 3 isolated
    let g = graph(4, [[0, 1], [1, 2], [0, 2]])
    testing.assert_eq(bfs(g, 0), [0, 1, 1, -1])
}

test "topo_sort orders a dag and names a cycle" {
    let dag = graph(4, [[0, 1], [0, 2], [1, 3], [2, 3]])
    let order = topo_sort(dag) |> unwrap
    testing.assert_eq(order[0], 0)
    testing.assert_eq(order[3], 3)
    let cyclic = graph(2, [[0, 1], [1, 0]])
    testing.assert_eq(topo_sort(cyclic), Err("cycle"))
}

test "dijkstra takes the cheaper route" {
    // 0 -> 1 (4), 0 -> 2 (1), 2 -> 1 (2): best 0 -> 1 is 3, via 2.
    let g = wgraph(3, [[0, 1, 4], [0, 2, 1], [2, 1, 2]])
    testing.assert_eq(dijkstra(g, 0), [0, 3, 1])
}
