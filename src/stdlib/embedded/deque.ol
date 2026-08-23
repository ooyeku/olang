// deque — a double-ended queue over a ring buffer, written in olang and
// bundled into the binary. Automatically available: `deque.new()` works
// with no `use`.
//
// Push and pop at either end in O(1): the queue behind breadth-first
// search, sliding windows, work lists, and anywhere else "first in,
// first out" (or "last in") is the shape of the problem.
//
// Representation: one flat list, `[head, size, cap, slot0..slotCap-1]` —
// the index of the front element, the element count, the slot count, then
// the slots as a ring: element `k` lives at `(head + k) % cap`. Growth
// doubles the ring and re-lays the elements front-first; a popped slot is
// overwritten with `()` so the queue holds no reference to a value it no
// longer contains. Writes go through the sole-owner fusion under the
// shared calling convention — rebind through every write
// (`q = deque.push_back(q, x)`), read (`front`, `back`, `size`) without
// rebinding.
//
// `front`/`back`/`pop_front`/`pop_back` are undefined on an empty deque
// (they raise an index error): emptiness is the caller's check, the same
// contract as `heap`.

// A new, empty deque (with a small initial ring).
share fn new() = [0, 0, 4, (), (), (), ()]

// The number of elements.
share fn size(q) = q[1]

// True when the deque holds nothing.
share fn is_empty(q) = q[1] == 0

// The front element, without removing it.
share fn front(q) = {
    if q[1] == 0 => { q[len(q)] }  // one past the end: raises, by contract
    else => { q[3 + q[0]] }
}

// The back element, without removing it.
share fn back(q) = {
    if q[1] == 0 => { q[len(q)] }  // one past the end: raises, by contract
    else => { q[3 + (q[0] + q[1] - 1) % q[2]] }
}

// The deque with `x` appended at the back: `q = deque.push_back(q, x)`.
share fn push_back(q, x) = {
    q = grow_if_full(q)
    let at = (q[0] + q[1]) % q[2]
    q = col.set(q, 3 + at, x)
    q = col.set(q, 1, q[1] + 1)
    q
}

// The deque with `x` prepended at the front: `q = deque.push_front(q, x)`.
share fn push_front(q, x) = {
    q = grow_if_full(q)
    let at = (q[0] + q[2] - 1) % q[2]
    q = col.set(q, 3 + at, x)
    q = col.set(q, 0, at)
    q = col.set(q, 1, q[1] + 1)
    q
}

// The deque with its front element removed: read `front` first, then
// `q = deque.pop_front(q)`.
share fn pop_front(q) = {
    let head = q[0]
    q = col.set(q, 3 + head, ())
    q = col.set(q, 0, (head + 1) % q[2])
    q = col.set(q, 1, q[1] - 1)
    q
}

// The deque with its back element removed: read `back` first, then
// `q = deque.pop_back(q)`.
share fn pop_back(q) = {
    let at = (q[0] + q[1] - 1) % q[2]
    q = col.set(q, 3 + at, ())
    q = col.set(q, 1, q[1] - 1)
    q
}

// The elements front-to-back, as a plain list.
share fn to_list(q) = {
    let mut out = []
    for k in range(0, q[1]) {
        out = out + [q[3 + (q[0] + k) % q[2]]]
    }
    out
}

// Double the ring when full, re-laying elements front-first. Amortized
// O(1) per push; a deque that grows and shrinks reuses its high-water
// ring without shrinking it back.
fn grow_if_full(q) = {
    if q[1] < q[2] => { q }
    else => {
        let cap = q[2]
        let mut bigger = col.filled(3 + cap * 2, ())
        bigger = col.set(bigger, 0, 0)
        bigger = col.set(bigger, 1, q[1])
        bigger = col.set(bigger, 2, cap * 2)
        for k in range(0, q[1]) {
            bigger = col.set(bigger, 3 + k, q[3 + (q[0] + k) % cap])
        }
        bigger
    }
}

test "fifo order" {
    let mut q = new()
    q = push_back(q, 1)
    q = push_back(q, 2)
    q = push_back(q, 3)
    testing.assert_eq(front(q), 1)
    testing.assert_eq(back(q), 3)
    q = pop_front(q)
    testing.assert_eq(front(q), 2)
    testing.assert_eq(size(q), 2)
}

test "both ends and growth" {
    let mut q = new()
    // Push past the initial ring from both ends.
    for i in range(0, 6) {
        q = push_back(q, i)
        q = push_front(q, 0 - i)
    }
    testing.assert_eq(size(q), 12)
    testing.assert_eq(front(q), -5)
    testing.assert_eq(back(q), 5)
    testing.assert_eq(to_list(q), [-5, -4, -3, -2, -1, 0, 0, 1, 2, 3, 4, 5])
}

test "drain to empty and reuse" {
    let mut q = new()
    q = push_back(q, "a")
    q = pop_front(q)
    testing.assert_eq(is_empty(q), true)
    q = push_front(q, "b")
    testing.assert_eq(front(q), "b")
    testing.assert_eq(back(q), "b")
}
