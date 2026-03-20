//! Pairing Heap — min-heap with O(1) amortised push/merge, O(log n) amortised pop.
//!
//! Two-pass merge for pop: pair children left-to-right, then merge right-to-left.

type Link<T> = Option<Box<PairNode<T>>>;

struct PairNode<T> {
    val: T,
    children: Vec<Box<PairNode<T>>>,
}

fn merge<T: Ord>(a: Link<T>, b: Link<T>) -> Link<T> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(mut a), Some(mut b)) => {
            if b.val < a.val {
                std::mem::swap(&mut a, &mut b);
            }
            a.children.push(b);
            Some(a)
        }
    }
}

pub struct PairingHeap<T: Ord> {
    root: Link<T>,
    len: usize,
}

impl<T: Ord> PairingHeap<T> {
    pub fn new() -> Self { PairingHeap { root: None, len: 0 } }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn push(&mut self, val: T) {
        let node = Some(Box::new(PairNode { val, children: vec![] }));
        self.root = merge(self.root.take(), node);
        self.len += 1;
    }

    pub fn peek(&self) -> Option<&T> {
        self.root.as_ref().map(|n| &n.val)
    }

    pub fn pop(&mut self) -> Option<T> {
        let node = self.root.take()?;
        self.len -= 1;
        let PairNode { val, children } = *node;
        // Two-pass merge
        let mut paired: Vec<Link<T>> = Vec::new();
        let mut iter = children.into_iter();
        // Pass 1: pair adjacent children left-to-right
        loop {
            match (iter.next(), iter.next()) {
                (Some(a), Some(b)) => paired.push(merge(Some(a), Some(b))),
                (Some(a), None) => { paired.push(Some(a)); break; }
                _ => break,
            }
        }
        // Pass 2: merge right-to-left
        self.root = paired.into_iter().rev().fold(None, |acc, x| merge(acc, x));
        Some(val)
    }
}

impl<T: Ord> Default for PairingHeap<T> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop() {
        let mut h = PairingHeap::new();
        for v in [5i32, 3, 8, 1, 4] { h.push(v); }
        let mut out = vec![];
        while let Some(v) = h.pop() { out.push(v); }
        assert_eq!(out, vec![1, 3, 4, 5, 8]);
    }

    #[test]
    fn test_empty() {
        let mut h: PairingHeap<i32> = PairingHeap::new();
        assert!(h.is_empty());
        assert_eq!(h.pop(), None);
        assert_eq!(h.peek(), None);
    }

    #[test]
    fn test_single() {
        let mut h = PairingHeap::new();
        h.push(42i32);
        assert_eq!(h.peek(), Some(&42));
        assert_eq!(h.pop(), Some(42));
        assert!(h.is_empty());
    }

    #[test]
    fn test_len() {
        let mut h = PairingHeap::new();
        for i in 0..5i32 { h.push(i); }
        assert_eq!(h.len(), 5);
        h.pop(); h.pop();
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn test_reverse() {
        let mut h = PairingHeap::new();
        for i in (0..10i32).rev() { h.push(i); }
        let mut out = vec![];
        while let Some(v) = h.pop() { out.push(v); }
        assert_eq!(out, (0..10i32).collect::<Vec<_>>());
    }

    #[test]
    fn test_all_same() {
        let mut h = PairingHeap::new();
        for _ in 0..5 { h.push(7i32); }
        let mut out = vec![];
        while let Some(v) = h.pop() { out.push(v); }
        assert_eq!(out, vec![7; 5]);
    }

    #[test]
    fn test_stress() {
        let mut h = PairingHeap::new();
        let vals: Vec<i32> = (0..50).map(|i| (i * 31 + 7) % 50).collect();
        for v in vals { h.push(v); }
        let mut prev = i32::MIN;
        let mut count = 0;
        while let Some(v) = h.pop() {
            assert!(v >= prev, "{v} < {prev}");
            prev = v;
            count += 1;
        }
        assert_eq!(count, 50);
    }
}
