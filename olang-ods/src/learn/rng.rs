//! A seeded, splittable random stream for the learning kernels.
//!
//! PCG-XSL-RR 128/64 (O'Neill's PCG64): a 128-bit LCG state with a
//! per-stream odd increment, output permuted to 64 bits. Two streams
//! with different `stream` ids are independent sequences of the same
//! seed, so a forest gives tree `t` the stream `t` and draws the same
//! trees in any order, on any number of threads.
//!
//! Nothing here touches olang's global `random` stream: a kernel gets a
//! seed from its caller (who draws one from the global stream only when
//! the program gave none).

const MULT: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

/// SplitMix64: spreads a small seed (7, 42) over all 64 bits, so seeds
/// that differ in one bit start far apart.
pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Clone, Debug)]
pub struct Pcg {
    state: u128,
    inc: u128,
}

impl Pcg {
    /// Stream `stream` of `seed`.
    pub fn new(seed: u64, stream: u64) -> Pcg {
        let s = splitmix64(seed);
        let s2 = splitmix64(s ^ 0xD1B5_4A32_D192_ED03);
        let inc = ((splitmix64(stream.wrapping_add(0x5851_F42D)) as u128) << 65)
            | ((stream as u128) << 1)
            | 1;
        let mut rng = Pcg { state: 0, inc };
        rng.step();
        rng.state = rng.state.wrapping_add(((s as u128) << 64) | s2 as u128);
        rng.step();
        rng
    }

    /// An independent child stream: what a kernel hands to sub-task `k`.
    pub fn child(seed: u64, k: u64) -> Pcg {
        Pcg::new(seed, k.wrapping_add(1))
    }

    #[inline]
    fn step(&mut self) {
        self.state = self.state.wrapping_mul(MULT).wrapping_add(self.inc);
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.step();
        let s = self.state;
        let rot = (s >> 122) as u32;
        let x = ((s >> 64) as u64) ^ (s as u64);
        x.rotate_right(rot)
    }

    /// Uniform in [0, 1), 53 bits.
    #[inline]
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in [0, n) (Lemire's multiply-shift; bias below
    /// 2^-64 · n, immaterial at these sizes). `n` must be > 0.
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    /// Standard normal (Box–Muller; one of the pair is discarded so the
    /// stream position depends only on how many draws were made).
    pub fn normal(&mut self) -> f64 {
        let mut u1 = self.uniform();
        while u1 <= f64::MIN_POSITIVE {
            u1 = self.uniform();
        }
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Fisher–Yates in place.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }

    /// A random permutation of 0..n.
    pub fn permutation(&mut self, n: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..n).collect();
        self.shuffle(&mut v);
        v
    }

    /// `k` distinct values from 0..n (partial Fisher–Yates), in draw order.
    pub fn choose(&mut self, n: usize, k: usize) -> Vec<usize> {
        let k = k.min(n);
        let mut v: Vec<usize> = (0..n).collect();
        for i in 0..k {
            let j = i + self.below(n - i);
            v.swap(i, j);
        }
        v.truncate(k);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Pcg::new(7, 0);
        let mut b = Pcg::new(7, 0);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn streams_differ() {
        let mut a = Pcg::new(7, 1);
        let mut b = Pcg::new(7, 2);
        let same = (0..64).filter(|_| a.next_u64() == b.next_u64()).count();
        assert_eq!(same, 0);
    }

    #[test]
    fn uniform_moments() {
        let mut r = Pcg::new(1, 0);
        let n = 200_000;
        let xs: Vec<f64> = (0..n).map(|_| r.uniform()).collect();
        let m = xs.iter().sum::<f64>() / n as f64;
        assert!((m - 0.5).abs() < 0.005, "{m}");
        let zs: Vec<f64> = (0..n).map(|_| r.normal()).collect();
        let m = zs.iter().sum::<f64>() / n as f64;
        let v = zs.iter().map(|z| (z - m) * (z - m)).sum::<f64>() / n as f64;
        assert!(m.abs() < 0.01 && (v - 1.0).abs() < 0.02, "{m} {v}");
    }

    #[test]
    fn below_is_in_range_and_covers() {
        let mut r = Pcg::new(3, 0);
        let mut seen = [0usize; 7];
        for _ in 0..7000 {
            seen[r.below(7)] += 1;
        }
        assert!(seen.iter().all(|&c| c > 800));
    }
}
