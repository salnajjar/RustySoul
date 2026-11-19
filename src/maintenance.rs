//! Stopped-daemon SQLite maintenance, with private temporary rollback and crash recovery.
use crate::backup::{atomic_copy, copy_private, lock, private_file, regular, space, sync_dir};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Optimize,
    Compact,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    id: String,
    action: Action,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub action: Action,
    pub status: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub reclaimed_bytes: u64,
    pub at: u64,
    pub elapsed_ms: u64,
    pub message: String,
}
#[derive(Serialize)]
pub struct Stats {
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub reusable_bytes: u64,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn pending_dir(data: &Path) -> PathBuf {
    data.join("maintenance-pending")
}
pub(crate) fn pending(data: &Path) -> bool {
    pending_dir(data).exists()
}
fn open(path: &Path, write: bool) -> Result<Connection> {
    regular(path)?;
    let mut db = Connection::open_with_flags(
        path,
        if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        },
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA cache_size=-2048; PRAGMA mmap_size=0;")?;
    let deadline = Instant::now() + Duration::from_secs(1800);
    db.progress_handler(1000, Some(move || Instant::now() >= deadline))?;
    Ok(db)
}
fn check(db: &Connection) -> Result<()> {
    ensure!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok",
        "Database integrity check failed"
    );
    ensure!(
        !db.prepare("PRAGMA foreign_key_check")?.exists([])?,
        "Database relationships failed validation"
    );
    Ok(())
}
pub fn stats(data: &Path) -> Result<Stats> {
    let path = data.join("rustysoul.sqlite3");
    let db = open(&path, false)?;
    let page = db
        .pragma_query_value(None, "page_size", |r| r.get::<_, i64>(0))?
        .max(0) as u64;
    let free = db
        .pragma_query_value(None, "freelist_count", |r| r.get::<_, i64>(0))?
        .max(0) as u64;
    Ok(Stats {
        database_bytes: fs::metadata(&path)?.len(),
        wal_bytes: fs::metadata(data.join("rustysoul.sqlite3-wal")).map_or(0, |m| m.len()),
        reusable_bytes: page.saturating_mul(free),
    })
}
fn json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut f = private_file(path)?;
    f.write_all(&serde_json::to_vec(value)?)?;
    f.sync_all()?;
    Ok(())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    regular(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 8192,
        "Invalid maintenance metadata"
    );
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn record(data: &Path, report: &Report) -> Result<()> {
    let target = data.join("maintenance-last-result.json");
    let temp = data.join("maintenance-result.new");
    if temp.exists() {
        regular(&temp)?;
        fs::remove_file(&temp)?;
    }
    json_file(&temp, report)?;
    fs::rename(temp, target)?;
    sync_dir(data)
}
pub fn pending_id(data: &Path) -> Option<String> {
    read_json::<Job>(&pending_dir(data).join("job.json"))
        .ok()
        .map(|j| j.id)
}
pub fn last_result(data: &Path) -> Option<Report> {
    read_json(&data.join("maintenance-last-result.json")).ok()
}
pub fn stage(config: &crate::config::Config, action: Action) -> Result<String> {
    ensure!(
        !pending(&config.data_dir) && !config.data_dir.join("restore-pending").exists(),
        "A restart operation is already queued"
    );
    let stats = stats(&config.data_dir)?;
    space(
        &config.data_dir,
        stats
            .database_bytes
            .saturating_add(stats.wal_bytes)
            .saturating_mul(if action == Action::Compact { 3 } else { 2 }),
    )?;
    let temp = tempfile::Builder::new()
        .prefix(".rustysoul-maintenance-")
        .tempdir_in(&config.data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    json_file(
        &temp.path().join("job.json"),
        &Job {
            id: id.clone(),
            action,
        },
    )?;
    sync_dir(temp.path())?;
    fs::rename(temp.path(), pending_dir(&config.data_dir))?;
    sync_dir(&config.data_dir)?;
    Ok(id)
}
fn restore_original(data: &Path, job: &Path) -> Result<()> {
    let original = job.join("rollback.sqlite3");
    check(&open(&original, false)?)?;
    let database = data.join("rustysoul.sqlite3");
    for suffix in ["-wal", "-shm", "-journal"] {
        let path = data.join(format!("rustysoul.sqlite3{suffix}"));
        if path.exists() {
            regular(&path)?;
            fs::remove_file(path)?;
        }
    }
    atomic_copy(&original, &database)?;
    check(&open(&database, false)?)?;
    Ok(())
}
/// Run before opening the engine, auth or log writers. A verified result survives restart.
pub fn apply_pending(config: &crate::config::Config) -> Result<Option<Report>> {
    let data = &config.data_dir;
    let dir = pending_dir(data);
    if !dir.exists() {
        return Ok(None);
    }
    let _lock = lock(config)?;
    let meta = fs::symlink_metadata(&dir)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "Invalid maintenance folder"
    );
    let job: Job = read_json(&dir.join("job.json"))?;
    uuid::Uuid::parse_str(&job.id)?;
    if dir.join("done.json").exists() {
        let report: Report = read_json(&dir.join("done.json"))?;
        ensure!(report.id == job.id, "Invalid maintenance completion");
        check(&open(&data.join("rustysoul.sqlite3"), false)?)?;
        record(data, &report)?;
        fs::remove_dir_all(&dir)?;
        sync_dir(data)?;
        return Ok(Some(report));
    }
    let path = data.join("rustysoul.sqlite3");
    let mut before = fs::metadata(&path).map_or(0, |m| m.len());
    let start = Instant::now();
    let result = (|| -> Result<()> {
        ensure!(
            !dir.join("running").exists(),
            "Maintenance was interrupted; recovering the previous database"
        );
        let db = open(&path, true)?;
        check(&db)?;
        let busy: i64 = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        ensure!(
            busy == 0,
            "Database is busy. Run maintenance when indexing and backups are idle"
        );
        before = fs::metadata(&path)?.len();
        space(
            data,
            fs::metadata(&path)?
                .len()
                .saturating_mul(if job.action == Action::Compact { 3 } else { 2 }),
        )?;
        let rollback = dir.join("rollback.sqlite3");
        if rollback.exists() {
            regular(&rollback)?;
            fs::remove_file(&rollback)?;
        }
        copy_private(&path, &rollback)?;
        sync_dir(&dir)?;
        private_file(&dir.join("running"))?.sync_all()?;
        sync_dir(&dir)?;
        db.execute_batch("PRAGMA synchronous=FULL;")?;
        if job.action == Action::Compact {
            db.execute_batch("VACUUM;")?;
        }
        db.execute_batch("PRAGMA optimize=0x10002;")?;
        check(&db)?;
        let busy: i64 = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        ensure!(busy == 0, "Could not finish database checkpoint");
        drop(db);
        fs::File::open(&path)?.sync_all()?;
        sync_dir(data)?;
        Ok(())
    })();
    let (status, message) = match result {
        Ok(()) => (
            "completed",
            if job.action == Action::Compact {
                "Database compacted and query statistics optimized. Browser sessions and application data were preserved.".to_string()
            } else {
                "Query statistics optimized and the database checkpoint completed. Application data was preserved.".to_string()
            },
        ),
        Err(error) => {
            if dir.join("running").exists() {
                restore_original(data, &dir)
                    .context("Maintenance failed; could not recover the previous database")?;
            }
            check(&open(&path, false)?)?;
            (
                "failed",
                format!(
                    "Maintenance did not complete. Previous application data is intact. {error:#}"
                ),
            )
        }
    };
    let after = fs::metadata(&path)?.len();
    let report = Report {
        id: job.id,
        action: job.action,
        status: status.into(),
        before_bytes: before,
        after_bytes: after,
        reclaimed_bytes: before.saturating_sub(after),
        at: now(),
        elapsed_ms: start.elapsed().as_millis().min(u64::MAX as u128) as u64,
        message,
    };
    json_file(&dir.join("done.json"), &report)?;
    sync_dir(&dir)?;
    record(data, &report)?;
    fs::remove_dir_all(dir)?;
    sync_dir(data)?;
    Ok(Some(report))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, crate::config::Config) {
        let t = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(t.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let c = crate::config::Config {
            data_dir: t.path().into(),
            ..Default::default()
        };
        let s = crate::store::Store::open(&t.path().join("rustysoul.sqlite3")).unwrap();
        s.set_buddy("keep", true).unwrap();
        s.connection.execute_batch("CREATE TABLE temporary_fixture(x BLOB); INSERT INTO temporary_fixture VALUES(zeroblob(8388608)); DROP TABLE temporary_fixture;").unwrap();
        s.checkpoint().unwrap();
        (t, c)
    }
    #[test]
    fn compact_reclaims_pages_preserves_data_and_clears_its_private_rollback() {
        let (_t, c) = fixture();
        let initial = stats(&c.data_dir).unwrap();
        assert!(initial.reusable_bytes > 4 * 1024 * 1024);
        stage(&c, Action::Compact).unwrap();
        let r = apply_pending(&c).unwrap().unwrap();
        assert_eq!(r.status, "completed");
        assert!(r.reclaimed_bytes > 4 * 1024 * 1024);
        let s = crate::store::Store::open(&c.data_dir.join("rustysoul.sqlite3")).unwrap();
        assert_eq!(s.buddies().unwrap(), ["keep"]);
        assert!(!pending(&c.data_dir));
        assert_eq!(last_result(&c.data_dir).unwrap().id, r.id);
    }
    #[test]
    fn interrupted_maintenance_recovers_the_snapshot_and_a_live_lock_blocks_it() {
        let (_t, c) = fixture();
        stage(&c, Action::Optimize).unwrap();
        let held = lock(&c).unwrap();
        assert!(apply_pending(&c).is_err());
        drop(held);
        let dir = pending_dir(&c.data_dir);
        copy_private(
            &c.data_dir.join("rustysoul.sqlite3"),
            &dir.join("rollback.sqlite3"),
        )
        .unwrap();
        private_file(&dir.join("running")).unwrap();
        fs::write(c.data_dir.join("rustysoul.sqlite3"), b"interrupted").unwrap();
        let r = apply_pending(&c).unwrap().unwrap();
        assert_eq!(r.status, "failed");
        let s = crate::store::Store::open(&c.data_dir.join("rustysoul.sqlite3")).unwrap();
        assert_eq!(s.buddies().unwrap(), ["keep"]);
    }
    #[test]
    fn optimize_keeps_schema_and_integrity_and_rejects_duplicate_jobs() {
        let (_t, c) = fixture();
        stage(&c, Action::Optimize).unwrap();
        assert!(stage(&c, Action::Compact).is_err());
        let r = apply_pending(&c).unwrap().unwrap();
        assert_eq!(r.status, "completed");
        let db = open(&c.data_dir.join("rustysoul.sqlite3"), false).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            16
        );
        check(&db).unwrap();
    }
}
