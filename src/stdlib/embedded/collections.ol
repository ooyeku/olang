// collections — the bundled data structures and algorithms, written in
// olang, as one module of submodules. Automatically available:
// `collections.heap.push(...)` works with no `use`, and a program that
// never touches it never pays for it.
//
//   collections.heap     binary min-heap of (priority, item) pairs
//   collections.deque    double-ended queue over a ring buffer
//   collections.table    flat hash table, open addressing
//   collections.dsu      disjoint sets (union–find)
//   collections.bitset   dense integer set, 63 members per word
//   collections.alg      sort / bisect / select / graphs / Dijkstra
//
// For short names, import the submodules you want — the idiomatic form
// for code that leans on them:
//
//   use collections { heap, table }
//   let mut h = heap.new()
//   h = heap.push(h, 3, "job")
//
// Every submodule follows one calling convention: operations that write
// take the handle first and return it, and the caller rebinds the same
// name (`h = heap.push(h, ...)`); reads never rebind. The rebind is
// load-bearing — it lets the runtime pass the handle by move and write
// in place. Each submodule's source documents its own flat layout.

use heap
use deque
use bitset
use dsu
use table
use alg

share let heap = heap
share let deque = deque
share let bitset = bitset
share let dsu = dsu
share let table = table
share let alg = alg
