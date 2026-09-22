//! Persistent title -> magnet cache, porting of legacy's `SmartCache`
//! (`corsaro_cache.json`). Scrapers consult it before visiting a detail page,
//! so a release already seen in a previous cycle costs no HTTP request. The
//! file is written atomically and evicts the oldest half past `MAX_ENTRIES`.

use crate::utils::atomic_write;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

const MAX_ENTRIES: usize = 5000;
const EVICT_TO: usize = 2500;

#[derive(Default)]
struct Store {
    path: Option<PathBuf>,
    data: HashMap<String, String>,
    dirty: bool,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::default()));

fn lock() -> std::sync::MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Loads the on-disk cache from the Rextto data directory. Safe to call once at
/// startup; without it the cache still works in memory only.
pub fn init(data_dir: &Path) {
    let path = data_dir.join("rextto_magnet_cache.json");
    let data = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<HashMap<String, String>>(&text).ok())
        .unwrap_or_default();
    let mut store = lock();
    store.path = Some(path);
    store.data = data;
    store.dirty = false;
}

pub fn get(key: &str) -> Option<String> {
    lock().data.get(key).cloned()
}

pub fn set(key: String, value: String) {
    let mut store = lock();
    store.data.insert(key, value);
    store.dirty = true;
    if store.data.len() > MAX_ENTRIES {
        // Drop down to a bounded half; `HashMap` does not preserve insertion
        // order, so the retained subset is arbitrary but bounded (legacy keeps
        // the newest half).
        let keys: Vec<String> = store.data.keys().take(EVICT_TO).cloned().collect();
        let mut kept = HashMap::with_capacity(EVICT_TO);
        for key in keys {
            if let Some(value) = store.data.remove(&key) {
                kept.insert(key, value);
            }
        }
        store.data = kept;
    }
}

pub fn len() -> usize {
    lock().data.len()
}

/// Persists the cache if anything changed. Called once per scrape cycle.
pub fn save() {
    let payload = {
        let mut store = lock();
        if !store.dirty {
            return;
        }
        let Some(path) = store.path.clone() else {
            store.dirty = false;
            return;
        };
        let payload = match serde_json::to_vec(&store.data) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "magnet cache serialization failed");
                return;
            }
        };
        store.dirty = false;
        (path, payload)
    };
    if let Err(error) = atomic_write(&payload.0, &payload.1) {
        tracing::warn!(path = %payload.0.display(), %error, "magnet cache write failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_reads_entries_in_memory() {
        set("cache-test-title".into(), "magnet:?xt=urn:btih:abc".into());
        assert_eq!(
            get("cache-test-title").as_deref(),
            Some("magnet:?xt=urn:btih:abc")
        );
        assert!(get("cache-test-missing").is_none());
    }

    #[test]
    fn evicts_when_over_capacity() {
        for index in 0..(MAX_ENTRIES + 10) {
            set(format!("evict-{index}"), format!("magnet:{index}"));
        }
        assert!(len() <= MAX_ENTRIES);
    }
}
