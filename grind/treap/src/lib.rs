//! Treap — randomized BST with heap priority.
//! O(log n) expected insert, delete, search.
//!
//! Each node carries a random priority; BST on keys, max-heap on priorities.
//! Rotation-based implementation for correct ownership semantics.

use rand::{thread_rng, Rng};

type Link<K, V> = Option<Box<Node<K, V>>>;

struct Node<K, V> {
    key: K,
    value: V,
    priority: u32,
    left: Link<K, V>,
    right: Link<K, V>,
}

impl<K, V> Node<K, V> {
    fn new(key: K, value: V) -> Box<Self> {
        Box::new(Node {
            key,
            value,
            priority: thread_rng().r#gen(),
            left: None,
            right: None,
        })
    }
}

/// Merge two treaps where all keys in `left` < all keys in `right`.
fn merge<K: Ord, V>(left: Link<K, V>, right: Link<K, V>) -> Link<K, V> {
    match (left, right) {
        (None, r) => r,
        (l, None) => l,
        (Some(mut l), Some(mut r)) => {
            if l.priority >= r.priority {
                l.right = merge(l.right.take(), Some(r));
                Some(l)
            } else {
                r.left = merge(Some(l), r.left.take());
                Some(r)
            }
        }
    }
}

pub struct Treap<K: Ord, V> {
    root: Link<K, V>,
    len: usize,
}

impl<K: Ord, V> Treap<K, V> {
    pub fn new() -> Self {
        Treap { root: None, len: 0 }
    }

    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    fn insert_node(node: Link<K, V>, new_node: Box<Node<K, V>>) -> (Link<K, V>, bool) {
        match node {
            None => (Some(new_node), false),
            Some(mut n) => {
                match new_node.key.cmp(&n.key) {
                    std::cmp::Ordering::Equal => {
                        n.value = new_node.value;
                        (Some(n), true) // replaced, not new
                    }
                    std::cmp::Ordering::Less => {
                        let (new_left, replaced) = Self::insert_node(n.left.take(), new_node);
                        n.left = new_left;
                        // Restore heap property via right rotation if needed
                        if n.left.as_ref().map_or(false, |l| l.priority > n.priority) {
                            let mut left = n.left.take().unwrap();
                            n.left = left.right.take();
                            left.right = Some(n);
                            (Some(left), replaced)
                        } else {
                            (Some(n), replaced)
                        }
                    }
                    std::cmp::Ordering::Greater => {
                        let (new_right, replaced) = Self::insert_node(n.right.take(), new_node);
                        n.right = new_right;
                        // Restore heap property via left rotation if needed
                        if n.right.as_ref().map_or(false, |r| r.priority > n.priority) {
                            let mut right = n.right.take().unwrap();
                            n.right = right.left.take();
                            right.left = Some(n);
                            (Some(right), replaced)
                        } else {
                            (Some(n), replaced)
                        }
                    }
                }
            }
        }
    }

    pub fn insert(&mut self, key: K, value: V) {
        let root = self.root.take();
        let new_node = Node::new(key, value);
        let (root, replaced) = Self::insert_node(root, new_node);
        self.root = root;
        if !replaced { self.len += 1; }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        let mut cur = self.root.as_deref();
        while let Some(node) = cur {
            match key.cmp(&node.key) {
                std::cmp::Ordering::Equal => return Some(&node.value),
                std::cmp::Ordering::Less => cur = node.left.as_deref(),
                std::cmp::Ordering::Greater => cur = node.right.as_deref(),
            }
        }
        None
    }

