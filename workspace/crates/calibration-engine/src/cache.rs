//! LRU cache of `FitResult` values backing `apply_fit_result(fit_id)`.
//!
//! Spec §5.3.2 dual-eviction policy:
//!   * capacity: 20 entries
//!   * age:      24 hours
//!
//! Extra rule (non-evicted last-applied snapshot): the entry most
//! recently `mark_applied()`'d is pinned and skipped by eviction. This
//! lets the UI read back the currently-active calibration details via
//! `get_current_fit_result` without worrying the user has rendered 20
//! variants since.

use calibration_types::FitResult;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, PartialEq)]
pub enum FitResultCacheError {
    #[error("fit_id {0} not found in cache")]
    NotFound(String),
    #[error("cache lock poisoned")]
    Poisoned,
}

struct Entry {
    value: FitResult,
    inserted: SystemTime,
    last_touched: u64, // monotonic ordinal for LRU
}

pub struct FitResultCache {
    inner: Mutex<CacheInner>,
    applied_id: RwLock<Option<String>>,
}

struct CacheInner {
    capacity: usize,
    max_age: Duration,
    ordinal: u64,
    entries: HashMap<String, Entry>,
    lru: VecDeque<(u64, String)>,
    // insert-order BTree for age-based eviction scans (cheap)
    by_inserted: BTreeMap<(SystemTime, String), ()>,
}

impl Default for FitResultCache {
    fn default() -> Self {
        Self::new(20, Duration::from_secs(60 * 60 * 24))
    }
}

impl FitResultCache {
    pub fn new(capacity: usize, max_age: Duration) -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                capacity: capacity.max(1),
                max_age,
                ordinal: 0,
                entries: HashMap::new(),
                lru: VecDeque::new(),
                by_inserted: BTreeMap::new(),
            }),
            applied_id: RwLock::new(None),
        }
    }

    pub fn insert(&self, mut value: FitResult) -> Result<(), FitResultCacheError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FitResultCacheError::Poisoned)?;
        let now = SystemTime::now();
        g.ordinal += 1;
        let ord = g.ordinal;
        let id = value.fit_id.clone();
        value.applied_at = None; // fresh cache slot has never been applied
        let entry = Entry {
            value,
            inserted: now,
            last_touched: ord,
        };
        g.entries.insert(id.clone(), entry);
        g.lru.push_back((ord, id.clone()));
        g.by_inserted.insert((now, id.clone()), ());
        let applied = self.applied_id.read().ok().and_then(|x| x.clone());
        g.evict(applied.as_deref());
        Ok(())
    }

    pub fn get(&self, fit_id: &str) -> Result<Option<FitResult>, FitResultCacheError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FitResultCacheError::Poisoned)?;
        g.ordinal += 1;
        let ord = g.ordinal;
        let applied = self.applied_id.read().ok().and_then(|x| x.clone());
        g.evict(applied.as_deref());
        if let Some(e) = g.entries.get_mut(fit_id) {
            e.last_touched = ord;
            let val = e.value.clone();
            g.lru.push_back((ord, fit_id.to_string()));
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// Mark a given fit_id as the active, currently-applied calibration.
    /// This entry becomes non-evictable until the next `mark_applied` call.
    pub fn mark_applied(&self, fit_id: &str) -> Result<(), FitResultCacheError> {
        *self
            .applied_id
            .write()
            .map_err(|_| FitResultCacheError::Poisoned)? = Some(fit_id.to_string());
        // Update applied_at in the stored record (bookkeeping).
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FitResultCacheError::Poisoned)?;
        if let Some(e) = g.entries.get_mut(fit_id) {
            if e.value.applied_at.is_none() {
                e.value.applied_at = Some(SystemTime::now());
            }
        }
        Ok(())
    }

    /// Returns the currently-applied FitResult, if any and if still cached.
    pub fn get_current(&self) -> Result<Option<FitResult>, FitResultCacheError> {
        let id = self
            .applied_id
            .read()
            .map_err(|_| FitResultCacheError::Poisoned)?
            .clone();
        match id {
            Some(id) => self.get(&id),
            None => Ok(None),
        }
    }

    pub fn len(&self) -> Result<usize, FitResultCacheError> {
        let g = self
            .inner
            .lock()
            .map_err(|_| FitResultCacheError::Poisoned)?;
        Ok(g.entries.len())
    }

    pub fn is_empty(&self) -> Result<bool, FitResultCacheError> {
        Ok(self.len()? == 0)
    }
}

