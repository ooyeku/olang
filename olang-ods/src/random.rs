//! Seeded generators: `random.rng(seed)` and every kernel that draws
//! whole columns at once.
//!
//! The generator is PCG64 (PCG-XSL-RR 128/64, O'Neill), seeded exactly as
//! `learn`'s stream 0 is, so `random.rng(7)` and `learn.split(…, seed: 7)`
//! start from the same state. A generator is independent of olang's
//! global `random` stream: drawing from one never moves the other.
//!
//! **Draws cost a fixed number of steps.** Every sampler here reads a
//! known number of 64-bit outputs per value (a uniform, a normal pair, an
//! inverse-CDF table lookup), so a column of `n` values advances the
//! generator by a known amount. That makes a large draw parallel *and*
//! identical to the sequential one: the column is cut into fixed blocks,
//! block `b` starts from the generator jumped ahead by its offset (PCG's
//! `advance`, O(log n)), and the generator ends where the last block
//! does. The thread count never changes a value.
//!
//! **Child streams.** `spawn(k)` hands out `k` generators on distinct
//! PCG streams (distinct odd increments), seeded from the parent's next
//! output: independent sequences, the same ones on every run, whatever
//! thread later uses each.

#[cfg(feature = "parallel")]
use rayon::prelude::*;

const MULT: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

/// Values per block of a parallel draw. Fixed, so the blocks (and so the
/// values) never depend on the thread count.
pub const BLOCK: usize = 1 << 16;

/// SplitMix64: spreads a small seed over all 64 bits.
pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcg64 {
    state: u128,
    inc: u128,
}

impl Pcg64 {
    /// The generator `random.rng(seed)` makes: stream 0 of `seed`.
    pub fn new(seed: u64) -> Pcg64 {
        Pcg64::with_stream(seed, 0)
    }