    pub fn contains(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    fn remove_node(node: Link<K, V>, key: &K) -> (Link<K, V>, Option<V>) {
        match node {
            None => (None, None),
            Some(mut n) => {
                match key.cmp(&n.key) {
                    std::cmp::Ordering::Equal => {
                        let merged = merge(n.left.take(), n.right.take());
                        let Node { value, .. } = *n; // move value out of box
                        (merged, Some(value))
                    }
                    std::cmp::Ordering::Less => {
                        let (new_left, val) = Self::remove_node(n.left.take(), key);
                        n.left = new_left;
                        (Some(n), val)
                    }
                    std::cmp::Ordering::Greater => {
                        let (new_right, val) = Self::remove_node(n.right.take(), key);
                        n.right = new_right;
                        (Some(n), val)
                    }
                }
            }
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let root = self.root.take();
        let (root, val) = Self::remove_node(root, key);
        self.root = root;
        if val.is_some() { self.len -= 1; }
        val
    }

    /// In-order traversal collecting (key, value) pairs.
    pub fn to_sorted_vec(&self) -> Vec<(&K, &V)> {
        let mut out = Vec::with_capacity(self.len);
        Self::inorder(self.root.as_deref(), &mut out);
        out
    }

    fn inorder<'a>(node: Option<&'a Node<K, V>>, out: &mut Vec<(&'a K, &'a V)>) {
        if let Some(n) = node {
            Self::inorder(n.left.as_deref(), out);
            out.push((&n.key, &n.value));
            Self::inorder(n.right.as_deref(), out);
        }
    }
}

impl<K: Ord, V> Default for Treap<K, V> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_get() {
        let mut t = Treap::new();
        t.insert("a", 1);
        t.insert("b", 2);
        t.insert("c", 3);
        assert_eq!(t.get(&"a"), Some(&1));
        assert_eq!(t.get(&"b"), Some(&2));
        assert_eq!(t.get(&"c"), Some(&3));
        assert_eq!(t.get(&"z"), None);
    }

    #[test]
    fn test_update_existing() {
        let mut t = Treap::new();
        t.insert("x", 10);
        t.insert("x", 99);
        assert_eq!(t.get(&"x"), Some(&99));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn test_remove() {
        let mut t = Treap::new();
        t.insert(1, "one");
        t.insert(2, "two");
        t.insert(3, "three");
        assert_eq!(t.remove(&2), Some("two"));
        assert_eq!(t.get(&2), None);
        assert_eq!(t.len(), 2);
        assert_eq!(t.remove(&99), None);
    }

    #[test]
    fn test_sorted_order() {
        let mut t = Treap::new();
        let keys = [5, 3, 8, 1, 4, 7, 9, 2, 6];
        for &k in &keys {
            t.insert(k, k * 10);
        }
        let sorted = t.to_sorted_vec();
        let ks: Vec<i32> = sorted.iter().map(|&(&k, _)| k).collect();
        assert_eq!(ks, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn test_contains() {
        let mut t = Treap::new();
        t.insert(42, ());
        assert!(t.contains(&42));
        assert!(!t.contains(&0));
    }

    #[test]
    fn test_empty() {
        let mut t: Treap<i32, i32> = Treap::new();
        assert!(t.is_empty());
        assert_eq!(t.get(&0), None);
        assert_eq!(t.remove(&0), None);
    }

    #[test]
    fn test_stress_sorted() {
        let mut t = Treap::new();
        for i in (0..200i32).rev() {
            t.insert(i, i);
        }
        assert_eq!(t.len(), 200);
        let sorted = t.to_sorted_vec();
        let ks: Vec<i32> = sorted.iter().map(|&(&k, _)| k).collect();
        let expected: Vec<i32> = (0..200).collect();
        assert_eq!(ks, expected);
    }

    #[test]
    fn test_remove_all() {
        let mut t = Treap::new();
        for i in 0..50i32 {
            t.insert(i, i);
        }
        for i in 0..50i32 {
            assert_eq!(t.remove(&i), Some(i));
        }
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn test_remove_and_reinsert() {
        let mut t = Treap::new();
        t.insert(10, "a");
        t.insert(20, "b");
        t.remove(&10);
        t.insert(10, "c");
        assert_eq!(t.get(&10), Some(&"c"));
        assert_eq!(t.get(&20), Some(&"b"));
        assert_eq!(t.len(), 2);
    }
}
