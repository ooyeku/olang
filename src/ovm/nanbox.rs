//! NaN-boxing primitives — the P3 foundation layer.
//!
//! This module is the *packing scheme only*: one u64 that is either a
//! real f64 or a tagged payload smuggled inside the quiet-NaN space. It
//! is deliberately not wired into the VM yet — the value-model rewrite
//! builds on these primitives next, and every bit-level subtlety is
//! settled and pinned here first, where it is cheap to reason about.
//!
//! ## Layout
//!
//! An IEEE-754 double is a NaN when its exponent bits are all ones and
//! the mantissa is nonzero. Hardware only ever *produces* one quiet NaN
//! (0x7FF8_0000_0000_0000, possibly sign-flipped), which leaves the rest
//! of the NaN space — sign bit set, quiet bit set, 51 payload bits —
//! free for tagged values:
//!
//! ```text
//! 0xFFF9_xxxx_xxxx_xxxx   small integer (48-bit two's complement)
//! 0xFFFA_xxxx_xxxx_xxxx   heap pointer  (48-bit address)
//! 0xFFFB_0000_0000_000v   constants: 0 = false, 1 = true, 2 = unit
//! ```
//!
//! Everything else is a plain double, stored as its own bits. A real NaN
//! produced by arithmetic is canonicalized to 0x7FF8_0000_0000_0000 on
//! the way in so it can never collide with a tag (olang NaN carries no
//! observable payload: NaN == NaN is already false, and Display says
//! "NaN" regardless).
//!
//! ## The integer problem, settled
//!
//! olang integers are full i64, but a NaN payload holds 48 bits. The
//! scheme therefore splits them: values in [-2^47, 2^47) — every loop
//! counter, index, and realistic arithmetic operand — pack inline;
//! anything wider must live behind a heap pointer, exactly like a string.
//! `pack_i64` returns `Packed::NeedsHeap` for those, and the VM wiring
//! decides what to do (box, or refuse the function into bytecode). The
//! boundary cases (±2^47, i64::MIN/MAX) are pinned below.

/// Inline small-int range: [-2^47, 2^47).
pub const SMALL_INT_MIN: i64 = -(1 << 47);
pub const SMALL_INT_MAX: i64 = (1 << 47) - 1;

const QNAN: u64 = 0x7FF8_0000_0000_0000;
const TAG_MASK: u64 = 0xFFFF_0000_0000_0000;
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

const TAG_INT: u64 = 0xFFF9_0000_0000_0000;
const TAG_PTR: u64 = 0xFFFA_0000_0000_0000;
const TAG_CONST: u64 = 0xFFFB_0000_0000_0000;

const CONST_FALSE: u64 = TAG_CONST;
const CONST_TRUE: u64 = TAG_CONST | 1;
const CONST_UNIT: u64 = TAG_CONST | 2;

/// A packed value, or the admission that it needs a heap cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packed {
    Inline(NanBox),
    /// An i64 outside the small-int range: the caller boxes it.
    NeedsHeap,
}

/// One 64-bit slot holding a float, small int, bool, unit, or pointer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NanBox(u64);

/// What a NanBox holds, unpacked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unpacked {
    Float(f64),
    Int(i64),
    Bool(bool),
    Unit,
    Ptr(u64),
}

impl NanBox {
    /// Pack a float. Real NaNs canonicalize so they can't alias a tag.
    #[inline]
    pub fn from_f64(f: f64) -> Self {
        if f.is_nan() {
            NanBox(QNAN)
        } else {
            NanBox(f.to_bits())
        }
    }

    /// Pack an integer if it fits the inline range.
    #[inline]
    pub fn pack_i64(i: i64) -> Packed {
        if (SMALL_INT_MIN..=SMALL_INT_MAX).contains(&i) {
            Packed::Inline(NanBox(TAG_INT | ((i as u64) & PAYLOAD_MASK)))
        } else {
            Packed::NeedsHeap
        }
    }

    #[inline]
    pub fn from_bool(b: bool) -> Self {
        NanBox(if b { CONST_TRUE } else { CONST_FALSE })
    }

    #[inline]
    pub fn unit() -> Self {
        NanBox(CONST_UNIT)
    }

    /// Pack a 48-bit heap address. Callers guarantee the pointer fits —
    /// true for user-space addresses on x86-64 and aarch64, asserted in
    /// debug builds so a future platform can't silently truncate.
    #[inline]
    pub fn from_ptr(addr: u64) -> Self {
        debug_assert_eq!(addr & !PAYLOAD_MASK, 0, "pointer exceeds 48 bits");
        NanBox(TAG_PTR | (addr & PAYLOAD_MASK))
    }

