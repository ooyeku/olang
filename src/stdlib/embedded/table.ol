// table — a flat hash table with open addressing, written in olang:
// the `collections.table` submodule. Fully qualified with no `use`, or
// `use collections { table }` for the short name.
//
// Keys are Ints or Strings; values are any olang value. Against the
// builtin persistent Map, this table trades structural sharing for flat
// probing: no per-entry allocation, one contiguous list, and O(1)
// writes through the sole-owner fusion — the right shape for counting,
// indexing, and hot loops that hammer one table. Where snapshots and
// sharing matter more than write rate, the builtin `#{}` Map remains
// the right tool.
//
// Representation: one flat list, `[count, used, cap, s0,k0,v0, ...]` —
// live entries, occupied-or-tombstoned slots, slot count, then `cap`
// slot triples: state (0 empty, 1 full, 2 tombstone), key, value.
// Linear probing from the key's hash; a removal leaves a tombstone so
// later probes keep walking. The table rebuilds at double size when
// three quarters of its slots are used, which also sweeps tombstones —
// so probes stay short no matter the history.
//
// The shared calling convention: rebind through every write
// (`t = table.put(t, k, v)`); read (`get`, `has`, `size`) without
// rebinding. `get` returns `()` for a missing key — Unit is the
// language's absence value — so a stored `()` is indistinguishable from
// absence by `get` alone; `has` answers presence exactly.

// A new, empty table (with a small initial slot count).
share fn new() = {
    let mut t = col.filled(3 + 8 * 3, 0)
    t = col.set(t, 2, 8)
    t
}

// The number of live entries.
share fn size(t) = t[0]

// True when `k` is present.
share fn has(t, k) = slot_of(t, k) >= 0

// The value under `k`, or `()` when absent.
share fn get(t, k) = {
    let at = slot_of(t, k)
    if at >= 0 => { t[at + 2] } else => { () }
}

// The value under `k`, or `fallback` when absent — the pattern for
// counters: `t = table.put(t, k, table.get_or(t, k, 0) + 1)`.
share fn get_or(t, k, fallback) = {
    let at = slot_of(t, k)
    if at >= 0 => { t[at + 2] } else => { fallback }
}

// The table with `k` set to `v` (inserted or overwritten):
// `t = table.put(t, k, v)`.
share fn put(t, k, v) = {
    t = grow_if_crowded(t)
    let cap = t[2]
    let mut probe = hash(k, cap)
    let mut target = -1
    while true {
        let at = 3 + probe * 3
        let state = t[at]
        if state == 0 => {
            // Empty ends the probe: insert here, or into an earlier
            // tombstone we passed.
            if target < 0 => { target = at }
            break
        }
        // Type first: equality between unrelated present types raises,
        // and an Int-keyed slot must not blow up a String probe.
        if state == 1 && typeof(t[at + 1]) == typeof(k) && t[at + 1] == k => {
            target = at
            break
        }
        // First tombstone on the path is the best insert slot, but the
        // probe must continue — the key may live further on.
        if state == 2 && target < 0 => { target = at }
        probe = (probe + 1) % cap
    }
    if t[target] != 1 || typeof(t[target + 1]) != typeof(k) || t[target + 1] != k => {
        // A genuinely new entry: count it, and count the slot as used
        // unless it recycles a tombstone.
        t = col.set(t, 0, t[0] + 1)
        if t[target] == 0 => { t = col.set(t, 1, t[1] + 1) }
    }
    t = col.set(t, target, 1)
    t = col.set(t, target + 1, k)
    t = col.set(t, target + 2, v)
    t
}

// The table with `k` removed (a no-op when absent):
// `t = table.remove(t, k)`.
share fn remove(t, k) = {
    let at = slot_of(t, k)
    if at < 0 => { t }
    else => {
        t = col.set(t, at, 2)
        t = col.set(t, at + 1, 0)
        t = col.set(t, at + 2, ())
        t = col.set(t, 0, t[0] - 1)
        t
    }
}

