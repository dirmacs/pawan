//! Segment Tree — O(log n) point update and range query.
//!
//! Supports any associative combine operation (sum, min, max, gcd, etc.).
//! Uses a flat array of size 4*n for zero-indexed input arrays.

pub struct SegTree<T, F> {
    n: usize,
    tree: Vec<T>,
    identity: T,
    combine: F,
}

impl<T: Copy + Clone, F: Fn(T, T) -> T> SegTree<T, F> {
    /// Build a segment tree from a slice.
    pub fn new(data: &[T], identity: T, combine: F) -> Self {
        let n = data.len();
        let mut tree = vec![identity; 4 * n.max(1)];
        if n > 0 {
            Self::build(&mut tree, data, 1, 0, n - 1, &combine);
        }
        SegTree { n, tree, identity, combine }
    }

    fn build(tree: &mut Vec<T>, data: &[T], node: usize, l: usize, r: usize, combine: &F) {
        if l == r {
            tree[node] = data[l];
            return;
        }
        let mid = (l + r) / 2;
        Self::build(tree, data, 2 * node, l, mid, combine);
        Self::build(tree, data, 2 * node + 1, mid + 1, r, combine);
        tree[node] = combine(tree[2 * node], tree[2 * node + 1]);
    }

    /// Point update: set index `i` to `val` (0-based).
    pub fn update(&mut self, i: usize, val: T) {
        if self.n == 0 { return; }
        Self::update_rec(&mut self.tree, i, val, 1, 0, self.n - 1, &self.combine);
    }

    fn update_rec(tree: &mut Vec<T>, i: usize, val: T, node: usize, l: usize, r: usize, combine: &F) {
        if l == r {
            tree[node] = val;
            return;
        }
        let mid = (l + r) / 2;
        if i <= mid {
            Self::update_rec(tree, i, val, 2 * node, l, mid, combine);
        } else {
            Self::update_rec(tree, i, val, 2 * node + 1, mid + 1, r, combine);
        }
        tree[node] = combine(tree[2 * node], tree[2 * node + 1]);
    }

    /// Range query: combine elements in [ql, qr] (0-based, inclusive).
    pub fn query(&self, ql: usize, qr: usize) -> T {
        if self.n == 0 || ql > qr { return self.identity; }
        let qr = qr.min(self.n - 1);
        self.query_rec(1, 0, self.n - 1, ql, qr)
    }

    fn query_rec(&self, node: usize, l: usize, r: usize, ql: usize, qr: usize) -> T {
        if ql > r || qr < l { return self.identity; }
        if ql <= l && r <= qr { return self.tree[node]; }
        let mid = (l + r) / 2;
        let left = self.query_rec(2 * node, l, mid, ql, qr);
        let right = self.query_rec(2 * node + 1, mid + 1, r, ql, qr);
        (self.combine)(left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sum_basic() {
        let data = vec![1i64, 3, 5, 7, 9, 11];
        let st = SegTree::new(&data, 0, |a, b| a + b);
        assert_eq!(st.query(0, 5), 36);
        assert_eq!(st.query(0, 0), 1);
        assert_eq!(st.query(2, 4), 21);
        assert_eq!(st.query(1, 3), 15);
    }

    #[test]
    fn test_sum_update() {
        let data = vec![1i64, 3, 5, 7, 9, 11];
        let mut st = SegTree::new(&data, 0, |a, b| a + b);
        st.update(3, 0); // 7 -> 0
        assert_eq!(st.query(0, 5), 29);
        assert_eq!(st.query(3, 3), 0);
        assert_eq!(st.query(2, 4), 14);
    }

    #[test]
    fn test_min_range() {
        let data = vec![4i32, 2, 7, 1, 8, 3];
        let st = SegTree::new(&data, i32::MAX, |a, b| a.min(b));
        assert_eq!(st.query(0, 5), 1);
        assert_eq!(st.query(0, 2), 2);
        assert_eq!(st.query(3, 5), 1);
        assert_eq!(st.query(4, 5), 3);
    }

    #[test]
    fn test_max_range() {
        let data = vec![4i32, 2, 7, 1, 8, 3];
        let st = SegTree::new(&data, i32::MIN, |a, b| a.max(b));
        assert_eq!(st.query(0, 5), 8);
        assert_eq!(st.query(0, 2), 7);
        assert_eq!(st.query(3, 5), 8);
        assert_eq!(st.query(0, 1), 4);
    }

    #[test]
    fn test_single_element() {
        let data = vec![42i64];
        let mut st = SegTree::new(&data, 0, |a, b| a + b);
        assert_eq!(st.query(0, 0), 42);
        st.update(0, 10);
        assert_eq!(st.query(0, 0), 10);
    }

    #[test]
    fn test_empty() {
        let data: Vec<i64> = vec![];
        let st = SegTree::new(&data, 0, |a, b| a + b);
        assert_eq!(st.query(0, 0), 0);
    }

    #[test]
    fn test_full_rebuild_consistency() {
        let n = 100usize;
        let data: Vec<i64> = (1..=n as i64).collect();
        let mut st = SegTree::new(&data, 0, |a, b| a + b);
        let expected_total: i64 = n as i64 * (n as i64 + 1) / 2;
        assert_eq!(st.query(0, n - 1), expected_total);

        // Zero out even indices (0,2,4,...,98)
        for i in (0..n).step_by(2) {
            st.update(i, 0);
        }
        // Odd-indexed positions (1,3,5,...,99) have values (2,4,6,...,100)
        let expected_odd: i64 = (2..=n as i64).step_by(2).sum();
        assert_eq!(st.query(0, n - 1), expected_odd);
    }

    #[test]
    fn test_min_update() {
        let data = vec![5i32, 3, 8, 2, 9];
        let mut st = SegTree::new(&data, i32::MAX, |a, b| a.min(b));
        assert_eq!(st.query(0, 4), 2);
        st.update(3, 100); // was min, now large
        assert_eq!(st.query(0, 4), 3);
        st.update(1, 1); // new min
        assert_eq!(st.query(0, 4), 1);
    }
}
