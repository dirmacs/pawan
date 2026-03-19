//! Bloom filter — probabilistic set membership, no false negatives, tunable FPR.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct BloomFilter {
    bits: Vec<u64>,
    k: usize,  // number of hash functions
    m: usize,  // number of bits
}

impl BloomFilter {
    /// capacity: expected number of elements, fpr: target false positive rate (0..1)
    pub fn new(capacity: usize, fpr: f64) -> Self {
        let m = optimal_m(capacity, fpr).max(64);
        let k = optimal_k(m, capacity).max(1);
        let words = (m + 63) / 64;
        BloomFilter { bits: vec![0u64; words], k, m }
    }

    pub fn insert<T: Hash>(&mut self, item: &T) {
        for i in 0..self.k {
            let bit = self.hash(item, i as u64);
            self.bits[bit / 64] |= 1u64 << (bit % 64);
        }
    }

    pub fn contains<T: Hash>(&self, item: &T) -> bool {
        (0..self.k).all(|i| {
            let bit = self.hash(item, i as u64);
            self.bits[bit / 64] & (1u64 << (bit % 64)) != 0
        })
    }

    fn hash<T: Hash>(&self, item: &T, seed: u64) -> usize {
        let mut h = DefaultHasher::new();
        seed.hash(&mut h);
        item.hash(&mut h);
        (h.finish() as usize) % self.m
    }
}

fn optimal_m(n: usize, p: f64) -> usize {
    (-(n as f64) * p.ln() / (2f64.ln().powi(2))).ceil() as usize
}

fn optimal_k(m: usize, n: usize) -> usize {
    ((m as f64 / n as f64) * 2f64.ln()).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_insert_contains() {
        let mut bf = BloomFilter::new(100, 0.01);
        bf.insert(&"hello");
        bf.insert(&"world");
        assert!(bf.contains(&"hello"));
        assert!(bf.contains(&"world"));
    }
    #[test] fn test_not_inserted() {
        let bf = BloomFilter::new(100, 0.01);
        assert!(!bf.contains(&"ghost"));
    }
    #[test] fn test_integers() {
        let mut bf = BloomFilter::new(1000, 0.01);
        for i in 0..100u32 { bf.insert(&i); }
        for i in 0..100u32 { assert!(bf.contains(&i), "missing {i}"); }
    }
    #[test] fn test_no_false_negatives() {
        // Items we insert must always be found
        let mut bf = BloomFilter::new(50, 0.05);
        let items: Vec<&str> = vec!["a","b","c","d","e","f","g","h"];
        for item in &items { bf.insert(item); }
        for item in &items { assert!(bf.contains(item)); }
    }
    #[test] fn test_large_capacity() {
        let mut bf = BloomFilter::new(10_000, 0.001);
        for i in 0..1000u32 { bf.insert(&i); }
        for i in 0..1000u32 { assert!(bf.contains(&i)); }
    }
}
