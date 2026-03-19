//! Disjoint Set Union (Union-Find) with path compression and union by rank.
//! O(α(n)) amortised per operation where α is the inverse Ackermann function.

pub struct Dsu {
    parent: Vec<usize>,
    rank: Vec<usize>,
    components: usize,
}

impl Dsu {
    /// Create a DSU with `n` elements (0..n), each in its own set.
    pub fn new(n: usize) -> Self {
        Dsu {
            parent: (0..n).collect(),
            rank: vec![0; n],
            components: n,
        }
    }

    /// Find the representative of the set containing `x` (with path compression).
    pub fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]); // path compression
        }
        self.parent[x]
    }

    /// Union the sets containing `x` and `y`. Returns true if they were in different sets.
    pub fn union(&mut self, x: usize, y: usize) -> bool {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry { return false; }
        // Union by rank
        match self.rank[rx].cmp(&self.rank[ry]) {
            std::cmp::Ordering::Less => self.parent[rx] = ry,
            std::cmp::Ordering::Greater => self.parent[ry] = rx,
            std::cmp::Ordering::Equal => {
                self.parent[ry] = rx;
                self.rank[rx] += 1;
            }
        }
        self.components -= 1;
        true
    }

    /// Check if `x` and `y` are in the same set.
    pub fn connected(&mut self, x: usize, y: usize) -> bool {
        self.find(x) == self.find(y)
    }

    /// Number of disjoint components.
    pub fn components(&self) -> usize {
        self.components
    }

    /// Total number of elements.
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let mut dsu = Dsu::new(5);
        assert_eq!(dsu.components(), 5);
        for i in 0..5 {
            assert_eq!(dsu.find(i), i);
        }
    }

    #[test]
    fn test_union_basic() {
        let mut dsu = Dsu::new(5);
        assert!(dsu.union(0, 1));
        assert!(dsu.connected(0, 1));
        assert!(!dsu.connected(0, 2));
        assert_eq!(dsu.components(), 4);
    }

    #[test]
    fn test_union_chain() {
        let mut dsu = Dsu::new(6);
        dsu.union(0, 1);
        dsu.union(1, 2);
        dsu.union(2, 3);
        assert!(dsu.connected(0, 3));
        assert!(!dsu.connected(0, 4));
        assert_eq!(dsu.components(), 3);
    }

    #[test]
    fn test_union_same_set() {
        let mut dsu = Dsu::new(4);
        assert!(dsu.union(0, 1));
        assert!(!dsu.union(0, 1)); // already same set
        assert!(!dsu.union(1, 0)); // same
        assert_eq!(dsu.components(), 3);
    }

    #[test]
    fn test_full_merge() {
        let n = 10;
        let mut dsu = Dsu::new(n);
        for i in 1..n {
            dsu.union(0, i);
        }
        assert_eq!(dsu.components(), 1);
        for i in 0..n {
            assert!(dsu.connected(0, i));
        }
    }

    #[test]
    fn test_two_groups() {
        let mut dsu = Dsu::new(6);
        dsu.union(0, 1);
        dsu.union(0, 2);
        dsu.union(3, 4);
        dsu.union(3, 5);
        assert_eq!(dsu.components(), 2);
        assert!(dsu.connected(0, 2));
        assert!(dsu.connected(3, 5));
        assert!(!dsu.connected(0, 3));
    }

    #[test]
    fn test_path_compression_correctness() {
        // After many finds, all nodes should still report correct component
        let mut dsu = Dsu::new(8);
        for i in 1..8 {
            dsu.union(0, i);
        }
        // Multiple finds to trigger path compression
        for _ in 0..3 {
            for i in 0..8 {
                assert_eq!(dsu.find(i), dsu.find(0));
            }
        }
    }

    #[test]
    fn test_empty() {
        let dsu = Dsu::new(0);
        assert_eq!(dsu.components(), 0);
        assert!(dsu.is_empty());
    }

    #[test]
    fn test_single() {
        let mut dsu = Dsu::new(1);
        assert_eq!(dsu.find(0), 0);
        assert_eq!(dsu.components(), 1);
        assert!(dsu.connected(0, 0));
    }

    #[test]
    fn test_stress() {
        let n = 1000;
        let mut dsu = Dsu::new(n);
        // Union consecutive pairs
        for i in (0..n - 1).step_by(2) {
            dsu.union(i, i + 1);
        }
        assert_eq!(dsu.components(), n / 2);
        // Union all pairs into one group
        for i in (0..n - 2).step_by(2) {
            dsu.union(i, i + 2);
        }
        assert_eq!(dsu.components(), 1);
        assert!(dsu.connected(0, n - 2));
    }
}
