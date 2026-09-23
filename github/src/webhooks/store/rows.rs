//! Copy-on-write row index: snapshots share payloads; only explicitly touched
//! rows are serialized. Index copies cost O(keys), never O(queued payload bytes).
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Index,
    sync::Arc,
};

pub struct Rows<V> {
    values: Arc<BTreeMap<String, Arc<V>>>,
    dirty: BTreeSet<String>,
}
impl<V> Default for Rows<V> {
    fn default() -> Self {
        Self {
            values: Arc::default(),
            dirty: BTreeSet::new(),
        }
    }
}
impl<V> Clone for Rows<V> {
    fn clone(&self) -> Self {
        Self {
            values: self.values.clone(),
            dirty: self.dirty.clone(),
        }
    }
}
impl<V> Rows<V> {
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
    pub fn get(&self, key: &str) -> Option<&V> {
        self.values.get(key).map(Arc::as_ref)
    }
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.values.keys()
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.values.values().map(Arc::as_ref)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.values.iter().map(|(k, v)| (k, v.as_ref()))
    }
    pub fn insert(&mut self, key: String, value: V) {
        self.dirty.insert(key.clone());
        Arc::make_mut(&mut self.values).insert(key, Arc::new(value));
    }
    pub fn remove(&mut self, key: &str) {
        if self.values.contains_key(key) {
            self.dirty.insert(key.into());
            Arc::make_mut(&mut self.values).remove(key);
        }
    }
    pub fn clear(&mut self) {
        self.dirty.extend(self.values.keys().cloned());
        self.values = Arc::default();
    }
    pub fn retain(&mut self, mut f: impl FnMut(&str, &V) -> bool) {
        Arc::make_mut(&mut self.values).retain(|k, v| {
            if f(k, v) {
                true
            } else {
                self.dirty.insert(k.clone());
                false
            }
        });
    }
    pub fn entry(&mut self, key: String) -> Entry<'_, V> {
        Entry { rows: self, key }
    }
    pub(super) fn dirty(&self) -> impl Iterator<Item = &String> {
        self.dirty.iter()
    }
    pub(super) fn clean(&mut self) {
        self.dirty.clear();
    }
}
impl<V: Clone> Rows<V> {
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        if !self.values.contains_key(key) {
            return None;
        }
        self.dirty.insert(key.into());
        Arc::make_mut(&mut self.values)
            .get_mut(key)
            .map(Arc::make_mut)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.dirty.extend(self.values.keys().cloned());
        Arc::make_mut(&mut self.values)
            .values_mut()
            .map(Arc::make_mut)
    }
}
pub struct Entry<'a, V> {
    rows: &'a mut Rows<V>,
    key: String,
}
impl<'a, V: Clone> Entry<'a, V> {
    pub fn or_insert_with(self, f: impl FnOnce() -> V) -> &'a mut V {
        self.rows.dirty.insert(self.key.clone());
        Arc::make_mut(
            Arc::make_mut(&mut self.rows.values)
                .entry(self.key)
                .or_insert_with(|| Arc::new(f())),
        )
    }
}
impl<V> Index<&str> for Rows<V> {
    type Output = V;
    fn index(&self, key: &str) -> &V {
        &self.values[key]
    }
}
impl<'a, V> IntoIterator for &'a Rows<V> {
    type Item = (&'a String, &'a V);
    type IntoIter = std::iter::Map<
        std::collections::btree_map::Iter<'a, String, Arc<V>>,
        fn((&'a String, &'a Arc<V>)) -> (&'a String, &'a V),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter().map(|(k, v)| (k, v.as_ref()))
    }
}
impl<'a, V: Clone> IntoIterator for &'a mut Rows<V> {
    type Item = (&'a String, &'a mut V);
    type IntoIter = std::iter::Map<
        std::collections::btree_map::IterMut<'a, String, Arc<V>>,
        fn((&'a String, &'a mut Arc<V>)) -> (&'a String, &'a mut V),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.dirty.extend(self.values.keys().cloned());
        Arc::make_mut(&mut self.values)
            .iter_mut()
            .map(|(k, v)| (k, Arc::make_mut(v)))
    }
}
impl<V: Serialize> Serialize for Rows<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}
impl<'de, V: Deserialize<'de>> Deserialize<'de> for Rows<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = BTreeMap::<String, V>::deserialize(deserializer)?;
        Ok(Self {
            values: Arc::new(values.into_iter().map(|(k, v)| (k, Arc::new(v))).collect()),
            dirty: BTreeSet::new(),
        })
    }
}
#[derive(Default, Clone)]
pub struct Keys(Rows<()>);
impl Keys {
    pub fn insert(&mut self, key: String) -> bool {
        if self.contains(&key) {
            return false;
        }
        self.0.insert(key, ());
        true
    }
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }
    pub(super) fn rows(&self) -> &Rows<()> {
        &self.0
    }
    pub(super) fn clean(&mut self) {
        self.0.clean();
    }
}
impl Serialize for Keys {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}
impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let keys = BTreeSet::<String>::deserialize(deserializer)?;
        let mut out = Self::default();
        for key in keys {
            out.insert(key);
        }
        out.clean();
        Ok(out)
    }
}
