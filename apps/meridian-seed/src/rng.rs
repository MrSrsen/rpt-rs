//! Deterministic random source.
//!
//! Wraps [`ChaCha8Rng`] and exposes only helpers built directly on the raw
//! 64-bit output stream. The ChaCha8 keystream is stable across platforms and
//! `rand_chacha` versions; the range/selection helpers here are hand-rolled
//! (Lemire's multiply-shift) rather than delegating to `rand`'s distributions,
//! whose algorithms are *not* guaranteed stable across releases. That keeps the
//! generated corpus byte-identical everywhere.

use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

/// A deterministic, platform-stable pseudo-random source.
pub(crate) struct Rng {
    inner: ChaCha8Rng,
}

impl std::fmt::Debug for Rng {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Rng(ChaCha8)")
    }
}

impl Rng {
    /// Seed the stream from a single `u64` constant.
    pub(crate) fn from_seed(seed: u64) -> Self {
        Self {
            inner: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// The next raw 64-bit word of the keystream.
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    /// A uniformly-distributed value in `0..n` (Lemire's multiply-shift; the
    /// residual bias is negligible for the small `n` used here and, crucially,
    /// deterministic).
    fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// A uniform integer in the inclusive range `[lo, hi]`.
    pub(crate) fn int(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(hi >= lo);
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    /// A uniform `f64` in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        // 53-bit mantissa of precision.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A uniform `f64` in `[lo, hi)`.
    pub(crate) fn real(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.unit() * (hi - lo)
    }

    /// `true` with probability `p`.
    pub(crate) fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Pick an index into a slice of length `len` (`len` must be > 0).
    pub(crate) fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    /// Borrow a uniformly-chosen element of a non-empty slice.
    pub(crate) fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.index(xs.len())]
    }

    /// A Gaussian deviate via Box–Muller (a true normal, with real tails).
    pub(crate) fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        let u1 = self.unit().max(1e-12);
        let u2 = self.unit();
        let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        mean + sd * z
    }

    /// A normal deviate clamped to `[lo, hi]` (a bounded bell around `mean`).
    pub(crate) fn bounded_normal(&mut self, mean: f64, sd: f64, lo: f64, hi: f64) -> f64 {
        self.normal(mean, sd).clamp(lo, hi)
    }

    /// A log-normal (right-skewed) positive value: `median · exp(σ·Z)`.
    pub(crate) fn lognormal(&mut self, median: f64, sigma: f64) -> f64 {
        median * (sigma * self.normal(0.0, 1.0)).exp()
    }

    /// A Poisson count with rate `lambda` (Knuth's method).
    pub(crate) fn poisson(&mut self, lambda: f64) -> i64 {
        let threshold = (-lambda).exp();
        let mut k = 0i64;
        let mut p = 1.0;
        loop {
            k += 1;
            p *= self.unit();
            if p <= threshold {
                break;
            }
        }
        k - 1
    }

    /// A Pareto (power-law) value with scale `x_min` and shape `alpha`
    /// (smaller `alpha` → heavier tail; ~1.16 gives the classic 80/20 split).
    pub(crate) fn pareto(&mut self, x_min: f64, alpha: f64) -> f64 {
        let u = self.unit().max(1e-12);
        x_min / u.powf(1.0 / alpha)
    }

    /// A weighted-categorical index into `weights` (shares need not sum to 1).
    pub(crate) fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u32 = weights.iter().sum();
        debug_assert!(total > 0);
        let mut pick = self.int(0, i64::from(total) - 1) as u32;
        for (i, w) in weights.iter().enumerate() {
            if pick < *w {
                return i;
            }
            pick -= *w;
        }
        weights.len() - 1
    }
}