    /// Stream `stream` of `seed` (`learn::Pcg::new`'s seeding, bit for bit).
    pub fn with_stream(seed: u64, stream: u64) -> Pcg64 {
        let s = splitmix64(seed);
        let s2 = splitmix64(s ^ 0xD1B5_4A32_D192_ED03);
        let inc = ((splitmix64(stream.wrapping_add(0x5851_F42D)) as u128) << 65)
            | ((stream as u128) << 1)
            | 1;
        let mut rng = Pcg64 { state: 0, inc };
        rng.step();
        rng.state = rng.state.wrapping_add(((s as u128) << 64) | s2 as u128);
        rng.step();
        rng
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

    /// Uniform in [0, 1), 53 bits. One step.
    #[inline]
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in (0, 1]: never zero, so its log is finite. One step.
    #[inline]
    pub fn uniform_open(&mut self) -> f64 {
        ((self.next_u64() >> 11) + 1) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in [0, n), Lemire's multiply-shift. One step.
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }

    /// Jump `delta` steps ahead in O(log delta) (Brown's algorithm, as
    /// PCG's `advance`).
    pub fn advance(&mut self, mut delta: u128) {
        let mut cur_mult = MULT;
        let mut cur_plus = self.inc;
        let mut acc_mult: u128 = 1;
        let mut acc_plus: u128 = 0;
        while delta > 0 {
            if delta & 1 == 1 {
                acc_mult = acc_mult.wrapping_mul(cur_mult);
                acc_plus = acc_plus.wrapping_mul(cur_mult).wrapping_add(cur_plus);
            }
            cur_plus = cur_mult.wrapping_add(1).wrapping_mul(cur_plus);
            cur_mult = cur_mult.wrapping_mul(cur_mult);
            delta >>= 1;
        }
        self.state = acc_mult.wrapping_mul(self.state).wrapping_add(acc_plus);
    }

    /// `k` independent child generators, on streams 1..=k of a seed drawn
    /// from this one (which moves by one step).
    pub fn spawn(&mut self, k: usize) -> Vec<Pcg64> {
        let seed = self.next_u64();
        (0..k as u64)
            .map(|i| Pcg64::with_stream(seed, i + 1))
            .collect()
    }

    /// A seed for a kernel that seeds itself (`learn`'s): one step.
    pub fn draw_seed(&mut self) -> u64 {
        self.next_u64()
    }

    /// Standard normal pair by Box–Muller: two steps for two values.
    #[inline]
    fn normal_pair(&mut self) -> (f64, f64) {
        let u1 = self.uniform_open();
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let (s, c) = (std::f64::consts::TAU * u2).sin_cos();
        (r * c, r * s)
    }
}

/// Fill `n` values from `rng`, `per` steps a value (`pair`: the sampler
/// writes two values per call and a column of odd length rounds up), in
/// fixed blocks run in parallel when `par`. The values and the
/// generator's final state are those of one sequential pass.
fn blocks<T, F>(rng: &mut Pcg64, n: usize, per: u128, par: bool, fill: F) -> Vec<T>
where
    T: Copy + Default + Send,
    F: Fn(&mut Pcg64, &mut [T]) + Sync,
{
    let mut out = vec![T::default(); n];
    if n == 0 {
        return out;
    }
    let start = rng.clone();
    let n_blocks = n.div_ceil(BLOCK);
    let at = |b: usize| {
        let mut r = start.clone();
        r.advance(per * (b * BLOCK) as u128);
        r
    };
    #[cfg(feature = "parallel")]
    if par && n_blocks > 1 {
        let ends: Vec<Pcg64> = out
            .par_chunks_mut(BLOCK)
            .enumerate()
            .map(|(b, chunk)| {
                let mut r = at(b);
                fill(&mut r, chunk);
                r
            })
            .collect();
        *rng = ends.into_iter().last().expect("n > 0");
        return out;
    }
    let _ = (par, n_blocks);
    let mut r = at(0);
    for chunk in out.chunks_mut(BLOCK) {
        fill(&mut r, chunk);
    }
    *rng = r;
    out
}

/// `n` uniforms in [lo, hi).
pub fn uniform(rng: &mut Pcg64, n: usize, lo: f64, hi: f64, par: bool) -> Vec<f64> {
    let w = hi - lo;
    blocks(rng, n, 1, par, |r, out: &mut [f64]| {
        for v in out {
            *v = lo + w * r.uniform();
        }
    })
}

/// `n` normals with mean `mu` and standard deviation `sd`. Box–Muller
/// pairs: two steps every two values, a lone last value two steps too.
pub fn normal(rng: &mut Pcg64, n: usize, mu: f64, sd: f64, par: bool) -> Vec<f64> {
    blocks(rng, n, 1, par, |r, out: &mut [f64]| {
        let mut chunks = out.chunks_exact_mut(2);
        for pair in &mut chunks {
            let (a, b) = r.normal_pair();
            pair[0] = mu + sd * a;
            pair[1] = mu + sd * b;
        }
        if let [last] = chunks.into_remainder() {
            *last = mu + sd * r.normal_pair().0;
        }
    })
}

/// `n` lognormals: `exp` of normals with mean `mu` and sd `sigma` (the
/// log's parameters, as NumPy's `lognormal`).
pub fn lognormal(rng: &mut Pcg64, n: usize, mu: f64, sigma: f64, par: bool) -> Vec<f64> {
    let mut v = normal(rng, n, mu, sigma, par);
    for x in v.iter_mut() {
        *x = x.exp();
    }
    v
}

/// `n` integers in [lo, hi). One step each.
pub fn integers(rng: &mut Pcg64, n: usize, lo: i64, hi: i64, par: bool) -> Vec<i64> {
    let span = (hi as i128 - lo as i128) as u64;
    blocks(rng, n, 1, par, |r, out: &mut [i64]| {
        for v in out {
            *v = lo.wrapping_add(r.below(span) as i64);
        }
    })
}

/// A discrete inverse CDF over the integers `offset..offset + cdf.len()`:
/// one uniform a value, a binary search.
struct Table {
    offset: i64,
    cdf: Vec<f64>,
}

impl Table {
    #[inline]
    fn draw(&self, u: f64) -> i64 {
        // The first k with cdf[k] > u.
        let k = self.cdf.partition_point(|&c| c <= u);
        self.offset + k.min(self.cdf.len() - 1) as i64
    }

    fn sample(&self, rng: &mut Pcg64, n: usize, par: bool) -> Vec<i64> {
        blocks(rng, n, 1, par, |r, out: &mut [i64]| {
            for v in out {
                *v = self.draw(r.uniform());
            }
        })
    }

