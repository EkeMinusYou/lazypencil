//! Keep retained entries and their eviction order under one owner.
use super::{Cached, DynamicSettings};
use std::collections::{BTreeMap, VecDeque};

#[derive(Default)]
pub(super) struct Cache {
    pub(super) entries: BTreeMap<String, Cached>,
    pub(super) order: VecDeque<String>,
}

impl Cache {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&str, &Cached) -> bool) {
        self.entries.retain(|key, entry| keep(key, entry));
        self.order.retain(|key| self.entries.contains_key(key));
    }

    pub(super) fn insert(&mut self, key: String, entry: Cached, settings: DynamicSettings) {
        let capacity = settings.cache_mib as usize * 1024 * 1024;
        if settings.cache_entries == 0 || capacity == 0 || entry.bytes() > capacity {
            return;
        }
        self.entries.remove(&key);
        self.order.retain(|existing| existing != &key);
        let mut total = self.entries.values().map(Cached::bytes).sum::<usize>();
        while self.entries.len() >= settings.cache_entries || total + entry.bytes() > capacity {
            let oldest = self.order.pop_front().expect("cache eviction order");
            if let Some(evicted) = self.entries.remove(&oldest) {
                total -= evicted.bytes();
            }
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, entry);
    }

    pub(super) fn get(&mut self, key: &str) -> Option<Cached> {
        let entry = self.entries.get(key)?.clone();
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.into());
        Some(entry)
    }
}