    /// The raw bits (for the JIT, which will hold these in registers).
    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }

    #[inline]
    pub fn from_bits(bits: u64) -> Self {
        NanBox(bits)
    }

    #[inline]
    pub fn is_float(self) -> bool {
        // A double unless it sits in our tag space. The canonical quiet
        // NaN (and every hardware-produced NaN, canonicalized on entry)
        // stays a float.
        (self.0 & TAG_MASK) < TAG_INT || (self.0 & TAG_MASK) > TAG_CONST
    }

    #[inline]
    pub fn is_int(self) -> bool {
        self.0 & TAG_MASK == TAG_INT
    }

    #[inline]
    pub fn is_ptr(self) -> bool {
        self.0 & TAG_MASK == TAG_PTR
    }

    /// Unpack. Total: every bit pattern decodes to exactly one variant.
    #[inline]
    pub fn unpack(self) -> Unpacked {
        match self.0 & TAG_MASK {
            TAG_INT => {
                // Sign-extend from bit 47.
                let raw = (self.0 & PAYLOAD_MASK) as i64;
                Unpacked::Int((raw << 16) >> 16)
            }
            TAG_PTR => Unpacked::Ptr(self.0 & PAYLOAD_MASK),
            TAG_CONST => match self.0 {
                CONST_FALSE => Unpacked::Bool(false),
                CONST_TRUE => Unpacked::Bool(true),
                _ => Unpacked::Unit,
            },
            _ => Unpacked::Float(f64::from_bits(self.0)),
        }
    }
}

impl std::fmt::Debug for NanBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NanBox({:#018x} = {:?})", self.0, self.unpack())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_int(i: i64) {
        match NanBox::pack_i64(i) {
            Packed::Inline(b) => assert_eq!(b.unpack(), Unpacked::Int(i), "int {i}"),
            Packed::NeedsHeap => panic!("{i} should pack inline"),
        }
    }

    #[test]
    fn small_ints_round_trip() {
        for i in [
            0,
            1,
            -1,
            42,
            -42,
            SMALL_INT_MIN,
            SMALL_INT_MAX,
            SMALL_INT_MIN + 1,
            SMALL_INT_MAX - 1,
            1 << 40,
            -(1 << 40),
        ] {
            round_trip_int(i);
        }
    }

    #[test]
    fn wide_ints_need_heap() {
        for i in [
            SMALL_INT_MIN - 1,
            SMALL_INT_MAX + 1,
            i64::MIN,
            i64::MAX,
            1 << 48,
            -(1 << 48),
        ] {
            assert_eq!(NanBox::pack_i64(i), Packed::NeedsHeap, "int {i}");
        }
    }

    #[test]
    fn floats_round_trip_bit_exact() {
        for f in [
            0.0,
            -0.0,
            1.5,
            -1.5,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MIN,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            std::f64::consts::PI,
        ] {
            let b = NanBox::from_f64(f);
            match b.unpack() {
                Unpacked::Float(g) => {
                    assert_eq!(g.to_bits(), f.to_bits(), "float {f} must be bit-exact")
                }
                other => panic!("float {f} decoded as {other:?}"),
            }
        }
    }

    #[test]
    fn nan_canonicalizes_and_stays_float() {
        // Real arithmetic NaNs — whatever their payload — must decode as
        // Float(NaN), never alias a tag.
        #[allow(clippy::zero_divided_by_zero)] // producing arithmetic NaNs is the point
        let candidates = [
            f64::NAN,
            -f64::NAN,
            0.0 / 0.0_f64,
            f64::INFINITY - f64::INFINITY,
            (f64::INFINITY * 0.0),
            f64::from_bits(0x7FF8_DEAD_BEEF_0001), // payload NaN
            f64::from_bits(0xFFF9_0000_0000_002A), // bit pattern of a tagged int!
        ];
        for f in candidates {
            assert!(f.is_nan());
            let b = NanBox::from_f64(f);
            assert_eq!(b.bits(), super::QNAN, "must canonicalize");
            match b.unpack() {
                Unpacked::Float(g) => assert!(g.is_nan()),
                other => panic!("NaN decoded as {other:?}"),
            }
        }
    }

    #[test]
    fn bool_and_unit_round_trip() {
        assert_eq!(NanBox::from_bool(true).unpack(), Unpacked::Bool(true));
        assert_eq!(NanBox::from_bool(false).unpack(), Unpacked::Bool(false));
        assert_eq!(NanBox::unit().unpack(), Unpacked::Unit);
    }

    #[test]
    fn pointers_round_trip() {
        for p in [0u64, 8, 0x7FFF_FFFF_FFF8, PAYLOAD_MASK & !7] {
            let b = NanBox::from_ptr(p);
            assert_eq!(b.unpack(), Unpacked::Ptr(p), "ptr {p:#x}");
            assert!(b.is_ptr());
        }
    }

    #[test]
    fn tags_are_disjoint_from_reachable_floats() {
        // Randomized cross-check: any NanBox built from a float unpacks as
        // a float; any built from a small int unpacks as that int. A
        // deterministic LCG keeps this reproducible.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..1_000_000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let f = f64::from_bits(state);
            match NanBox::from_f64(f).unpack() {
                Unpacked::Float(g) => {
                    assert!(g.to_bits() == f.to_bits() || (f.is_nan() && g.is_nan()))
                }
                other => panic!("float bits {state:#x} decoded as {other:?}"),
            }
            let i = (state as i64) >> 17; // always in small-int range
            round_trip_int(i);
        }
    }

    #[test]
    fn a_nanbox_is_eight_bytes() {
        assert_eq!(std::mem::size_of::<NanBox>(), 8);
        assert_eq!(std::mem::size_of::<Option<NanBox>>(), 16); // no niche — by design
    }
}
