use std::collections::{BTreeMap, VecDeque};

pub const CAP: usize = 65_536;

#[derive(Clone, Debug)]
pub struct Kept<T> {
    items: VecDeque<T>,
    pub dropped: u64,
}

impl<T> Default for Kept<T> {
    fn default() -> Self {
        Kept { items: VecDeque::new(), dropped: 0 }
    }
}

impl<T> Kept<T> {
    pub fn push(&mut self, item: T) {
        if self.items.len() == CAP {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.items.push_back(item);
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, T> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<'a, T> IntoIterator for &'a Kept<T> {
    type Item = &'a T;
    type IntoIter = std::collections::vec_deque::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

#[derive(Clone, Debug)]
pub struct KeptMap<K, V> {
    map: BTreeMap<K, V>,
    pub dropped: u64,
}

impl<K, V> Default for KeptMap<K, V> {
    fn default() -> Self {
        KeptMap { map: BTreeMap::new(), dropped: 0 }
    }
}

impl<K: Ord, V> KeptMap<K, V> {
    pub fn insert(&mut self, k: K, v: V) {
        if self.map.len() >= CAP && !self.map.contains_key(&k) {
            self.dropped += 1;
            return;
        }
        self.map.insert(k, v);
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        self.map.get(k)
    }

    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, K, V> {
        self.map.iter()
    }

    pub fn keys(&self) -> std::collections::btree_map::Keys<'_, K, V> {
        self.map.keys()
    }

    pub fn values(&self) -> std::collections::btree_map::Values<'_, K, V> {
        self.map.values()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl<'a, K, V> IntoIterator for &'a KeptMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_and_maps_stay_within_the_cap() {
        let mut k = Kept::default();
        for i in 0..CAP + 10 {
            k.push(i);
        }
        assert_eq!((k.len(), k.dropped, k.iter().next().copied()), (CAP, 10, Some(10)));
        let mut m = KeptMap::default();
        for i in 0..CAP + 5 {
            m.insert(i, i);
        }
        m.insert(0, 7);
        assert_eq!((m.len(), m.dropped, m.get(&0).copied()), (CAP, 5, Some(7)));
    }
}