    /// The table of a pmf over `lo..=hi`, normalized.
    fn of(lo: i64, hi: i64, pmf: impl Fn(i64) -> f64) -> Table {
        let mut cdf = Vec::with_capacity((hi - lo + 1) as usize);
        let mut acc = 0.0;
        for k in lo..=hi {
            acc += pmf(k);
            cdf.push(acc);
        }
        let total = acc;
        for c in cdf.iter_mut() {
            *c /= total;
        }
        if let Some(last) = cdf.last_mut() {
            *last = 1.0;
        }
        Table { offset: lo, cdf }
    }
}

fn ln_factorial(k: i64) -> f64 {
    crate::dist::ln_gamma(k as f64 + 1.0)
}

/// `n` Poisson counts with mean `lam` (≥ 0): an exact inverse CDF over the
/// counts within 40 standard deviations of the mean.
pub fn poisson(rng: &mut Pcg64, n: usize, lam: f64, par: bool) -> Vec<i64> {
    if lam == 0.0 {
        // Still one step a value, so the stream moves as for any mean.
        rng.advance(n as u128);
        return vec![0; n];
    }
    let sd = lam.sqrt();
    let lo = (lam - 40.0 * sd - 10.0).floor().max(0.0) as i64;
    let hi = (lam + 40.0 * sd + 10.0).ceil() as i64;
    let ln_lam = lam.ln();
    let table = Table::of(lo, hi, |k| {
        (k as f64 * ln_lam - lam - ln_factorial(k)).exp()
    });
    table.sample(rng, n, par)
}

/// `n` binomial counts of `trials` trials with success chance `p`.
pub fn binomial(rng: &mut Pcg64, n: usize, trials: i64, p: f64, par: bool) -> Vec<i64> {
    if p <= 0.0 || p >= 1.0 || trials == 0 {
        rng.advance(n as u128);
        let v = if p >= 1.0 { trials } else { 0 };
        return vec![v; n];
    }
    let mean = trials as f64 * p;
    let sd = (mean * (1.0 - p)).sqrt();
    let lo = ((mean - 40.0 * sd - 10.0).floor().max(0.0)) as i64;
    let hi = ((mean + 40.0 * sd + 10.0).ceil() as i64).min(trials);
    let (lp, lq) = (p.ln(), (1.0 - p).ln());
    let lnf_n = ln_factorial(trials);
    let table = Table::of(lo, hi, |k| {
        (lnf_n - ln_factorial(k) - ln_factorial(trials - k)
            + k as f64 * lp
            + (trials - k) as f64 * lq)
            .exp()
    });
    table.sample(rng, n, par)
}

/// `n` draws by the inverse of a continuous CDF (`ppf`), one step each.
pub fn by_ppf<F>(rng: &mut Pcg64, n: usize, par: bool, ppf: F) -> Vec<f64>
where
    F: Fn(f64) -> f64 + Sync,
{
    blocks(rng, n, 1, par, |r, out: &mut [f64]| {
        for v in out {
            *v = ppf(r.uniform_open().min(1.0 - 1e-16));
        }
    })
}

/// `n` positions in `0..len`, with replacement, by weight when `weights`
/// is given (one step each), or without replacement (`n ≤ len`):
/// a partial Fisher–Yates, or by weight the Efraimidis–Spirakis keys
/// (one step per candidate).
pub fn choice(
    rng: &mut Pcg64,
    len: usize,
    n: usize,
    weights: Option<&[f64]>,
    replace: bool,
    par: bool,
) -> Result<Vec<usize>, String> {
    if len == 0 {
        return Err("choice: no values to choose from".to_string());
    }
    if let Some(w) = weights {
        if w.len() != len {
            return Err(format!("choice: {} weights for {} values", w.len(), len));
        }
        if w.iter().any(|x| !x.is_finite() || *x < 0.0) {
            return Err("choice: weights must be finite and non-negative".to_string());
        }
        if w.iter().sum::<f64>() <= 0.0 {
            return Err("choice: the weights sum to zero".to_string());
        }
    }
    if replace {
        return Ok(match weights {
            None => integers(rng, n, 0, len as i64, par)
                .into_iter()
                .map(|i| i as usize)
                .collect(),
            Some(w) => {
                let mut cdf = Vec::with_capacity(len);
                let mut acc = 0.0;
                for x in w {
                    acc += x;
                    cdf.push(acc);
                }
                let total = acc;
                let table = Table {
                    offset: 0,
                    cdf: cdf.into_iter().map(|c| c / total).collect(),
                };
                // A zero weight's cdf step is flat: partition_point skips it.
                table
                    .sample(rng, n, par)
                    .into_iter()
                    .map(|i| i as usize)
                    .collect()
            }
        });
    }
    if n > len {
        return Err(format!(
            "choice: {} values without replacement from {}",
            n, len
        ));
    }
    Ok(match weights {
        None => {
            let mut v: Vec<usize> = (0..len).collect();
            for i in 0..n {
                let j = i + rng.below((len - i) as u64) as usize;
                v.swap(i, j);
            }
            v.truncate(n);
            v
        }
        Some(w) => {
            let positive = w.iter().filter(|x| **x > 0.0).count();
            if n > positive {
                return Err(format!(
                    "choice: {} values without replacement, but only {} have weight",
                    n, positive
                ));
            }
            // key = u^(1/w); the n largest keys, largest first.
            let mut keyed: Vec<(f64, usize)> = w
                .iter()
                .enumerate()
                .map(|(i, &wi)| {
                    let u = rng.uniform_open();
                    let key = if wi > 0.0 {
                        u.ln() / wi
                    } else {
                        f64::NEG_INFINITY
                    };
                    (key, i)
                })
                .collect();
            keyed.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            keyed.truncate(n);
            keyed.into_iter().map(|(_, i)| i).collect()
        }
    })
}

/// A random permutation of `0..n` (Fisher–Yates, n − 1 steps).
pub fn permutation(rng: &mut Pcg64, n: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        v.swap(i, j);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeding_matches_learns_stream() {
        let mut a = Pcg64::new(7);
        let mut b = crate::learn::Pcg::new(7, 0);
        for _ in 0..50 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn advance_is_stepping() {
        let mut a = Pcg64::new(3);
        let mut b = a.clone();
        for _ in 0..12_345 {
            a.next_u64();
        }
        b.advance(12_345);
        assert_eq!(a, b);
    }

    #[test]
    fn parallel_draws_equal_sequential_ones() {
        let n = BLOCK * 3 + 17;
        let fs: [fn(&mut Pcg64, bool) -> Vec<f64>; 2] = [
            |r, par| normal(r, BLOCK * 3 + 17, 1.0, 2.0, par),
            |r, par| uniform(r, BLOCK * 3 + 17, -1.0, 1.0, par),
        ];
        for f in fs {
            let mut a = Pcg64::new(9);
            let mut b = Pcg64::new(9);
            let x = f(&mut a, false);
            let y = f(&mut b, true);
            assert_eq!(x.len(), n);
            assert_eq!(x, y);
            assert_eq!(a, b);
        }
        let mut a = Pcg64::new(9);
        let mut b = Pcg64::new(9);
        assert_eq!(
            poisson(&mut a, n, 3.5, false),
            poisson(&mut b, n, 3.5, true)
        );
        assert_eq!(a, b);
    }

    #[test]
    fn a_column_moves_the_stream_by_its_length() {
        let mut a = Pcg64::new(5);
        let mut b = Pcg64::new(5);
        let _ = uniform(&mut a, 1000, 0.0, 1.0, false);
        b.advance(1000);
        assert_eq!(a, b);
        // normals: an odd count rounds up to the pair
        let mut a = Pcg64::new(5);
        let mut b = Pcg64::new(5);
        let _ = normal(&mut a, 7, 0.0, 1.0, false);
        b.advance(8);
        assert_eq!(a, b);
    }

    #[test]
    fn moments() {
        let mut r = Pcg64::new(1);
        let n = 400_000;
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let z = normal(&mut r, n, 0.0, 1.0, false);
        let m = mean(&z);
        let v = z.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n as f64;
        assert!(m.abs() < 0.01 && (v - 1.0).abs() < 0.01, "{m} {v}");
        let p: Vec<f64> = poisson(&mut r, n, 4.0, false)
            .into_iter()
            .map(|k| k as f64)
            .collect();
        assert!((mean(&p) - 4.0).abs() < 0.02);
        let b: Vec<f64> = binomial(&mut r, n, 20, 0.3, false)
            .into_iter()
            .map(|k| k as f64)
            .collect();
        assert!((mean(&b) - 6.0).abs() < 0.02);
        let i = integers(&mut r, n, -3, 4, false);
        assert!(i.iter().all(|x| (-3..4).contains(x)));
    }

    #[test]
    fn weighted_choice_follows_the_weights() {
        let mut r = Pcg64::new(2);
        let picks = choice(&mut r, 3, 300_000, Some(&[1.0, 0.0, 3.0]), true, false).unwrap();
        let c0 = picks.iter().filter(|&&i| i == 0).count() as f64 / 300_000.0;
        assert!(picks.iter().all(|&i| i != 1));
        assert!((c0 - 0.25).abs() < 0.01, "{c0}");
        let distinct = choice(&mut r, 10, 10, Some(&[1.0; 10]), false, false).unwrap();
        let mut s = distinct.clone();
        s.sort();
        assert_eq!(s, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn spawned_streams_differ() {
        let mut r = Pcg64::new(1);
        let mut kids = r.spawn(3);
        let a: Vec<u64> = (0..8).map(|_| kids[0].next_u64()).collect();
        let b: Vec<u64> = (0..8).map(|_| kids[1].next_u64()).collect();
        assert!(a.iter().zip(&b).all(|(x, y)| x != y));
        let mut again = Pcg64::new(1).spawn(3);
        assert_eq!(again[0].next_u64(), a[0]);
    }
}
