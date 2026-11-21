use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::Session;

/// Session storage whose map lock is held only inside these methods.
///
/// Each map method calls nothing else while the lock is held, so the map lock
/// is a leaf lock. Hold no other lock's guard across `begin_creation().await`.
pub(super) struct Sessions {
    map: Mutex<BTreeMap<u64, Arc<Session>>>,
    next: AtomicU64,
    creating: tokio::sync::Mutex<()>,
}

impl Sessions {
    pub(super) fn new() -> Self {
        Self {
            map: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            creating: tokio::sync::Mutex::new(()),
        }
    }

    pub(super) async fn begin_creation(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.creating.lock().await
    }

    pub(super) fn next_number(&self) -> Option<u64> {
        self.next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok()
    }

    pub(super) fn insert(&self, number: u64, session: Arc<Session>) {
        let replaced = self
            .map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(number, session);
        drop(replaced);
    }

    pub(super) fn remove(&self, number: u64) {
        let removed = self
            .map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&number);
        drop(removed);
    }

    pub(super) fn clear(&self) {
        let map = {
            let mut sessions = self.map.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *sessions)
        };
        drop(map);
    }

    pub(super) fn get(&self, number: u64) -> Option<Arc<Session>> {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&number)
            .cloned()
    }

    pub(super) fn snapshot(&self) -> Vec<Arc<Session>> {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }
}
