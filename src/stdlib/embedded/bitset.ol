// bitset — a fixed-capacity set of small non-negative integers, written
// in olang: the `collections.bitset` submodule. Fully qualified with no
// `use`, or `use collections { bitset }` for the short name.
//
// Sixty-three members per word: membership, insertion, and removal are
// one index plus one mask, and whole-set algebra (union, intersect,
// difference) runs a word at a time. Sixty-three rather than sixty-four
// because olang's Int is signed — bit 63 is the sign, and a mask that
// reaches it would overflow. This is the structure behind sieves,
// visited-sets over dense ids, and set algebra where the universe is
// known in advance.
//
// Representation: one flat list, `[nbits, w0, w1, ...]` — the capacity,
// then ceil(nbits/63) words of bits, bit `i` at word `i / 63`, position
// `i % 63`. Writes go through the sole-owner fusion, so
// `b = bitset.add(b, i)` is O(1) in place under the shared calling
// convention: rebind through every write, read without rebinding.
//
// Bit positions must be in `0 <= i < nbits`; anything else raises the
// list's own index error — a bad position is a caller's bug, not data.

// A new, empty set with capacity for members 0..n-1.
share fn new(n) = {
    let mut b = col.filled(1 + (n + 62) / 63, 0)
    b = col.set(b, 0, n)
    b
}

// The capacity the set was created with.
share fn capacity(b) = b[0]

// True when `i` is a member.
share fn has(b, i) = {
    let word = b[1 + i / 63]
    (word / pow2(i % 63)) % 2 == 1
}

// The set with `i` added: `b = bitset.add(b, i)`.
share fn add(b, i) = {
    let at = 1 + i / 63
    let mask = pow2(i % 63)
    if (b[at] / mask) % 2 == 1 => { b }
    else => {
        b = col.set(b, at, b[at] + mask)
        b
    }
}

// The set with `i` removed: `b = bitset.remove(b, i)`.
share fn remove(b, i) = {
    let at = 1 + i / 63
    let mask = pow2(i % 63)
    if (b[at] / mask) % 2 == 1 => {
        b = col.set(b, at, b[at] - mask)
        b
    }
    else => { b }
}

// The number of members. Walks each word's set bits.
share fn count(b) = {
    let mut total = 0
    for w in range(1, len(b)) {
        let mut word = b[w]
        while word != 0 {
            total = total + word % 2
            word = word / 2
        }
    }
    total
}

// The union of two sets of equal capacity: members of either.
share fn union(a, b) = merge_words(a, b, (x, y) => bit_or(x, y))

// The intersection of two sets of equal capacity: members of both.
share fn intersect(a, b) = merge_words(a, b, (x, y) => bit_and(x, y))

// The difference of two sets of equal capacity: members of `a` not in `b`.
share fn difference(a, b) = merge_words(a, b, (x, y) => x - bit_and(x, y))

// The members, in ascending order, as a list of Ints.
share fn to_list(b) = {
    let mut out = []
    for w in range(1, len(b)) {
        let base = (w - 1) * 63
        let mut word = b[w]
        let mut bit = 0
        while word != 0 {
            if word % 2 == 1 => { out = out + [base + bit] }
            word = word / 2
            bit = bit + 1
        }
    }
    out
}

// ── word helpers ──────────────────────────────────────────────────────
// olang has no bitwise operators; these build the three operations the
// set algebra needs from arithmetic, one bit pair at a time. Word loops
// bound the cost: at most 63 steps per word regardless of set size.

// 2^k for 0 <= k <= 62 — always below the sign bit, by the 63-bit word
// layout above.
fn pow2(k) = {
    let mut v = 1
    for step in range(0, k) { v = v * 2 }
    v
}

fn bit_and(x, y) = {
    let mut a = x
    let mut b = y
    let mut out = 0
    let mut place = 1
    while a != 0 && b != 0 {
        if a % 2 == 1 && b % 2 == 1 => { out = out + place }
        a = a / 2
        b = b / 2
        place = place * 2
    }
    out
}

fn bit_or(x, y) = {
    let mut a = x
    let mut b = y
    let mut out = 0
    let mut place = 1
    while a != 0 || b != 0 {
        if a % 2 == 1 || b % 2 == 1 => { out = out + place }
        a = a / 2
        b = b / 2
        place = place * 2
    }
    out
}

fn merge_words(a, b, f) = {
    let mut out = a
    for w in range(1, len(a)) {
        out = col.set(out, w, f(a[w], b[w]))
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
