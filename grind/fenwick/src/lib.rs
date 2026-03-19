/// Fenwick Tree (Binary Indexed Tree) implementation
/// Provides efficient prefix sum and range sum queries

pub struct FenwickTree {
    tree: Vec<i64>,
}

impl FenwickTree {
    /// Create a new Fenwick Tree with size n
    pub fn new(n: usize) -> Self {
        FenwickTree {
            tree: vec![0; n + 1],
        }
    }

    /// Update the tree by adding delta to element at index i (0-based)
    pub fn update(&mut self, i: usize, delta: i64) {
        let mut j = (i + 1) as i64; // Convert to 1-based index
        let n = self.tree.len() as i64;
        while j < n {
            self.tree[j as usize] += delta;
            j += j & (-j); // Move to next node that covers this index
        }
    }

    /// Get the prefix sum from index 0 to i (inclusive, 0-based)
    pub fn prefix_sum(&self, i: usize) -> i64 {
        if self.tree.len() <= 1 {
            return 0;
        }
        let mut j = ((i + 1) as i64).min(self.tree.len() as i64 - 1);
        let mut sum = 0i64;
        while j > 0 {
            sum += self.tree[j as usize];
            j -= j & (-j); // Move to previous node that covers this index
        }
        sum
    }

    /// Get the range sum from l to r (inclusive, 0-based)
    pub fn range_sum(&self, l: usize, r: usize) -> i64 {
        if l == 0 {
            self.prefix_sum(r)
        } else {
            self.prefix_sum(r) - self.prefix_sum(l - 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let ft = FenwickTree::new(0);
        assert_eq!(ft.prefix_sum(0), 0);
    }

    #[test]
    fn test_new_basic() {
        let ft = FenwickTree::new(5);
        assert_eq!(ft.tree.len(), 6);
    }

    #[test]
    fn test_update_single() {
        let mut ft = FenwickTree::new(5);
        ft.update(0, 10);
        assert_eq!(ft.prefix_sum(0), 10);
    }

    #[test]
    fn test_prefix_sum() {
        let mut ft = FenwickTree::new(5);
        ft.update(0, 5);
        ft.update(1, 3);
        ft.update(2, 7);
        assert_eq!(ft.prefix_sum(0), 5);
        assert_eq!(ft.prefix_sum(1), 8);
        assert_eq!(ft.prefix_sum(2), 15);
    }

    #[test]
    fn test_range_sum() {
        let mut ft = FenwickTree::new(5);
        ft.update(0, 5);
        ft.update(1, 3);
        ft.update(2, 7);
        ft.update(3, 2);
        assert_eq!(ft.range_sum(0, 2), 15);
        assert_eq!(ft.range_sum(1, 3), 12);
        assert_eq!(ft.range_sum(2, 3), 9); // indices 2+3 = 7+2
    }

    #[test]
    fn test_range_sum_edge() {
        let mut ft = FenwickTree::new(3);
        ft.update(0, 1);
        ft.update(1, 2);
        ft.update(2, 3);
        assert_eq!(ft.range_sum(0, 2), 6);
        assert_eq!(ft.range_sum(0, 0), 1);
        assert_eq!(ft.range_sum(2, 2), 3);
    }

    #[test]
    fn test_range_sum_l_gt_r() {
        let mut ft = FenwickTree::new(5);
        ft.update(0, 10);
        ft.update(4, 20);
        // Range where l > r should return 0
        assert_eq!(ft.range_sum(4, 2), 0);
    }
}
