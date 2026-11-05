// bitset — a fixed-capacity set of small non-negative integers, written
// in olang: the `collections.bitset` submodule. Fully qualified with no
// `use`, or `use collections { bitset }` for the short name.
//
// Sixty-four members per word: membership, insertion, and removal are
// one index plus one mask, and whole-set algebra (union, intersect,
// difference) runs a word at a time with `&`, `|` and `^`. Bit 63 is the
// Int's sign bit, so its mask is made with `math.wrapping_shl`, which
// `<<` would refuse as an overflow. This is the structure behind sieves,
// visited-sets over dense ids, and set algebra where the universe is
// known in advance.
//
// Representation: one flat list, `[nbits, w0, w1, ...]` — the capacity,
// then ceil(nbits/64) words of bits, bit `i` at word `i >> 6`, position
// `i & 63`. Writes go through the sole-owner fusion, so
// `b = bitset.add(b, i)` is O(1) in place under the shared calling
// convention: rebind through every write, read without rebinding.
//
// Bit positions must be in `0 <= i < nbits`; a position past the last
// word, or a negative one (read unsigned, so far past it), raises the
// list's own index error — a bad position is a caller's bug, not data.

// A new, empty set with capacity for members 0..n-1.
share fn new(n) = {
    let mut b = col.filled(1 + (n + 63) / 64, 0)
    b = col.set(b, 0, n)
    b
}

// The capacity the set was created with.
share fn capacity(b) = b[0]

// True when `i` is a member.
share fn has(b, i) = (b[1 + math.ushr(i, 6)] & math.wrapping_shl(1, i & 63)) != 0

// The set with `i` added: `b = bitset.add(b, i)`.
share fn add(b, i) = {
    let at = 1 + math.ushr(i, 6)
    let word = b[at] | math.wrapping_shl(1, i & 63)
    b = col.set(b, at, word)
    b
}

// The set with `i` removed: `b = bitset.remove(b, i)`.
share fn remove(b, i) = {
    let at = 1 + math.ushr(i, 6)
    let word = b[at] & (math.wrapping_shl(1, i & 63) ^ -1)
    b = col.set(b, at, word)
    b
}

// The number of members: a popcount per word.
share fn count(b) = {
    let mut total = 0
    for w in range(1, len(b)) { total = total + math.popcount(b[w]) }
    total
}

// The union of two sets of equal capacity: members of either.
share fn union(a, b) = {
    let mut out = a
    for w in range(1, len(a)) { out = col.set(out, w, a[w] | b[w]) }
    out
}

// The intersection of two sets of equal capacity: members of both.
share fn intersect(a, b) = {
    let mut out = a
    for w in range(1, len(a)) { out = col.set(out, w, a[w] & b[w]) }
    out
}

// The difference of two sets of equal capacity: members of `a` not in `b`.
share fn difference(a, b) = {
    let mut out = a
    for w in range(1, len(a)) { out = col.set(out, w, a[w] & (b[w] ^ -1)) }
    out
}

// The members, in ascending order, as a list of Ints. Each step takes
// the lowest set bit of a word (`ctz`) and clears it.
share fn to_list(b) = {
    let mut out = []
    for w in range(1, len(b)) {
        let base = (w - 1) * 64
        let mut word = b[w]
        while word != 0 {
            out = out + [base + math.ctz(word)]
            word = word & math.wrapping_sub(word, 1)
        }
    }
    out
}

test "membership round-trips" {
    let mut b = new(200)
    b = add(b, 0)
    b = add(b, 63)
    b = add(b, 64)
    b = add(b, 199)
    testing.assert_eq(has(b, 63), true)
    testing.assert_eq(has(b, 64), true)
    testing.assert_eq(has(b, 1), false)
    b = remove(b, 63)
    testing.assert_eq(has(b, 63), false)
    testing.assert_eq(has(b, 64), true)
}

test "set algebra" {
    let mut a = new(128)
    let mut b = new(128)
    a = add(a, 1)
    a = add(a, 70)
    b = add(b, 70)
    b = add(b, 100)
    testing.assert_eq(to_list(union(a, b)), [1, 70, 100])
    testing.assert_eq(to_list(intersect(a, b)), [70])
    testing.assert_eq(to_list(difference(a, b)), [1])
}

test "adding twice is once" {
    let mut b = new(10)
    b = add(b, 5)
    b = add(b, 5)
    testing.assert_eq(to_list(b), [5])
}

test "the top bit of a word is a member like any other" {
    let mut b = new(130)
    b = add(b, 63)
    b = add(b, 127)
    b = add(b, 128)
    testing.assert_eq(has(b, 63), true)
    testing.assert_eq(has(b, 62), false)
    testing.assert_eq(count(b), 3)
    testing.assert_eq(to_list(b), [63, 127, 128])
    b = remove(b, 63)
    testing.assert_eq(to_list(b), [127, 128])
}

test "full words survive the set algebra" {
    let mut a = new(256)
    let mut b = new(256)
    for i in range(0, 256) { a = add(a, i) }
    for i in range(0, 128) { b = add(b, i * 2) }
    testing.assert_eq(count(union(a, b)), 256)
    testing.assert_eq(count(intersect(a, b)), 128)
    testing.assert_eq(first(to_list(difference(a, b))), 1)
    testing.assert_eq(count(difference(a, b)), 128)
}
