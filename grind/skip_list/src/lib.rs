//! Skip List — probabilistic O(log n) sorted map.
//!
//! Arena-based implementation (usize indices into a Vec) — avoids Rc<RefCell> complexity.
//! Index 0 is the sentinel header with no key/value.

use rand::{thread_rng, Rng};

const MAX_LEVEL: usize = 16;
const PROB: f64 = 0.5;
const NIL: usize = usize::MAX;

struct Node<K, V> {
    key: Option<K>,
    value: Option<V>,
    /// next[level] = arena index of next node at that level, or NIL
    next: Vec<usize>,
}

pub struct SkipList<K: Ord, V> {
    arena: Vec<Node<K, V>>,
    level: usize, // current max active level (0-indexed)
    len: usize,
}

impl<K: Ord, V> SkipList<K, V> {
    pub fn new() -> Self {
        let header = Node {
            key: None,
            value: None,
            next: vec![NIL; MAX_LEVEL],
        };
        SkipList {
            arena: vec![header],
            level: 0,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn random_level(&self) -> usize {
        let mut lvl = 1;
        let mut rng = thread_rng();
        while lvl < MAX_LEVEL && rng.gen::<f64>() < PROB {
            lvl += 1;
        }
        lvl
    }

    /// Returns arena indices of the predecessor node at each level.
    fn find_predecessors(&self, key: &K) -> Vec<usize> {
        let mut preds = vec![0usize; MAX_LEVEL]; // 0 = header
        let mut cur = 0usize;
        for i in (0..MAX_LEVEL).rev() {
            loop {
                let nxt = self.arena[cur].next[i];
                if nxt == NIL {
                    break;
                }
                match self.arena[nxt].key.as_ref().unwrap().cmp(key) {
                    std::cmp::Ordering::Less => cur = nxt,
                    _ => break,
                }
            }
            preds[i] = cur;
        }
        preds
    }

    pub fn insert(&mut self, key: K, value: V) {
        let preds = self.find_predecessors(&key);

        // Check if key already exists (successor of pred at level 0)
        let succ = self.arena[preds[0]].next[0];
        if succ != NIL {
            if let Some(k) = &self.arena[succ].key {
                if k == &key {
                    self.arena[succ].value = Some(value);
                    return;
                }
            }
        }

        let new_level = self.random_level();
        if new_level > self.level {
            self.level = new_level;
        }

        let new_idx = self.arena.len();
        let mut new_nexts = vec![NIL; MAX_LEVEL];
        for i in 0..new_level {
            new_nexts[i] = self.arena[preds[i]].next[i];
        }

        self.arena.push(Node {
            key: Some(key),
            value: Some(value),
            next: new_nexts,
        });

        for i in 0..new_level {
            self.arena[preds[i]].next[i] = new_idx;
        }

        self.len += 1;
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        let mut cur = 0usize;
        for i in (0..MAX_LEVEL).rev() {
            loop {
                let nxt = self.arena[cur].next[i];
                if nxt == NIL {
                    break;
                }
                match self.arena[nxt].key.as_ref().unwrap().cmp(key) {
                    std::cmp::Ordering::Less => cur = nxt,
                    std::cmp::Ordering::Equal => return self.arena[nxt].value.as_ref(),
                    std::cmp::Ordering::Greater => break,
                }
            }
        }
        None
    }

    pub fn contains(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let preds = self.find_predecessors(key);

        let target = self.arena[preds[0]].next[0];
        if target == NIL {
            return None;
        }
        if self.arena[target].key.as_ref().unwrap() != key {
            return None;
        }

        for i in 0..MAX_LEVEL {
            if self.arena[preds[i]].next[i] == target {
                self.arena[preds[i]].next[i] = self.arena[target].next[i];
            }
        }

        self.len -= 1;
        self.arena[target].value.take()
    }
}

impl<K: Ord, V> Default for SkipList<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_get() {
        let mut sl = SkipList::new();
        sl.insert("a", 1);
        sl.insert("b", 2);
        sl.insert("c", 3);
        assert_eq!(sl.get(&"a"), Some(&1));
        assert_eq!(sl.get(&"b"), Some(&2));
        assert_eq!(sl.get(&"c"), Some(&3));
    }

    #[test]
    fn test_update_existing_key() {
        let mut sl = SkipList::new();
        sl.insert("a", 1);
        sl.insert("a", 100);
        assert_eq!(sl.get(&"a"), Some(&100));
        assert_eq!(sl.len(), 1);
    }

    #[test]
    fn test_remove() {
        let mut sl = SkipList::new();
        sl.insert("x", 10);
        sl.insert("y", 20);
        sl.insert("z", 30);
        assert_eq!(sl.remove(&"x"), Some(10));
        assert_eq!(sl.remove(&"y"), Some(20));
        assert_eq!(sl.remove(&"z"), Some(30));
        assert_eq!(sl.len(), 0);
        assert!(sl.is_empty());
    }

    #[test]
    fn test_contains() {
        let mut sl = SkipList::new();
        sl.insert("apple", 1);
        sl.insert("banana", 2);
        assert!(sl.contains(&"apple"));
        assert!(sl.contains(&"banana"));
        assert!(!sl.contains(&"cherry"));
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut sl: SkipList<i32, String> = SkipList::new();
        sl.insert(1, "one".to_string());
        assert_eq!(sl.remove(&999), None);
        assert_eq!(sl.len(), 1);
    }

    #[test]
    fn test_ordering_large() {
        let mut sl = SkipList::new();
        for i in (0..100i32).rev() {
            sl.insert(i, i * 2);
        }
        assert_eq!(sl.len(), 100);
        for i in 0..100i32 {
            assert_eq!(sl.get(&i), Some(&(i * 2)));
        }
    }

    #[test]
    fn test_integer_min_max() {
        let mut sl = SkipList::new();
        sl.insert(i32::MIN, "min");
        sl.insert(i32::MAX, "max");
        sl.insert(0, "zero");
        assert_eq!(sl.get(&i32::MIN), Some(&"min"));
        assert_eq!(sl.get(&i32::MAX), Some(&"max"));
        assert_eq!(sl.get(&0), Some(&"zero"));
        assert_eq!(sl.len(), 3);
    }

    #[test]
    fn test_remove_and_reinsert() {
        let mut sl = SkipList::new();
        sl.insert(1, "a");
        sl.insert(2, "b");
        sl.remove(&1);
        sl.insert(1, "a2");
        assert_eq!(sl.get(&1), Some(&"a2"));
        assert_eq!(sl.get(&2), Some(&"b"));
    }

    #[test]
    fn test_stress() {
        let mut sl = SkipList::new();
        for i in 0..500i32 {
            sl.insert(i, i);
        }
        assert_eq!(sl.len(), 500);
        for i in (0..500i32).step_by(2) {
            sl.remove(&i);
        }
        assert_eq!(sl.len(), 250);
        for i in (1..500i32).step_by(2) {
            assert_eq!(sl.get(&i), Some(&i));
        }
    }
}
