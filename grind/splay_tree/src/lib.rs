//! Splay Tree — self-adjusting BST. Recently accessed elements near the root.
//! O(log n) amortised insert/get/remove via splaying.

type Link<K, V> = Option<Box<Node<K, V>>>;

struct Node<K, V> {
    key: K,
    value: V,
    left: Link<K, V>,
    right: Link<K, V>,
}

pub struct SplayTree<K: Ord, V> {
    root: Link<K, V>,
    len: usize,
}

impl<K: Ord, V> SplayTree<K, V> {
    pub fn new() -> Self { SplayTree { root: None, len: 0 } }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn insert(&mut self, key: K, value: V) {
        let root = self.root.take();
        let (left, existing, right) = Self::split(root, &key);
        let replaced = existing.is_some();
        self.root = Some(Box::new(Node { key, value, left, right }));
        if !replaced { self.len += 1; }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let root = self.root.take();
        self.root = Self::splay(root, key);
        self.root.as_ref().and_then(|n| if &n.key == key { Some(&n.value) } else { None })
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let root = self.root.take();
        let root = Self::splay(root, key);
        match root {
            Some(node) if &node.key == key => {
                let Node { value, left, right, .. } = *node;
                self.root = Self::join(left, right);
                self.len -= 1;
                Some(value)
            }
            other => {
                self.root = other;
                None
            }
        }
    }

    /// Splay the node with the given key (or nearest) to the root.
    fn splay(root: Link<K, V>, key: &K) -> Link<K, V> {
        let mut node = match root {
            None => return None,
            Some(n) => n,
        };

        match key.cmp(&node.key) {
            std::cmp::Ordering::Equal => Some(node),
            std::cmp::Ordering::Less => {
                match node.left.take() {
                    None => Some(node),
                    Some(mut left) => {
                        if key < &left.key {
                            // Zig-zig: splay left-left, then rotate right twice
                            left.left = Self::splay(left.left.take(), key);
                            node.left = Some(left);
                            node = Self::rotate_right(node);
                            if node.left.is_some() {
                                node = Self::rotate_right(node);
                            }
                            Some(node)
                        } else if key > &left.key {
                            // Zig-zag: splay left-right
                            left.right = Self::splay(left.right.take(), key);
                            if left.right.is_some() {
                                let mut mid = left.right.take().unwrap();
                                left.right = mid.left.take();
                                node.left = mid.right.take();
                                mid.left = Some(left);
                                mid.right = Some(node);
                                Some(mid)
                            } else {
                                node.left = Some(left);
                                Some(Self::rotate_right(node))
                            }
                        } else {
                            // Found at left child — zig
                            node.left = left.right.take();
                            left.right = Some(node);
                            Some(left)
                        }
                    }
                }
            }
            std::cmp::Ordering::Greater => {
                match node.right.take() {
                    None => Some(node),
                    Some(mut right) => {
                        if key > &right.key {
                            // Zig-zig right
                            right.right = Self::splay(right.right.take(), key);
                            node.right = Some(right);
                            node = Self::rotate_left(node);
                            if node.right.is_some() {
                                node = Self::rotate_left(node);
                            }
                            Some(node)
                        } else if key < &right.key {
                            // Zig-zag right-left
                            right.left = Self::splay(right.left.take(), key);
                            if right.left.is_some() {
                                let mut mid = right.left.take().unwrap();
                                right.left = mid.right.take();
                                node.right = mid.left.take();
                                mid.right = Some(right);
                                mid.left = Some(node);
                                Some(mid)
                            } else {
                                node.right = Some(right);
                                Some(Self::rotate_left(node))
                            }
                        } else {
                            // Found at right child — zig
                            node.right = right.left.take();
                            right.left = Some(node);
                            Some(right)
                        }
                    }
                }
            }
        }
    }

    fn rotate_right(mut node: Box<Node<K, V>>) -> Box<Node<K, V>> {
        if let Some(mut left) = node.left.take() {
            node.left = left.right.take();
            left.right = Some(node);
            left
        } else {
            node
        }
    }

    fn rotate_left(mut node: Box<Node<K, V>>) -> Box<Node<K, V>> {
        if let Some(mut right) = node.right.take() {
            node.right = right.left.take();
            right.left = Some(node);
            right
        } else {
            node
        }
    }

    /// Split tree into (keys < key, node with key if exists, keys > key)
    fn split(root: Link<K, V>, key: &K) -> (Link<K, V>, Option<Box<Node<K, V>>>, Link<K, V>) {
        let root = Self::splay(root, key);
        match root {
            None => (None, None, None),
            Some(mut node) => {
                match key.cmp(&node.key) {
                    std::cmp::Ordering::Equal => {
                        let left = node.left.take();
                        let right = node.right.take();
                        (left, Some(node), right)
                    }
                    std::cmp::Ordering::Less => {
                        let left = node.left.take();
                        (left, None, Some(node))
                    }
                    std::cmp::Ordering::Greater => {
                        let right = node.right.take();
                        (Some(node), None, right)
                    }
                }
            }
        }
    }

    /// Join two trees where all keys in left < all keys in right.
    fn join(left: Link<K, V>, right: Link<K, V>) -> Link<K, V> {
        match left {
            None => right,
            Some(mut l) => {
                // Splay the max of left to root (it has no right child after splay)
                l = Self::splay_max(l);
                l.right = right;
                Some(l)
            }
        }
    }

    fn splay_max(mut node: Box<Node<K, V>>) -> Box<Node<K, V>> {
        if node.right.is_none() { return node; }
        let mut right = node.right.take().unwrap();
        right = Self::splay_max(right);
        node.right = right.left.take();
        right.left = Some(node);
        right
    }
}

impl<K: Ord, V> Default for SplayTree<K, V> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_get() {
        let mut t = SplayTree::new();
        t.insert("a", 1);
        t.insert("b", 2);
        t.insert("c", 3);
        assert_eq!(t.get(&"a"), Some(&1));
        assert_eq!(t.get(&"b"), Some(&2));
        assert_eq!(t.get(&"c"), Some(&3));
        assert_eq!(t.get(&"z"), None);
    }

    #[test]
    fn test_update() {
        let mut t = SplayTree::new();
        t.insert("x", 10);
        t.insert("x", 99);
        assert_eq!(t.get(&"x"), Some(&99));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn test_remove() {
        let mut t = SplayTree::new();
        t.insert(1, "one");
        t.insert(2, "two");
        t.insert(3, "three");
        assert_eq!(t.remove(&2), Some("two"));
        assert_eq!(t.get(&2), None);
        assert_eq!(t.get(&1), Some(&"one"));
        assert_eq!(t.get(&3), Some(&"three"));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn test_empty() {
        let mut t: SplayTree<i32, i32> = SplayTree::new();
        assert!(t.is_empty());
        assert_eq!(t.get(&0), None);
        assert_eq!(t.remove(&0), None);
    }

    #[test]
    fn test_len() {
        let mut t = SplayTree::new();
        for i in 0..5i32 { t.insert(i, i); }
        assert_eq!(t.len(), 5);
        t.remove(&1);
        t.remove(&3);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn test_large() {
        let mut t = SplayTree::new();
        for i in 0..100i32 { t.insert(i, i * 10); }
        for i in 0..100i32 { assert_eq!(t.get(&i), Some(&(i * 10))); }
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut t = SplayTree::new();
        t.insert(1, "a");
        assert_eq!(t.remove(&99), None);
        assert_eq!(t.len(), 1);
    }
}
