//! Leftist Heap — min-heap with O(log n) merge, push, pop.
//!
//! Leftist property: rank(left) >= rank(right) at every node.
//! Rank = length of rightmost path to None.

type Link<T> = Option<Box<Node<T>>>;

struct Node<T> {
    val: T,
    rank: usize,
    left: Link<T>,
    right: Link<T>,
}

impl<T> Node<T> {
    fn singleton(val: T) -> Box<Self> {
        Box::new(Node { val, rank: 1, left: None, right: None })
    }
}

fn node_rank<T>(link: &Link<T>) -> usize {
    link.as_ref().map(|n| n.rank).unwrap_or(0)
}

fn merge<T: Ord>(a: Link<T>, b: Link<T>) -> Link<T> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(mut a), Some(mut b)) => {
            // Keep smaller root on top (min-heap)
            if b.val < a.val {
                std::mem::swap(&mut a, &mut b);
            }
            // Merge a's right child with b
            a.right = merge(a.right.take(), Some(b));
            // Restore leftist property: left rank >= right rank
            if node_rank(&a.left) < node_rank(&a.right) {
                std::mem::swap(&mut a.left, &mut a.right);
            }
            a.rank = 1 + node_rank(&a.right);
            Some(a)
        }
    }
}

pub struct LeftistHeap<T: Ord> {
    root: Link<T>,
    len: usize,
}

impl<T: Ord> LeftistHeap<T> {
    pub fn new() -> Self {
        LeftistHeap { root: None, len: 0 }
    }

    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn push(&mut self, val: T) {
        let node = Some(Node::singleton(val));
        self.root = merge(self.root.take(), node);
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T> {
        self.root.take().map(|node| {
            self.root = merge(node.left, node.right);
            self.len -= 1;
            node.val
        })
    }

    pub fn peek(&self) -> Option<&T> {
        self.root.as_ref().map(|n| &n.val)
    }
}

impl<T: Ord> Default for LeftistHeap<T> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop_order() {
        let mut h = LeftistHeap::new();
        for v in [3i32, 1, 4, 1, 5, 9, 2, 6] {
            h.push(v);
        }
        let mut out = vec![];
        while let Some(v) = h.pop() { out.push(v); }
        assert_eq!(out, vec![1, 1, 2, 3, 4, 5, 6, 9]);
    }

    #[test]
    fn test_empty() {
        let mut h: LeftistHeap<i32> = LeftistHeap::new();
        assert!(h.is_empty());
        assert_eq!(h.pop(), None);
        assert_eq!(h.peek(), None);
    }

    #[test]
    fn test_single() {
        let mut h = LeftistHeap::new();
        h.push(42i32);
        assert_eq!(h.peek(), Some(&42));
        assert_eq!(h.len(), 1);
        assert_eq!(h.pop(), Some(42));
        assert!(h.is_empty());
    }

    #[test]
    fn test_len_tracking() {
        let mut h = LeftistHeap::new();
        for i in 0..5i32 { h.push(i); }
        assert_eq!(h.len(), 5);
        h.pop(); h.pop();
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn test_reverse_order() {
        let mut h = LeftistHeap::new();
        for i in (0..10i32).rev() { h.push(i); }
        let mut out = vec![];
        while let Some(v) = h.pop() { out.push(v); }
        assert_eq!(out, (0..10i32).collect::<Vec<_>>());
    }

    #[test]
    fn test_all_same() {
        let mut h = LeftistHeap::new();
        for _ in 0..7 { h.push(5i32); }
        let mut out = vec![];
        while let Some(v) = h.pop() { out.push(v); }
        assert_eq!(out, vec![5; 7]);
    }

    #[test]
    fn test_stress_sorted() {
        let mut h = LeftistHeap::new();
        // Push in pseudo-random order
        let vals: Vec<i32> = (0..100).map(|i| (i * 37 + 13) % 100).collect();
        for v in vals { h.push(v); }
        let mut prev = i32::MIN;
        let mut count = 0;
        while let Some(v) = h.pop() {
            assert!(v >= prev, "{v} < {prev}");
            prev = v;
            count += 1;
        }
        assert_eq!(count, 100);
    }

    #[test]
    fn test_min_property() {
        let mut h = LeftistHeap::new();
        for v in [7i32, 2, 9, 1, 5] { h.push(v); }
        assert_eq!(h.peek(), Some(&1));
        h.pop();
        assert_eq!(h.peek(), Some(&2));
    }

    #[test]
    fn test_leftist_rank() {
        // After pushes, rank of root's right path should be minimal
        let mut h = LeftistHeap::new();
        for i in 0..20i32 { h.push(i); }
        // Verify rank invariant: right rank <= left rank at every node
        fn check<T: Ord>(link: &Link<T>) {
            if let Some(n) = link {
                assert!(node_rank(&n.left) >= node_rank(&n.right));
                check(&n.left);
                check(&n.right);
            }
        }
        check(&h.root);
    }
}