impl CacheInner {
    fn evict(&mut self, pinned: Option<&str>) {
        let now = SystemTime::now();
        // Phase 1: age-based
        let mut aged_ids = vec![];
        for (k, _) in self.by_inserted.iter() {
            let (t, id) = k.clone();
            if pinned == Some(id.as_str()) {
                continue;
            }
            match now.duration_since(t) {
                Ok(d) if d > self.max_age => aged_ids.push((t, id)),
                _ => break, // BTree sorted ascending; older entries are first
            }
        }
        for (t, id) in aged_ids {
            self.by_inserted.remove(&(t, id.clone()));
            self.entries.remove(&id);
        }
        // Phase 2: size-based LRU. Compress lru deque lazily by peeking
        // oldest; if its entry's last_touched doesn't match, drop and loop.
        while self.entries.len() > self.capacity {
            let mut found = None;
            while let Some((ord, id)) = self.lru.front().cloned() {
                if pinned == Some(id.as_str()) {
                    self.lru.pop_front();
                    continue;
                }
                let mismatch = self
                    .entries
                    .get(&id)
                    .map(|e| e.last_touched != ord)
                    .unwrap_or(true);
                if mismatch {
                    self.lru.pop_front();
                    continue;
                }
                found = Some(id);
                self.lru.pop_front();
                break;
            }
            match found {
                Some(id) => {
                    if let Some(e) = self.entries.remove(&id) {
                        self.by_inserted.remove(&(e.inserted, id));
                    }
                }
                None => break, // nothing evictable, give up
            }
        }
    }
}

/// Helper: construct `Arc<FitResultCache>` in one call (phonon-tauri AppState
/// pattern).
pub fn new_arc_cache() -> Arc<FitResultCache> {
    Arc::new(FitResultCache::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(id: &str) -> FitResult {
        use calibration_types::FitCurves;
        FitResult {
            fit_id: id.to_string(),
            target_curve_id: "diffuse_field_flat".into(),
            bands_L: vec![],
            bands_R: vec![],
            fir_L: vec![],
            fir_R: vec![],
            curves_L: FitCurves::standard_log_grid(),
            curves_R: FitCurves::standard_log_grid(),
            rms_error_db: 0.0,
            peak_error_db: 0.0,
            applied_at: None,
        }
    }

    #[test]
    fn capacity_evicts_oldest_unpinned() {
        let c = FitResultCache::new(3, Duration::from_secs(3600));
        for i in 0..5u8 {
            c.insert(dummy(&format!("id-{i}"))).unwrap();
        }
        assert_eq!(c.len().unwrap(), 3);
        // id-0, id-1 evicted; id-2, id-3, id-4 remain
        assert!(c.get("id-0").unwrap().is_none());
        assert!(c.get("id-1").unwrap().is_none());
        assert!(c.get("id-2").unwrap().is_some());
        assert!(c.get("id-3").unwrap().is_some());
        assert!(c.get("id-4").unwrap().is_some());
    }

    #[test]
    fn pinned_entry_survives_capacity_eviction() {
        let c = FitResultCache::new(3, Duration::from_secs(3600));
        c.insert(dummy("pinned")).unwrap();
        c.mark_applied("pinned").unwrap();
        for i in 0..5u8 {
            c.insert(dummy(&format!("other-{i}"))).unwrap();
        }
        assert_eq!(c.len().unwrap(), 3);
        assert!(c.get("pinned").unwrap().is_some(), "pinned must survive");
        // pinned + the two latest others; other-0..=2 gone
        assert!(c.get("other-0").unwrap().is_none());
    }
}
