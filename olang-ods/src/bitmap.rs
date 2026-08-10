//! Validity bitmaps: one bit per element, 1 = valid, 0 = null.
//!
//! `None` at the series level means "no nulls anywhere" and costs nothing;
//! a bitmap only exists once a null does. Words are Arc-shared so cloning
//! a series never copies validity, and mutation goes through `Arc::make_mut`
//! (copy-on-write: in place when the refcount is 1).
//!
//! Invariant: bits at positions >= len are zero, so popcounts need no
//! tail masking.

use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    len: usize,
    words: Arc<Vec<u64>>,
}

impl Bitmap {
    fn word_count(len: usize) -> usize {
        len.div_ceil(64)
    }

    /// All-valid bitmap. Prefer `None` at the series level; this exists for
    /// kernels that must materialize before clearing individual bits.
    pub fn new_valid(len: usize) -> Self {
        let mut words = vec![u64::MAX; Self::word_count(len)];
        if !len.is_multiple_of(64)
            && let Some(last) = words.last_mut()
        {
            *last = (1u64 << (len % 64)) - 1;
        }
        Self {
            len,
            words: Arc::new(words),
        }
    }

    pub fn from_bools(bits: &[bool]) -> Self {
        let mut words = vec![0u64; Self::word_count(bits.len())];
        for (i, &valid) in bits.iter().enumerate() {
            if valid {
                words[i / 64] |= 1u64 << (i % 64);
            }
        }
        Self {
            len: bits.len(),
            words: Arc::new(words),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> bool {
        debug_assert!(i < self.len);
        self.words[i / 64] >> (i % 64) & 1 == 1
    }

    pub fn set(&mut self, i: usize, valid: bool) {
        debug_assert!(i < self.len);
        let words = Arc::make_mut(&mut self.words);
        if valid {
            words[i / 64] |= 1u64 << (i % 64);
        } else {
            words[i / 64] &= !(1u64 << (i % 64));
        }
    }

    pub fn count_valid(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// The raw validity words (64 elements per word, tail bits zero).
    /// Kernels iterate these directly: an all-ones word means a dense
    /// 64-element block, and set-bit iteration beats per-index `get`.
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    pub fn all_valid(&self) -> bool {
        self.count_valid() == self.len
    }

    /// Position-wise AND: valid only where both are valid.
    pub fn and(&self, other: &Bitmap) -> Bitmap {
        debug_assert_eq!(self.len, other.len);
        let words = self
            .words
            .iter()
            .zip(other.words.iter())
            .map(|(a, b)| a & b)
            .collect();
        Bitmap {
            len: self.len,
            words: Arc::new(words),
        }
    }
}

/// Merge two optional validities for an elementwise kernel: the result is
/// null wherever either input is. `None` means all-valid.
pub fn merge_validity(a: Option<&Bitmap>, b: Option<&Bitmap>) -> Option<Bitmap> {
    match (a, b) {
        (None, None) => None,
        (Some(v), None) | (None, Some(v)) => Some(v.clone()),
        (Some(a), Some(b)) => Some(a.and(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_count() {
        let mut bm = Bitmap::new_valid(70);
        assert!(bm.all_valid());
        bm.set(0, false);
        bm.set(69, false);
        assert!(!bm.get(0));
        assert!(bm.get(1));
        assert!(!bm.get(69));
        assert_eq!(bm.count_valid(), 68);
    }

    #[test]
    fn tail_bits_stay_zero() {
        let bm = Bitmap::new_valid(65);
        assert_eq!(bm.count_valid(), 65);
        let all = Bitmap::from_bools(&[true; 65]);
        assert_eq!(all.count_valid(), 65);
        assert_eq!(bm, all);
    }

    #[test]
    fn and_merges_nulls() {
        let a = Bitmap::from_bools(&[true, false, true, true]);
        let b = Bitmap::from_bools(&[true, true, false, true]);
        let c = a.and(&b);
        assert_eq!(
            (0..4).map(|i| c.get(i)).collect::<Vec<_>>(),
            vec![true, false, false, true]
        );
    }

    #[test]
    fn copy_on_write_shares_until_set() {
        let a = Bitmap::new_valid(64);
        let mut b = a.clone();
        assert!(Arc::ptr_eq(&a.words, &b.words));
        b.set(3, false);
        assert!(!Arc::ptr_eq(&a.words, &b.words));
        assert!(a.get(3));
    }
}
