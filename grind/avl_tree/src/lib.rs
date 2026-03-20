type Link<K, V> = Option<Box<Node<K, V>>>;

struct Node<K: Ord, V> {
    key: K, value: V, height: i32,
    left: Link<K, V>, right: Link<K, V>,
}

pub struct AvlTree<K: Ord, V> { root: Link<K, V>, len: usize }

impl<K: Ord, V> Node<K, V> {
    fn new(key: K, value: V) -> Box<Self> {
        Box::new(Node { key, value, height: 1, left: None, right: None })
    }
    fn height(node: &Link<K, V>) -> i32 {
        node.as_ref().map(|n| n.height).unwrap_or(0)
    }
    fn update_height(&mut self) {
        self.height = 1 + Self::height(&self.left).max(Self::height(&self.right));
    }
    fn balance_factor(&self) -> i32 {
        Self::height(&self.left) - Self::height(&self.right)
    }
}

fn rotate_right<K: Ord, V>(mut node: Box<Node<K, V>>) -> Box<Node<K, V>> {
    let mut left = node.left.take().unwrap();
    node.left = left.right.take();
    node.update_height();
    left.right = Some(node);
    left.update_height();
    left
}

fn rotate_left<K: Ord, V>(mut node: Box<Node<K, V>>) -> Box<Node<K, V>> {
    let mut right = node.right.take().unwrap();
    node.right = right.left.take();
    node.update_height();
    right.left = Some(node);
    right.update_height();
    right
}

fn balance<K: Ord, V>(mut node: Box<Node<K, V>>) -> Box<Node<K, V>> {
    node.update_height();
    let bf = node.balance_factor();
    if bf > 1 {
        if node.left.as_ref().unwrap().balance_factor() < 0 {
            node.left = Some(rotate_left(node.left.take().unwrap()));
        }
        return rotate_right(node);
    }
    if bf < -1 {
        if node.right.as_ref().unwrap().balance_factor() > 0 {
            node.right = Some(rotate_right(node.right.take().unwrap()));
        }
        return rotate_left(node);
    }
    node
}

impl<K: Ord, V> AvlTree<K, V> {
    pub fn new() -> Self { AvlTree { root: None, len: 0 } }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn insert(&mut self, key: K, value: V) {
        let (root, inserted) = Self::insert_node(self.root.take(), key, value);
        self.root = Some(root);
        if inserted { self.len += 1; }
    }

    fn insert_node(node: Link<K, V>, key: K, value: V) -> (Box<Node<K, V>>, bool) {
        match node {
            None => (Node::new(key, value), true),
            Some(mut n) => {
                let inserted = match key.cmp(&n.key) {
                    std::cmp::Ordering::Equal => { n.value = value; false }
                    std::cmp::Ordering::Less => {
                        let (left, ins) = Self::insert_node(n.left.take(), key, value);
                        n.left = Some(left); ins
                    }
                    std::cmp::Ordering::Greater => {
                        let (right, ins) = Self::insert_node(n.right.take(), key, value);
                        n.right = Some(right); ins
                    }
                };
                (balance(n), inserted)
            }
        }
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

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (root, val) = Self::remove_node(self.root.take(), key);
        self.root = root;
        if val.is_some() { self.len -= 1; }
        val
    }

    fn remove_node(node: Link<K, V>, key: &K) -> (Link<K, V>, Option<V>) {
        match node {
            None => (None, None),
            Some(mut n) => match key.cmp(&n.key) {
                std::cmp::Ordering::Less => {
                    let (left, val) = Self::remove_node(n.left.take(), key);
                    n.left = left; (Some(balance(n)), val)
                }
                std::cmp::Ordering::Greater => {
                    let (right, val) = Self::remove_node(n.right.take(), key);
                    n.right = right; (Some(balance(n)), val)
                }
                std::cmp::Ordering::Equal => {
                    let Node { value, left, right, .. } = *n;
                    match (left, right) {
                        (None, None) => (None, Some(value)),
                        (Some(l), None) => (Some(balance(l)), Some(value)),
                        (None, Some(r)) => (Some(balance(r)), Some(value)),
                        (Some(l), Some(r)) => {
                            let (new_right, min) = Self::remove_min(r);
                            let mut min = min.unwrap();
                            min.left = Some(l);
                            min.right = new_right;
                            (Some(balance(min)), Some(value))
                        }
                    }
                }
            }
        }
    }

    fn remove_min(node: Box<Node<K, V>>) -> (Link<K, V>, Option<Box<Node<K, V>>>) {
        let Node { key, value, left, right, .. } = *node;
        match left {
            None => (right, Some(Node::new(key, value))),
            Some(l) => {
                let (new_left, min) = Self::remove_min(l);
                let mut n = Node::new(key, value);
                n.left = new_left;
                n.right = right;
                (Some(balance(n)), min)
            }
        }
    }
}

impl<K: Ord, V> Default for AvlTree<K, V> { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_insert_get() {
        let mut t = AvlTree::new();
        for i in 0..10i32 { t.insert(i, i*10); }
        for i in 0..10 { assert_eq!(t.get(&i), Some(&(i*10))); }
        assert_eq!(t.get(&99), None);
    }
    #[test] fn test_remove() {
        let mut t = AvlTree::new();
        t.insert(1, 'a'); t.insert(2, 'b'); t.insert(3, 'c');
        assert_eq!(t.remove(&2), Some('b'));
        assert_eq!(t.get(&2), None);
        assert_eq!(t.len(), 2);
    }
    #[test] fn test_empty() {
        let mut t: AvlTree<i32,i32> = AvlTree::new();
        assert!(t.is_empty()); assert_eq!(t.get(&0), None); assert_eq!(t.remove(&0), None);
    }
    #[test] fn test_update() {
        let mut t = AvlTree::new();
        t.insert(1, 10); t.insert(1, 99);
        assert_eq!(t.get(&1), Some(&99)); assert_eq!(t.len(), 1);
    }
    #[test] fn test_stress() {
        let mut t = AvlTree::new();
        for i in (0..100i32).rev() { t.insert(i, i); }
        assert_eq!(t.len(), 100);
        for i in 0..100 { assert_eq!(t.get(&i), Some(&i)); }
        for i in 0..50 { t.remove(&i); }
        assert_eq!(t.len(), 50);
    }
}