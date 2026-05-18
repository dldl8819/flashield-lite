use std::collections::{HashMap, VecDeque};

pub trait CacheValue: Clone {
    fn size(&self) -> u64;
}

#[derive(Debug, Clone)]
pub struct LruCache<V: CacheValue> {
    capacity_bytes: u64,
    used_bytes: u64,
    entries: HashMap<String, V>,
    order: VecDeque<String>,
}

impl<V: CacheValue> LruCache<V> {
    pub fn new(capacity_bytes: u64) -> Self {
        Self {
            capacity_bytes,
            used_bytes: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn get(&mut self, key: &str) -> Option<V> {
        let value = self.entries.get(key).cloned();
        if value.is_some() {
            self.touch(key);
        }
        value
    }

    pub fn insert(&mut self, key: String, value: V) -> Vec<(String, V)> {
        if let Some(old) = self.entries.remove(&key) {
            self.used_bytes = self.used_bytes.saturating_sub(old.size());
            self.remove_from_order(&key);
        }

        if value.size() > self.capacity_bytes {
            return Vec::new();
        }

        self.used_bytes += value.size();
        self.entries.insert(key.clone(), value);
        self.order.push_back(key);
        self.evict_until_within_capacity()
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let removed = self.entries.remove(key);
        if let Some(value) = &removed {
            self.used_bytes = self.used_bytes.saturating_sub(value.size());
            self.remove_from_order(key);
        }
        removed
    }

    fn touch(&mut self, key: &str) {
        self.remove_from_order(key);
        self.order.push_back(key.to_string());
    }

    fn remove_from_order(&mut self, key: &str) {
        self.order.retain(|candidate| candidate != key);
    }

    fn evict_until_within_capacity(&mut self) -> Vec<(String, V)> {
        let mut evicted = Vec::new();
        while self.used_bytes > self.capacity_bytes {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if let Some(value) = self.entries.remove(&key) {
                self.used_bytes = self.used_bytes.saturating_sub(value.size());
                evicted.push((key, value));
            }
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::{CacheValue, LruCache};

    #[derive(Clone, Debug, PartialEq)]
    struct TestValue(u64);

    impl CacheValue for TestValue {
        fn size(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn evicts_least_recently_used_entry() {
        let mut cache = LruCache::new(4);
        assert!(cache.insert("a".to_string(), TestValue(2)).is_empty());
        assert!(cache.insert("b".to_string(), TestValue(2)).is_empty());
        assert_eq!(cache.get("a"), Some(TestValue(2)));

        let evicted = cache.insert("c".to_string(), TestValue(2));

        assert_eq!(evicted, vec![("b".to_string(), TestValue(2))]);
        assert_eq!(cache.get("a"), Some(TestValue(2)));
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("c"), Some(TestValue(2)));
    }
}
