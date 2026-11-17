//! A soft free-space floor shared by all active files on a filesystem. Space
//! is probed off the control loop. Credits and outstanding writes prevent the
//! daemon's concurrent transfers spending the same observed free bytes twice.
use cap_std::fs::Dir;
use soulseek_rs::types::DownloadWriteGuard;
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_VOLUMES: usize = 64;
const PROBE_AGE: Duration = Duration::from_millis(250);
const MAX_CREDIT: u64 = 1024 * 1024;
pub const DEFAULT_MINIMUM_MIB: u64 = 1024;
pub const MAX_MINIMUM_MIB: u64 = 1_024_000_000;

#[derive(Default)]
struct Budget {
    available: Option<u64>,
    checked: Option<Instant>,
    spent: u64,
    pending: u64,
    error: Option<String>,
}
struct Volume {
    directory: Arc<Dir>,
    budget: Mutex<Budget>,
    probe: Mutex<()>,
}
#[derive(Default)]
struct Volumes {
    primary: Option<u64>,
    rows: BTreeMap<u64, Arc<Volume>>,
}
pub struct Guard {
    minimum: AtomicU64,
    volumes: Mutex<Volumes>,
}
impl Guard {
    pub fn new(minimum_mib: u64) -> Self {
        Self {
            minimum: AtomicU64::new(minimum_mib.saturating_mul(1024 * 1024)),
            volumes: Mutex::new(Volumes::default()),
        }
    }
    pub fn set_minimum(&self, minimum_mib: u64) {
        self.minimum
            .store(minimum_mib.saturating_mul(1024 * 1024), Ordering::Release);
    }
    pub fn enabled(&self) -> bool {
        self.minimum.load(Ordering::Acquire) > 0
    }
    pub fn register(&self, directory: &Dir, primary: bool) -> Result<u64, String> {
        #[cfg(unix)]
        let id = {
            use cap_std::fs::MetadataExt;
            directory.dir_metadata().map_err(|e| e.to_string())?.dev()
        };
        #[cfg(not(unix))]
        let id = 0;
        let mut volumes = self
            .volumes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !volumes.rows.contains_key(&id) {
            if volumes.rows.len() >= MAX_VOLUMES {
                return Err("Waiting for a storage monitoring slot; too many destination filesystems are active.".into());
            }
            volumes.rows.insert(
                id,
                Arc::new(Volume {
                    directory: Arc::new(directory.try_clone().map_err(|e| e.to_string())?),
                    budget: Mutex::new(Budget::default()),
                    probe: Mutex::new(()),
                }),
            );
        }
        if primary {
            volumes.primary = Some(id);
        }
        Ok(id)
    }
    fn volume(&self, id: u64) -> Option<Arc<Volume>> {
        self.volumes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .rows
            .get(&id)
            .cloned()
    }
    pub fn primary(&self) -> Option<u64> {
        self.volumes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .primary
    }
    pub fn update_primary(&self, path: &std::path::Path) {
        if let Ok(dir) = Dir::open_ambient_dir(path, cap_std::ambient_authority()) {
            let _ = self.register(&dir, true);
        } else {
            self.volumes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .primary = None;
        }
    }
    pub fn retain(&self, ids: &HashSet<u64>) {
        let mut volumes = self
            .volumes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let primary = volumes.primary;
        volumes.rows.retain(|id, v| {
            Some(*id) == primary
                || ids.contains(id)
                || v.budget
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .pending
                    > 0
        });
    }
    pub fn refresh(&self) {
        if !self.enabled() {
            return;
        }
        let rows: Vec<_> = self
            .volumes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .rows
            .values()
            .cloned()
            .collect();
        for v in rows {
            if let Ok(_probe) = v.probe.try_lock() {
                Self::probe(&v, &*v.directory);
            }
        }
    }
    fn probe(volume: &Volume, file: impl std::os::fd::AsFd) {
        let before = volume
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .spent;
        let result = available(file);
        Self::record(volume, result, before);
    }
    fn record(volume: &Volume, result: Result<u64, String>, before: u64) {
        let mut budget = volume
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        budget.checked = Some(Instant::now());
        // Writes released while the syscall ran may not be reflected in its sample.
        budget.spent = budget.spent.saturating_sub(before);
        match result {
            Ok(n) => {
                budget.available = Some(n);
                budget.error = None;
            }
            Err(e) => {
                budget.available = None;
                budget.error = Some(e);
            }
        }
    }
    pub fn can_start(&self, id: u64, size: u64) -> Result<(), String> {
        if !self.enabled() {
            return Ok(());
        }
        let v = self.volume(id).ok_or("Checking destination storage…")?;
        let b = v
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if b.checked
            .is_none_or(|t| t.elapsed() > Duration::from_secs(15))
        {
            return Err("Checking destination storage…".into());
        }
        let required = self
            .minimum
            .load(Ordering::Acquire)
            .saturating_add(size.min(64 * 1024));
        if b.available
            .is_some_and(|n| n.saturating_sub(b.spent).saturating_sub(b.pending) >= required)
        {
            Ok(())
        } else {
            Err(b.error.clone().unwrap_or_else(||"Waiting for free disk space. Partial files are kept and downloads resume automatically when space is available.".into()))
        }
    }
}
#[cfg(unix)]
fn available(file: impl std::os::fd::AsFd) -> Result<u64, String> {
    let s = rustix::fs::fstatvfs(file).map_err(|e| e.to_string())?;
    s.f_bavail
        .checked_mul(s.f_frsize)
        .ok_or_else(|| "Filesystem capacity overflow".into())
}
#[cfg(not(unix))]
fn available(_file: impl std::fmt::Debug) -> Result<u64, String> {
    Err("Storage protection requires Unix".into())
}

