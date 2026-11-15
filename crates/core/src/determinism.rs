//! Determinism helpers (specification §7.6): seeded random numbers and stable hashes.

/// 64-bit FNV-1a: a stable hash, the same on every machine and every Rust version, used to seed
/// random sampling. Not for security.
pub fn stable_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The seed for one page, one step and one sub-step (for example a side and a pyramid level),
/// as A10 and A27 need (§7.6).
pub fn seed_for(page_id: u64, step: crate::Step, part: u64) -> u64 {
    let mut bytes = Vec::with_capacity(24);
    bytes.extend_from_slice(&page_id.to_le_bytes());
    bytes.extend_from_slice(step.to_string().as_bytes());
    bytes.extend_from_slice(&part.to_le_bytes());
    stable_hash(&bytes)
}

/// The xorshift64* generator (Vigna 2016), the only random source in ScanCraft (§7.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xorshift64Star {
    state: u64,
}

impl Xorshift64Star {
    /// A generator from `seed`. The all-zero state never changes, so a zero seed is replaced by a
    /// fixed non-zero constant.
    pub fn new(seed: u64) -> Self {
        Self { state: if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed } }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A value in [0, 1) with 53 random bits.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A value in 0..n, by Lemire's multiply-shift (a negligible bias for the small n used here).
    /// Returns 0 when n is 0.
    pub fn below(&mut self, n: u64) -> u64 {
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::{Xorshift64Star, seed_for, stable_hash};
    use crate::Step;

    #[test]
    fn fnv1a_known_values() {
        assert_eq!(stable_hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn xorshift_is_reproducible_and_never_stuck() {
        let mut a = Xorshift64Star::new(7);
        let mut b = Xorshift64Star::new(7);
        let xs: Vec<u64> = (0..5).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..5).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        let mut z = Xorshift64Star::new(0);
        assert_ne!(z.next_u64(), 0);
        assert!((0..1000).all(|_| z.next_f64() < 1.0));
        assert!((0..1000).all(|_| z.below(10) < 10));
    }

    #[test]
    fn seeds_differ_by_step_and_part() {
        assert_ne!(seed_for(1, Step::A10, 0), seed_for(1, Step::A10, 1));
        assert_ne!(seed_for(1, Step::A10, 0), seed_for(1, Step::A27, 0));
        assert_eq!(seed_for(3, Step::A27, 2), seed_for(3, Step::A27, 2));
    }
}
