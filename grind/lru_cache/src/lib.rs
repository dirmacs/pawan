use std::collections::HashMap;
use std::hash::Hash;

pub struct LruCache<K, V> {
    capacity: usize,
    map: HashMap<K, V>,
    order: Vec<K>,
}

impl<K: Eq + Hash + Clone + PartialEq, V> LruCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        LruCache {
            capacity,
            map: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        match self.map.get(key) {
            Some(value) => {
                // Move accessed key to the end (most recently used)
                if let Some(pos) = self.order.iter().position(|k| *k == *key) {
                    self.order.remove(pos);
                    self.order.push(key.clone());
                }
                Some(value)
            }
            None => None,
        }
    }

    pub fn put(&mut self, key: K, value: V) {
        if self.map.contains_key(&key) {
            // Key exists, update and move to end
            self.map.insert(key.clone(), value);
            if let Some(pos) = self.order.iter().position(|k| *k == key) {
                self.order.remove(pos);
            }
            self.order.push(key);
        } else {
            // New key
            if self.capacity == 0 {
                // Don't store anything if capacity is 0
                return;
            }
            self.map.insert(key.clone(), value);
            if self.order.len() >= self.capacity {
                // Evict least recently used (first in order)
                let evict_key = self.order.remove(0);
                self.map.remove(&evict_key);
            }
            self.order.push(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cache() {
        let cache: LruCache<String, i32> = LruCache::new(2);
        assert_eq!(cache.capacity, 2);
        assert!(cache.map.is_empty());
        assert!(cache.order.is_empty());
    }

    #[test]
    fn test_put_and_get() {
        let mut cache: LruCache<String, i32> = LruCache::new(3);
        cache.put("a".to_string(), 1);
        cache.put("b".to_string(), 2);
        
        assert_eq!(cache.get(&"a".to_string()), Some(&1));
        assert_eq!(cache.get(&"b".to_string()), Some(&2));
    }

    #[test]
    fn test_eviction() {
        let mut cache: LruCache<String, i32> = LruCache::new(2);
        cache.put("a".to_string(), 1);
        cache.put("b".to_string(), 2);
        cache.put("c".to_string(), 3);
        
        // "a" should be evicted as it's least recently used
        assert_eq!(cache.get(&"a".to_string()), None);
        assert_eq!(cache.get(&"b".to_string()), Some(&2));
        assert_eq!(cache.get(&"c".to_string()), Some(&3));
    }

    #[test]
    fn test_update_moves_to_end() {
        let mut cache: LruCache<String, i32> = LruCache::new(2);
        cache.put("a".to_string(), 1);
        cache.put("b".to_string(), 2);
        // Update "a" — moves it to most recently used
        cache.put("a".to_string(), 10);
        // Insert "c" — should evict "b" (now LRU), not "a"
        cache.put("c".to_string(), 3);

        assert_eq!(cache.get(&"b".to_string()), None); // evicted
        assert_eq!(cache.get(&"a".to_string()), Some(&10)); // updated, still present
        assert_eq!(cache.get(&"c".to_string()), Some(&3)); // newest
    }

    #[test]
    fn test_capacity_zero() {
        let mut cache: LruCache<String, i32> = LruCache::new(0);
        cache.put("a".to_string(), 1);
        assert_eq!(cache.get(&"a".to_string()), None);
        cache.put("b".to_string(), 2);
        assert_eq!(cache.get(&"b".to_string()), None);
    }
}
