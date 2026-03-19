/// A min-leftist heap implementation.
///
/// Maintains:
/// - BST min-heap order (parent <= children)
/// - Leftist property (rank of left child >= rank of right child)
///
/// Provides O(log n) merge and O(log n) push/pop operations.
pub struct LeftistHeap<T: Ord> {
    root: Link<T>,
    len: usize,
}

/// A node in the leftist heap.
#[derive(Debug)]
pub struct Node<T: Ord> {
    val: T,
    rank: usize,
    left: Link<T>,
    right: Link<T>,
}

/// A linked reference to a node (Option<Box<Node<T>>>).
pub type Link<T> = Option<Box<Node<T>>>;

/// Returns the rank of a node (0 if None).
fn node_rank<T>(n: &Link<T>) -> usize {
    n.as_ref().map(|n| n.rank).unwrap_or(0)
}

/// Merges two leftist heaps into one.
/// 
/// Time complexity: O(log n) amortized.
fn merge<T: Ord>(a: Link<T>, b: Link<T>) -> Link<T> {
    // If one is None, return the other
    match (a, b) {
        (None, b) => b,
        (a, None) => a,
        (Some(a), Some(b)) => {
            // Compare values to maintain min-heap property
            if a.val < b.val {
                // a is smaller, merge b with a's right child
                let new_root = Some(a);
                let new_right = merge(b, a.left.as_ref().map(|n| n.right));
                let new_left = a.left;
                // Enforce leftist property: left rank >= right rank
                match (&new_left, &new_right) {
                    (None, Some(_)) => {
                        // Swap to maintain leftist property
                        Some(Box::new(Node {
                            val: new_root.as_ref().unwrap().val,
                            rank: new_right.as_ref().unwrap().rank + 1,
                            left: new_right,
                            right: new_left,
                        }))
                    }
                    _ => {
                        Some(Box::new(Node {
                            val: new_root.as_ref().unwrap().val,
                            rank: node_rank(&new_left) + 1,
                            left: new_left,
                            right: new_right,
                        }))
                    }
                }
            } else {
                // b is smaller, merge a with b's right child
                let new_root = Some(b);
                let new_right = merge(a, b.left.as_ref().map(|n| n.right));
                let new_left = b.left;
                // Enforce leftist property: left rank >= right rank
                match (&new_left, &new_right) {
                    (None, Some(_)) => {
                        // Swap to maintain leftist property
                        Some(Box::new(Node {
                            val: new_root.as_ref().unwrap().val,
                            rank: new_right.as_ref().unwrap().rank + 1,
                            left: new_right,
                            right: new_left,
                        }))
                    }
                    _ => {
                        Some(Box::new(Node {
                            val: new_root.as_ref().unwrap().val,
                            rank: node_rank(&new_left) + 1,
                            left: new_left,
                            right: new_right,
                        }))
                    }
                }
            }
        }
    }
}

impl<T: Ord> LeftistHeap<T> {
    /// Creates a new empty leftist heap.
    pub fn new() -> Self {
        LeftistHeap { root: None, len: 0 }
    }

    /// Pushes a value onto the heap.
    /// 
    /// # Time Complexity
    /// O(log n) amortized.
    pub fn push(&mut self, val: T) {
        self.root = merge(self.root, Some(Box::new(Node {
            val,
            rank: 1,
            left: None,
            right: None,
        })));
        self.len += 1;
    }

    /// Removes and returns the minimum value from the heap.
    /// 
    /// Returns `None` if the heap is empty.
    /// 
    /// # Time Complexity
    /// O(log n) amortized.
    pub fn pop(&mut self) -> Option<T> {
        match self.root {
            None => None,
            Some(root) => {
                self.len -= 1;
                let min_val = root.val;
                self.root = merge(root.left, root.right);
                Some(min_val)
            }
        }
    }

    /// Returns a reference to the minimum value without removing it.
    /// 
    /// Returns `None` if the heap is empty.
    pub fn peek(&self) -> Option<&T> {
        self.root.as_ref().map(|n| &n.val)
    }

    /// Returns the number of elements in the heap.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the heap is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that push/pop maintains ascending order.
    /// Push 3,1,4,1,5,9,2,6 — successive pops give 1,1,2,3,4,5,6,9.
    #[test]
    fn test_push_pop_order() {
        let mut heap = LeftistHeap::new();
        let values = vec![3, 1, 4, 1, 5, 9, 2, 6];
        for val in values {
            heap.push(val);
        }
        
        let mut expected = vec![1, 1, 2, 3, 4, 5, 6, 9];
        while !heap.is_empty() {
            assert_eq!(heap.pop(), expected.remove(0));
        }
        assert!(heap.is_empty());
    }

    /// Test behavior on empty heap.
    #[test]
    fn test_empty() {
        let mut heap = LeftistHeap::new();
        assert_eq!(heap.pop(), None);
        assert_eq!(heap.peek(), None);
        assert!(heap.is_empty());
    }

    /// Test single element heap.
    #[test]
    fn test_single() {
        let mut heap = LeftistHeap::new();
        heap.push(42);
        assert_eq!(heap.peek(), Some(&42));
        assert_eq!(heap.len(), 1);
        assert_eq!(heap.pop(), Some(42));
        assert!(heap.is_empty());
    }

    /// Test length tracking.
    #[test]
    fn test_len_tracking() {
        let mut heap = LeftistHeap::new();
        for _ in 0..5 {
            heap.push(5);
        }
        assert_eq!(heap.len(), 5);
        
        for _ in 0..2 {
            heap.pop();
        }
        assert_eq!(heap.len(), 3);
    }

    /// Test with reverse order insertion (descending).
    /// Push 10,9,8,...,0 — pops should give 0,1,2,...,10 ascending.
    #[test]
    fn test_reverse_order() {
        let mut heap = LeftistHeap::new();
        for i in (0..=10).rev() {
            heap.push(i);
        }
        
        let mut expected = 0;
        while !heap.is_empty() {
            let val = heap.pop().unwrap();
            assert_eq!(val, expected);
            expected += 1;
        }
    }

    /// Test with all same values.
    #[test]
    fn test_all_same() {
        let mut heap = LeftistHeap::new();
        for _ in 0..5 {
            heap.push(5);
        }
        
        for _ in 0..5 {
            assert_eq!(heap.pop(), Some(5));
        }
        assert!(heap.is_empty());
    }

    /// Stress test: push 0..100i32, verify pops are in non-decreasing order.
    #[test]
    fn test_stress() {
        let mut heap = LeftistHeap::new();
        for i in 0..=100 {
            heap.push(i);
        }
        
        let mut prev = i32::MIN;
        while !heap.is_empty() {
            let val = heap.pop().unwrap();
            assert!(val >= prev, "Expected value >= {}, got {}", prev, val);
            prev = val;
        }
    }

    /// Test min-heap property: peek returns the minimum value.
    #[test]
    fn test_min_property() {
        let mut heap = LeftistHeap::new();
        for val in [7, 2, 9, 1, 5] {
            heap.push(val);
        }
        assert_eq!(heap.peek(), Some(&1));
    }
}