impl DownloadWriteGuard for Guard {
    fn reserve(&self, file: &File, bytes: usize) -> Result<Option<u64>, String> {
        let minimum = self.minimum.load(Ordering::Acquire);
        if minimum == 0 || bytes == 0 {
            return Ok(None);
        }
        #[cfg(unix)]
        let id = {
            use std::os::unix::fs::MetadataExt;
            file.metadata().map_err(|e| e.to_string())?.dev()
        };
        #[cfg(not(unix))]
        let id = 0;
        let v = self
            .volume(id)
            .ok_or("Destination storage has not been checked yet.")?;
        let needs_probe = {
            let b = v
                .budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            b.checked.is_none_or(|t| t.elapsed() >= PROBE_AGE) || b.spent >= MAX_CREDIT
        };
        if needs_probe {
            if let Ok(_probe) = v.probe.try_lock() {
                Self::probe(&v, file);
            } else if v
                .budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .checked
                .is_none_or(|t| t.elapsed() > Duration::from_secs(1))
            {
                return Err("A destination storage check is in progress. The download will resume automatically.".into());
            }
        }
        let minimum = self.minimum.load(Ordering::Acquire);
        if minimum == 0 {
            return Ok(None);
        }
        let mut b = v
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let room = b
            .available
            .unwrap_or(0)
            .saturating_sub(b.spent)
            .saturating_sub(b.pending);
        if room < minimum.saturating_add(bytes as u64) {
            return Err(b.error.clone().unwrap_or_else(|| {
                "The configured free-space reserve has been reached. Partial files are kept.".into()
            }));
        }
        b.pending = b.pending.saturating_add(bytes as u64);
        Ok(Some(id))
    }
    fn release(&self, token: Option<u64>, bytes: usize) {
        if let Some(v) = token.and_then(|id| self.volume(id)) {
            let mut b = v
                .budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            b.pending = b.pending.saturating_sub(bytes as u64);
            b.spent = b.spent.saturating_add(bytes as u64);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn setup(room: u64) -> (tempfile::TempDir, Guard, u64, File) {
        let temp = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let guard = Guard::new(1);
        let id = guard.register(&dir, true).unwrap();
        let file = File::create(temp.path().join("part")).unwrap();
        Guard::record(&guard.volume(id).unwrap(), Ok(1024 * 1024 + room), 0);
        (temp, guard, id, file)
    }
    #[test]
    fn concurrent_writes_share_one_budget_and_released_bytes_remain_spent() {
        let (_temp, guard, id, file) = setup(100);
        let first = guard.reserve(&file, 60).unwrap();
        assert_eq!(first, Some(id));
        assert!(guard.reserve(&file, 41).is_err());
        guard.release(first, 60);
        assert!(guard.reserve(&file, 41).is_err());
        let last = guard.reserve(&file, 40).unwrap();
        guard.release(last, 40);
        assert!(guard.reserve(&file, 1).is_err());
        assert!(guard.can_start(id, 1).is_err());
    }
    #[test]
    fn sampling_preserves_writes_released_during_the_syscall() {
        let (_temp, guard, id, file) = setup(100);
        let token = guard.reserve(&file, 60).unwrap();
        let volume = guard.volume(id).unwrap();
        let before = volume.budget.lock().unwrap().spent;
        guard.release(token, 60);
        Guard::record(&volume, Ok(1024 * 1024 + 100), before);
        assert!(guard.reserve(&file, 41).is_err());
    }
    #[test]
    fn another_probe_uses_fresh_cached_credits_without_false_holds() {
        let (_temp, guard, id, file) = setup(100);
        let volume = guard.volume(id).unwrap();
        volume.budget.lock().unwrap().checked = Some(Instant::now() - Duration::from_millis(300));
        let _probe = volume.probe.lock().unwrap();
        let token = guard.reserve(&file, 60).unwrap();
        guard.release(token, 60);
        assert!(guard.reserve(&file, 41).is_err());
        volume.budget.lock().unwrap().checked = Some(Instant::now() - Duration::from_secs(2));
        assert!(guard.reserve(&file, 1).is_err());
    }
    #[test]
    fn live_disable_and_policy_changes_take_effect_without_recreating_guard() {
        let (_temp, guard, id, file) = setup(100);
        guard.set_minimum(2);
        assert!(guard.reserve(&file, 1).is_err());
        guard.set_minimum(0);
        assert_eq!(guard.reserve(&file, 1000).unwrap(), None);
        assert!(guard.can_start(id, u64::MAX).is_ok());
        guard.set_minimum(1);
        assert!(guard.reserve(&file, 100).is_ok());
    }
    #[test]
    fn same_filesystem_deduplicates_and_pending_writes_pin_only_its_directory() {
        let (temp, guard, id, file) = setup(100);
        std::fs::create_dir(temp.path().join("other")).unwrap();
        let other =
            Dir::open_ambient_dir(temp.path().join("other"), cap_std::ambient_authority()).unwrap();
        assert_eq!(guard.register(&other, false).unwrap(), id);
        assert_eq!(guard.volumes.lock().unwrap().rows.len(), 1);
        guard.volumes.lock().unwrap().primary = None;
        let token = guard.reserve(&file, 10).unwrap();
        guard.retain(&HashSet::new());
        assert!(guard.volume(id).is_some());
        guard.release(token, 10);
        guard.retain(&HashSet::new());
        assert!(guard.volume(id).is_none());
    }
    #[test]
    fn unknown_stale_and_failed_samples_hold_until_a_successful_refresh() {
        let (_temp, guard, id, _file) = setup(100);
        let volume = guard.volume(id).unwrap();
        volume.budget.lock().unwrap().checked = None;
        assert!(guard.can_start(id, 1).is_err());
        Guard::record(&volume, Err("Unavailable destination".into()), 0);
        assert_eq!(
            guard.can_start(id, 1).unwrap_err(),
            "Unavailable destination"
        );
        Guard::record(&volume, Ok(1024 * 1024 + 65536), 0);
        assert!(guard.can_start(id, u64::MAX).is_ok());
        volume.budget.lock().unwrap().checked = Some(Instant::now() - Duration::from_secs(16));
        assert!(guard.can_start(id, 1).is_err());
    }
    #[test]
    fn older_settings_default_and_api_bounds_are_explicit() {
        let settings: crate::model::NetworkSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.minimum_free_space_mib, 1024);
        assert!(
            crate::model::Command::UpdateStorageReserve {
                minimum_free_space_mib: 0
            }
            .validate()
            .is_ok()
        );
        assert!(
            crate::model::Command::UpdateStorageReserve {
                minimum_free_space_mib: MAX_MINIMUM_MIB
            }
            .validate()
            .is_ok()
        );
        assert!(
            crate::model::Command::UpdateStorageReserve {
                minimum_free_space_mib: MAX_MINIMUM_MIB + 1
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_str::<crate::model::Command>(
                r#"{"type":"update_storage_reserve","minimum_free_space_mib":-1}"#
            )
            .is_err()
        );
    }
}