// The live keys, in slot order (an arbitrary but stable order between
// writes).
share fn keys(t) = {
    let mut out = []
    for slot in range(0, t[2]) {
        let at = 3 + slot * 3
        if t[at] == 1 => { out = out + [t[at + 1]] }
    }
    out
}

// The live values, in the same slot order as `keys`.
share fn values(t) = {
    let mut out = []
    for slot in range(0, t[2]) {
        let at = 3 + slot * 3
        if t[at] == 1 => { out = out + [t[at + 2]] }
    }
    out
}

// ── internals ─────────────────────────────────────────────────────────

// The slot base index of `k`'s live entry, or -1. Linear probe from the
// hash; an empty slot proves absence, a tombstone keeps the walk alive.
fn slot_of(t, k) = {
    let cap = t[2]
    let mut probe = hash(k, cap)
    let mut found = -1
    while true {
        let at = 3 + probe * 3
        let state = t[at]
        if state == 0 => { break }
        if state == 1 && typeof(t[at + 1]) == typeof(k) && t[at + 1] == k => {
            found = at
            break
        }
        probe = (probe + 1) % cap
    }
    found
}

// A slot index in 0..cap for an Int or String key. Multiplies stay
// bounded (operands reduced first) so hashing never overflows; String
// keys fold char codes polynomially. Anything else is a caller's bug.
fn hash(k, cap) = {
    if typeof(k) == "Int" => {
        let mixed = (k % 1000003) * 40503 + (k / 1000003) % 65521 * 631
        (mixed % cap + cap) % cap
    }
    else => {
        if typeof(k) == "String" => {
            let mut acc = 7
            for c in str.chars(k) {
                acc = (acc * 131 + str.char_code(c)) % 1000000007
            }
            acc % cap
        }
        else => {
            // A non-Int/String key is a caller's bug: abort with a
            // message rather than probing with garbage.
            testing.fail("table: keys must be Int or String, got " + typeof(k))
        }
    }
}

// Rebuild at double size once 3/4 of the slots are used (full or
// tombstoned). Reinserting live entries sweeps every tombstone, so
// probe chains reset no matter how churned the table was.
fn grow_if_crowded(t) = {
    if t[1] * 4 < t[2] * 3 => { t }
    else => {
        let cap = t[2]
        let mut bigger = col.filled(3 + cap * 2 * 3, 0)
        bigger = col.set(bigger, 2, cap * 2)
        for slot in range(0, cap) {
            let at = 3 + slot * 3
            if t[at] == 1 => { bigger = put(bigger, t[at + 1], t[at + 2]) }
        }
        bigger
    }
}

test "put, get, overwrite" {
    let mut t = new()
    t = put(t, "a", 1)
    t = put(t, "b", 2)
    t = put(t, "a", 10)
    testing.assert_eq(get(t, "a"), 10)
    testing.assert_eq(get(t, "b"), 2)
    testing.assert_eq(get(t, "c"), ())
    testing.assert_eq(size(t), 2)
}

test "int keys, growth, and removal" {
    let mut t = new()
    for i in range(0, 100) {
        t = put(t, i * 7, i)
    }
    testing.assert_eq(size(t), 100)
    testing.assert_eq(get(t, 693), 99)
    t = remove(t, 693)
    testing.assert_eq(has(t, 693), false)
    testing.assert_eq(size(t), 99)
    // A removed key can return; a tombstone must not hide the reinsert.
    t = put(t, 693, -1)
    testing.assert_eq(get(t, 693), -1)
}

test "counters via get_or" {
    let mut t = new()
    for w in ["a", "b", "a", "a"] {
        t = put(t, w, get_or(t, w, 0) + 1)
    }
    testing.assert_eq(get(t, "a"), 3)
    testing.assert_eq(get(t, "b"), 1)
}
