//! A deterministic pseudo-random generator.
//!
//! Hand-rolled on purpose. The corpus must be **byte-identical forever**, on every machine and
//! at every future date, or a benchmark comparison across two sessions is meaningless. Pulling
//! in `rand` would tie the corpus to a crate version: `rand` has changed its generator and its
//! distribution algorithms more than once, and a benchmark that silently changes its own input
//! is worse than no benchmark.
//!
//! This is xorshift64*, which is more than adequate for choosing file names.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // A zero state is a fixed point for xorshift, so it must never be used.
        Self(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform in `0..n`. Rejection-sampled, so the distribution has no modulo bias.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0) has no valid answer");
        let n = n as u64;
        let limit = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < limit {
                return (v % n) as usize;
            }
        }
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// True with probability `percent`/100.
    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn range(&mut self, low: usize, high: usize) -> usize {
        debug_assert!(low <= high);
        low + self.below(high - low + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_produces_the_same_sequence() {
        let a: Vec<u64> = (0..64).map(|_| Rng::new(42).next_u64()).take(1).collect();
        let b: Vec<u64> = (0..64).map(|_| Rng::new(42).next_u64()).take(1).collect();
        assert_eq!(a, b);

        let mut x = Rng::new(7);
        let mut y = Rng::new(7);
        for _ in 0..1000 {
            assert_eq!(x.next_u64(), y.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut x = Rng::new(1);
        let mut y = Rng::new(2);
        assert_ne!(x.next_u64(), y.next_u64());
    }

    #[test]
    fn a_zero_seed_does_not_collapse() {
        // xorshift has zero as a fixed point: without the guard, seed 0 emits zeros forever.
        let mut r = Rng::new(0);
        let first = r.next_u64();
        let second = r.next_u64();
        assert_ne!(first, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut r = Rng::new(99);
        let mut seen = [false; 10];
        for _ in 0..10_000 {
            let v = r.below(10);
            assert!(v < 10);
            seen[v] = true;
        }
        assert!(seen.iter().all(|&s| s), "every value in range should occur");
    }

    #[test]
    fn below_one_is_always_zero() {
        let mut r = Rng::new(5);
        for _ in 0..100 {
            assert_eq!(r.below(1), 0);
        }
    }

    #[test]
    fn range_includes_both_ends() {
        let mut r = Rng::new(3);
        let mut low = false;
        let mut high = false;
        for _ in 0..1000 {
            let v = r.range(5, 8);
            assert!((5..=8).contains(&v));
            low |= v == 5;
            high |= v == 8;
        }
        assert!(low && high, "range must be inclusive at both ends");
    }
}
