//! Bounded, read-only operational sampling. Never walks media directories or
//! records samples in SQLite. A single worker keeps filesystem/procfs reads off
//! both the protocol control loop and HTTP runtime workers.
use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

pub const INTERVAL: Duration = Duration::from_secs(5);
const STATES: [&str; 16] = [
    "waiting",
    "waiting_for_space",
    "queued",
    "downloading",
    "uploading",
    "finalizing",
    "paused",
    "retry_waiting",
    "completed",
    "failed",
    "cancelled",
    "interrupted",
    "timed_out",
    "checking_shares",
    "blocked",
    "other",
];

#[derive(Clone, Default, Serialize)]
pub struct Process {
    pub resident_bytes: Option<u64>,
    pub threads: Option<u64>,
    pub cpu_seconds: Option<f64>,
    /// 100% means one fully occupied CPU core, not the entire host.
    pub cpu_percent: Option<f64>,
}
#[derive(Clone, Default, Serialize)]
pub struct Limits {
    pub memory_bytes: Option<u64>,
    pub memory_used_bytes: Option<u64>,
    pub memory_scope: Option<&'static str>,
    pub tasks: Option<u64>,
    pub tasks_used: Option<u64>,
    pub tasks_scope: Option<&'static str>,
    pub cpu_cores: Option<f64>,
}
#[derive(Clone, Default, Serialize)]
pub struct Storage {
    pub path: String,
    pub total_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub error: Option<String>,
}
#[derive(Clone, Default, Serialize)]
pub struct Transfers {
    pub total: u64,
    pub states: BTreeMap<String, u64>,
    pub speed: f64,
}
#[derive(Clone, Default, Serialize)]
pub struct View {
    pub sampled_at: u64,
    pub uptime_seconds: u64,
    pub process: Process,
    pub limits: Limits,
    pub data: Storage,
    pub downloads: Storage,
    pub database_bytes: Option<u64>,
    pub wal_bytes: Option<u64>,
    pub download_transfers: Option<Transfers>,
    pub upload_transfers: Option<Transfers>,
    pub process_error: Option<String>,
    pub database_error: Option<String>,
}

pub struct Monitor {
    wake: mpsc::SyncSender<()>,
    paths: Arc<Mutex<(PathBuf, PathBuf)>>,
    latest: watch::Receiver<View>,
}
impl Monitor {
    pub fn start(data: PathBuf, downloads: PathBuf, started: Instant) -> Result<Self> {
        Self::start_with_storage(data, downloads, started, None)
    }
    pub fn start_with_storage(
        data: PathBuf,
        downloads: PathBuf,
        started: Instant,
        storage: Option<Arc<crate::storage::Guard>>,
    ) -> Result<Self> {
        let paths = Arc::new(Mutex::new((data, downloads)));
        let (wake, requests) = mpsc::sync_channel(1);
        let (updates, latest) = watch::channel(View::default());
        let worker_paths = paths.clone();
        std::thread::Builder::new()
            .name("system-status".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let mut previous_cpu = None;
                loop {
                    let paths = worker_paths
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    let mut view = sample(&paths.0, &paths.1, started);
                    if let Some(guard) = &storage {
                        guard.update_primary(&paths.1);
                        guard.refresh();
                    }
                    let time = Instant::now();
                    if let Some(seconds) = view.process.cpu_seconds {
                        view.process.cpu_percent = cpu_percent(previous_cpu, seconds, time);
                        previous_cpu = Some((seconds, time));
                    } else {
                        previous_cpu = None;
                    }
                    updates.send_replace(view);
                    match requests.recv_timeout(INTERVAL) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .context("Could not start the system status worker")?;
        Ok(Self {
            wake,
            paths,
            latest,
        })
    }
    pub fn set_downloads(&self, path: PathBuf) {
        self.paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1 = path;
        let _ = self.wake.try_send(());
    }
    pub fn latest(&self) -> View {
        let mut view = self.latest.borrow().clone();
        let paths = self
            .paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A slow old sample may finish after a destination change. Do not show
        // its capacity as the capacity of the newly configured destination.
        if view.downloads.path != paths.1.to_string_lossy() {
            view.downloads = Storage {
                path: paths.1.to_string_lossy().into_owned(),
                ..Default::default()
            };
        }
        view
    }
    pub fn request_refresh(&self) {
        let _ = self.wake.try_send(());
    }
}
fn cpu_percent(previous: Option<(f64, Instant)>, seconds: f64, at: Instant) -> Option<f64> {
    let (old, time) = previous?;
    let elapsed = at.saturating_duration_since(time).as_secs_f64();
    (elapsed > 0. && seconds >= old)
        .then(|| (seconds - old) / elapsed * 100.)
        .filter(|v| v.is_finite())
}
fn value(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        line.strip_prefix(key)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}
fn process_from(status: &str, stat: &str, ticks: u64) -> Option<Process> {
    let rest = stat.rsplit_once(") ")?.1;
    let fields: Vec<_> = rest.split_whitespace().collect();
    let user: u64 = fields.get(11)?.parse().ok()?;
    let kernel: u64 = fields.get(12)?.parse().ok()?;
    let cpu = (ticks > 0).then(|| user.saturating_add(kernel) as f64 / ticks as f64);
    Some(Process {
        resident_bytes: value(status, "VmRSS:").and_then(|v| v.checked_mul(1024)),
        threads: value(status, "Threads:"),
        cpu_seconds: cpu,
        cpu_percent: None,
    })
}
fn process() -> Result<Process> {
    #[cfg(target_os = "linux")]
    {
        process_from(
            &std::fs::read_to_string("/proc/self/status")?,
            &std::fs::read_to_string("/proc/self/stat")?,
            rustix::param::clock_ticks_per_second(),
        )
        .context("Process statistics are unavailable")
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Process statistics require Linux")
    }
}
fn storage(path: &Path) -> Storage {
    let mut view = Storage {
        path: path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    #[cfg(unix)]
    let result = (|| -> Result<(u64, u64, u64)> {
        let file = std::fs::File::open(path)?;
        let stat = rustix::fs::fstatvfs(file)?;
        let block = stat.f_frsize;
        let total = stat
            .f_blocks
            .checked_mul(block)
            .context("Filesystem capacity overflow")?;
        let free = stat
            .f_bfree
            .checked_mul(block)
            .context("Filesystem capacity overflow")?;
        let available = stat
            .f_bavail
            .checked_mul(block)
            .context("Filesystem capacity overflow")?;
        Ok((total, total.saturating_sub(free), available.min(total)))
    })();
    #[cfg(not(unix))]
    let result: Result<(u64, u64, u64)> =
        Err(anyhow::anyhow!("Filesystem statistics require Unix"));
    match result {
        Ok((total, used, available)) => {
            view.total_bytes = Some(total);
            view.used_bytes = Some(used);
            view.available_bytes = Some(available);
        }
        Err(e) => view.error = Some(e.to_string()),
    }
    view
}
fn numeric(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
fn cgroup_dir(root: &Path, membership: &str) -> Option<PathBuf> {
    let group = membership.lines().find_map(|s| s.strip_prefix("0::"))?;
    let relative = Path::new(group.strip_prefix('/')?);
    if relative
        .components()
        .any(|p| !matches!(p, Component::Normal(_)))
    {
        return None;
    }
    let candidate = root.join(relative);
    if candidate.join("cgroup.controllers").exists() {
        Some(candidate)
    } else if root.join("cgroup.controllers").exists() {
        Some(root.to_path_buf())
    } else {
        None
    }
}
fn cgroup_limits(root: &Path, leaf: &Path) -> Limits {
    let mut view = Limits {
        memory_used_bytes: numeric(&leaf.join("memory.current")),
        tasks_used: numeric(&leaf.join("pids.current")),
        ..Default::default()
    };
    // Pair usage with its own limit. An ancestor shared with other workloads
    // may have less headroom than a smaller leaf limit.
    let mut memory_headroom = None;
    let mut task_headroom = None;
    for path in leaf
        .ancestors()
        .take(32)
        .take_while(|p| p.starts_with(root))
    {
        for (name, current, limit, used, scope, headroom) in [
            (
                "memory.max",
                "memory.current",
                &mut view.memory_bytes,
                &mut view.memory_used_bytes,
                &mut view.memory_scope,
                &mut memory_headroom,
            ),
            (
                "pids.max",
                "pids.current",
                &mut view.tasks,
                &mut view.tasks_used,
                &mut view.tasks_scope,
                &mut task_headroom,
            ),
        ] {
            if let Some(n) = numeric(&path.join(name)) {
                let current = numeric(&path.join(current));
                let left = current.map(|used| n.saturating_sub(used));
                if limit.is_none()
                    || left.is_some_and(|remaining| headroom.is_none_or(|old| remaining < old))
                {
                    *limit = Some(n);
                    *used = current;
                    *headroom = left;
                    *scope = Some(if path == leaf { "self" } else { "ancestor" });
                }
            }
        }
        if let Ok(raw) = std::fs::read_to_string(path.join("cpu.max")) {
            let mut fields = raw.split_whitespace();
            if let (Some(quota), Some(period)) = (
                fields.next().and_then(|s| s.parse::<u64>().ok()),
                fields.next().and_then(|s| s.parse::<u64>().ok()),
            ) && period > 0
            {
                let n = quota as f64 / period as f64;
                view.cpu_cores = Some(view.cpu_cores.map_or(n, |old| old.min(n)));
            }
        }
    }
    view
}
fn limits() -> Limits {
    let root = Path::new("/sys/fs/cgroup");
    std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|s| cgroup_dir(root, &s))
        .map(|p| cgroup_limits(root, &p))
        .unwrap_or_default()
}
fn transfer_counts(db: &rusqlite::Connection, uploads: bool) -> Result<Transfers> {
    let table = if uploads { "uploads" } else { "downloads" };
    let mut view = Transfers {
        states: STATES.iter().map(|s| ((*s).to_owned(), 0)).collect(),
        ..Default::default()
    };
    for row in db
        .prepare(&format!(
            "SELECT status,COUNT(*),COALESCE(SUM(speed),0) FROM {table} GROUP BY status"
        ))?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })?
    {
        let (state, count, speed) = row?;
        let count = u64::try_from(count)?;
        view.total = view.total.saturating_add(count);
        *view
            .states
            .get_mut(if STATES.contains(&state.as_str()) {
                state.as_str()
            } else {
                "other"
            })
            .unwrap() += count;
        if matches!(state.as_str(), "downloading" | "uploading") && speed.is_finite() && speed >= 0.
        {
            let sum = view.speed + speed;
            if sum.is_finite() {
                view.speed = sum;
            }
        }
    }
    Ok(view)
}
fn sample(data: &Path, downloads: &Path, started: Instant) -> View {
    let mut view = View {
        sampled_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        uptime_seconds: started.elapsed().as_secs(),
        data: storage(data),
        downloads: storage(downloads),
        limits: limits(),
        ..Default::default()
    };
    match process() {
        Ok(p) => view.process = p,
        Err(e) => view.process_error = Some(e.to_string()),
    };
    let path = data.join("rustysoul.sqlite3");
    view.database_bytes = std::fs::metadata(&path).ok().map(|m| m.len());
    // A missing WAL is normal once SQLite has checkpointed it.
    view.wal_bytes = match std::fs::metadata(data.join("rustysoul.sqlite3-wal")) {
        Ok(m) => Some(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    };
    let result = (|| -> Result<(Transfers, Transfers)> {
        let mut db = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(250))?;
        let tx = db.transaction()?;
        Ok((transfer_counts(&tx, false)?, transfer_counts(&tx, true)?))
    })();
    match result {
        Ok((d, u)) => {
            view.download_transfers = Some(d);
            view.upload_transfers = Some(u);
        }
        Err(e) => view.database_error = Some(e.to_string()),
    };
    view
}

/// Prometheus 0.0.4 text. Labels come from fixed enums only: never user names,
/// paths, rooms, queries, errors or any credential/session information.
pub fn metrics(state: &crate::model::Snapshot, now: u64) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(8192);
    let v = &state.system;
    let mut metric = |name: &str, help: &str, kind: &str, value: Option<f64>| {
        if let Some(value) = value.filter(|v| v.is_finite()) {
            let _ = writeln!(
                out,
                "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}"
            );
        }
    };
    metric(
        "rustysoul_network_connected",
        "Whether the Soulseek session is connected.",
        "gauge",
        Some(if state.connection == "connected" {
            1.
        } else {
            0.
        }),
    );
    metric(
        "rustysoul_library_files",
        "Files in the current shared index.",
        "gauge",
        Some(f64::from(state.shares.files)),
    );
    metric(
        "rustysoul_library_folders",
        "Folders in the current shared index.",
        "gauge",
        Some(f64::from(state.shares.folders)),
    );
    metric(
        "rustysoul_library_scanning",
        "Whether a library scan is running.",
        "gauge",
        Some(if state.library.scanning { 1. } else { 0. }),
    );
    metric(
        "rustysoul_download_queue_used",
        "Unfinished downloads using configured queue capacity.",
        "gauge",
        Some(state.download_capacity_used as f64),
    );
    metric(
        "rustysoul_download_queue_capacity",
        "Configured maximum download queue capacity.",
        "gauge",
        Some(state.transfer_capacity as f64),
    );
    metric(
        "rustysoul_effective_peer_limit",
        "Effective peer actor capacity after resource budgeting.",
        "gauge",
        Some(state.effective_peer_limit as f64),
    );
    metric(
        "rustysoul_payload_downloaded_bytes_total",
        "Lifetime received file payload bytes, excluding resume offsets.",
        "counter",
        Some(state.traffic.downloaded as f64),
    );
    metric(
        "rustysoul_payload_uploaded_bytes_total",
        "Lifetime sent file payload bytes, excluding resume offsets.",
        "counter",
        Some(state.traffic.uploaded as f64),
    );
    metric(
        "rustysoul_system_sample_timestamp_seconds",
        "Unix time of the last operational sample.",
        "gauge",
        Some(v.sampled_at as f64),
    );
    if v.sampled_at > 0 {
        metric(
            "rustysoul_system_sample_age_seconds",
            "Age of the cached operational sample.",
            "gauge",
            Some(now.saturating_sub(v.sampled_at) as f64),
        );
        metric(
            "rustysoul_uptime_seconds",
            "Daemon uptime since startup.",
            "gauge",
            Some(v.uptime_seconds as f64),
        );
    }
    for (name, help, value) in [
        (
            "rustysoul_process_resident_memory_bytes",
            "Daemon resident memory in bytes.",
            v.process.resident_bytes,
        ),
        (
            "rustysoul_process_threads",
            "Number of daemon threads.",
            v.process.threads,
        ),
        (
            "rustysoul_cgroup_memory_limit_bytes",
            "Memory limit with the smallest observed cgroup headroom.",
            v.limits.memory_bytes,
        ),
        (
            "rustysoul_cgroup_memory_used_bytes",
            "Memory charged to the reported limiting cgroup.",
            v.limits.memory_used_bytes,
        ),
        (
            "rustysoul_cgroup_tasks_limit",
            "Task limit with the smallest observed cgroup headroom.",
            v.limits.tasks,
        ),
        (
            "rustysoul_cgroup_tasks_used",
            "Tasks charged to the reported limiting cgroup.",
            v.limits.tasks_used,
        ),
        (
            "rustysoul_database_bytes",
            "Main SQLite database size, excluding WAL.",
            v.database_bytes,
        ),
        (
            "rustysoul_database_wal_bytes",
            "Current SQLite WAL size.",
            v.wal_bytes,
        ),
    ] {
        metric(name, help, "gauge", value.map(|v| v as f64));
    }
    metric(
        "rustysoul_process_cpu_seconds_total",
        "Daemon user and kernel CPU time.",
        "counter",
        v.process.cpu_seconds,
    );
    metric(
        "rustysoul_process_cpu_percent",
        "Recent CPU use where one busy core equals 100 percent.",
        "gauge",
        v.process.cpu_percent,
    );
    metric(
        "rustysoul_cgroup_cpu_cores",
        "Smallest reported cgroup CPU quota in cores.",
        "gauge",
        v.limits.cpu_cores,
    );
    for (role, s) in [("data", &v.data), ("downloads", &v.downloads)] {
        for (suffix, help, value) in [
            ("total_bytes", "Filesystem total capacity.", s.total_bytes),
            ("used_bytes", "Filesystem used capacity.", s.used_bytes),
            (
                "available_bytes",
                "Filesystem space available to the daemon user.",
                s.available_bytes,
            ),
        ] {
            let name = format!("rustysoul_storage_{role}_{suffix}");
            metric(&name, help, "gauge", value.map(|v| v as f64));
        }
    }
    for (direction, t) in [
        ("download", &v.download_transfers),
        ("upload", &v.upload_transfers),
    ] {
        if let Some(t) = t {
            metric(
                &format!("rustysoul_{direction}_speed_bytes_per_second"),
                "Current aggregate payload speed over all transfer rows.",
                "gauge",
                Some(t.speed),
            );
        }
    }
    let _ = writeln!(
        out,
        "# HELP rustysoul_transfers Retained transfer records by direction and state.\n# TYPE rustysoul_transfers gauge"
    );
    for (direction, t) in [
        ("download", &v.download_transfers),
        ("upload", &v.upload_transfers),
    ] {
        if let Some(t) = t {
            for status in STATES {
                let _ = writeln!(
                    out,
                    "rustysoul_transfers{{direction=\"{direction}\",state=\"{status}\"}} {}",
                    t.states.get(status).copied().unwrap_or(0)
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_parser_handles_spaces_and_parentheses_and_cpu_units() {
        let mut fields = vec!["0"; 20];
        fields[0] = "S";
        fields[11] = "150";
        fields[12] = "50";
        let stat = format!("123 (name with ) spaces) {}", fields.join(" "));
        let v = process_from("VmRSS:\t2048 kB\nThreads:\t7\n", &stat, 100).unwrap();
        assert_eq!(v.resident_bytes, Some(2 * 1024 * 1024));
        assert_eq!(v.threads, Some(7));
        assert_eq!(v.cpu_seconds, Some(2.));
        assert!(process_from("", "broken stat", 100).is_none());
        assert!(
            process_from("VmRSS: invalid", &stat, 0)
                .unwrap()
                .cpu_seconds
                .is_none()
        );
    }
    #[test]
    fn cpu_samples_handle_multiple_cores_counter_resets_and_first_samples() {
        let at = Instant::now();
        assert_eq!(
            cpu_percent(Some((1., at)), 5., at + Duration::from_secs(2)),
            Some(200.)
        );
        assert!(cpu_percent(None, 2., at).is_none());
        assert!(cpu_percent(Some((2., at)), 1., at + INTERVAL).is_none());
        assert!(cpu_percent(Some((1., at)), 2., at).is_none());
    }
    #[test]
    fn cgroup_hierarchy_pairs_limits_with_usage_and_finds_shared_parent_pressure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let leaf = root.join("service/daemon");
        std::fs::create_dir_all(&leaf).unwrap();
        std::fs::write(root.join("cgroup.controllers"), "memory pids cpu").unwrap();
        std::fs::write(leaf.join("cgroup.controllers"), "").unwrap();
        for (path, files) in [
            (
                root,
                [
                    ("memory.max", "2000"),
                    ("memory.current", "1900"),
                    ("pids.max", "500"),
                    ("pids.current", "400"),
                    ("cpu.max", "50000 100000"),
                ],
            ),
            (
                leaf.as_path(),
                [
                    ("memory.max", "1000"),
                    ("memory.current", "200"),
                    ("pids.max", "128"),
                    ("pids.current", "20"),
                    ("cpu.max", "200000 100000"),
                ],
            ),
        ] {
            for (name, text) in files {
                std::fs::write(path.join(name), text).unwrap();
            }
        }
        assert_eq!(cgroup_dir(root, "0::/service/daemon\n"), Some(leaf.clone()));
        assert_eq!(
            cgroup_dir(root, "0::/container-host-prefix\n"),
            Some(root.into())
        );
        assert!(cgroup_dir(root, "0::/../outside").is_none());
        assert!(cgroup_dir(root, "4:memory:/").is_none());
        let v = cgroup_limits(root, &leaf);
        assert_eq!(
            (v.memory_bytes, v.memory_used_bytes),
            (Some(2000), Some(1900))
        );
        assert_eq!(v.memory_scope, Some("ancestor"));
        assert_eq!((v.tasks, v.tasks_used), (Some(500), Some(400)));
        assert_eq!(v.tasks_scope, Some("ancestor"));
        assert_eq!(v.cpu_cores, Some(0.5));
        for path in [root, leaf.as_path()] {
            for name in ["memory.max", "pids.max", "cpu.max"] {
                std::fs::write(path.join(name), "max").unwrap();
            }
        }
        let v = cgroup_limits(root, &leaf);
        assert!(v.memory_bytes.is_none() && v.tasks.is_none() && v.cpu_cores.is_none());
        assert_eq!(v.memory_used_bytes, Some(200));
    }
    #[test]
    fn filesystem_reports_available_capacity_and_unavailable_paths_without_fake_zeroes() {
        let dir = tempfile::tempdir().unwrap();
        let v = storage(dir.path());
        assert!(v.total_bytes.unwrap() > 0);
        assert!(v.available_bytes.unwrap() <= v.total_bytes.unwrap());
        assert!(v.used_bytes.unwrap() <= v.total_bytes.unwrap());
        assert!(v.error.is_none());
        let missing = storage(&dir.path().join("missing"));
        assert!(missing.available_bytes.is_none());
        assert!(missing.error.is_some());
    }
    #[test]
    fn readonly_samples_count_all_transfer_rows_and_metrics_never_expose_private_labels() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        let tx = crate::store::write_transaction(&db.connection).unwrap();
        for n in 0..315 {
            let (status, speed) = if n < 300 {
                ("completed", 0)
            } else if n < 305 {
                ("downloading", 123)
            } else if n < 310 {
                ("waiting", 0)
            } else {
                ("failed", 0)
            };
            tx.execute("INSERT INTO downloads(username,path,size,status,speed) VALUES('private-peer',?1,10,?2,?3)",rusqlite::params![format!("private/{n}"),status,speed]).unwrap();
        }
        tx.execute("INSERT INTO uploads(username,path,size,status,transferred,speed,added_at,updated_at) VALUES('private-peer','private-file',20,'uploading',10,20,1,1)",[]).unwrap();
        tx.commit().unwrap();
        let v = sample(
            dir.path(),
            dir.path(),
            Instant::now() - Duration::from_secs(17),
        );
        assert!(v.database_error.is_none());
        assert!(v.database_bytes.unwrap() > 0);
        assert!(v.uptime_seconds >= 17);
        let d = v.download_transfers.as_ref().unwrap();
        assert_eq!(d.total, 315);
        assert_eq!(d.states["completed"], 300);
        assert_eq!(d.states["failed"], 5);
        assert_eq!(d.speed, 615.);
        assert_eq!(v.upload_transfers.as_ref().unwrap().speed, 20.);
        let mut state = crate::model::Snapshot {
            system: v,
            username: "TOP-SECRET".into(),
            ..Default::default()
        };
        state.system.data.path = "TOP-SECRET".into();
        state.system.database_error = Some("TOP-SECRET".into());
        state.traffic.downloaded = 987;
        state
            .system
            .download_transfers
            .as_mut()
            .unwrap()
            .states
            .insert("injected\"\nTOP-SECRET".into(), 99);
        let text = metrics(&state, state.system.sampled_at + 7);
        assert!(
            !text.contains("TOP-SECRET")
                && !text.contains("private-peer")
                && !text.contains("private-file")
                && !text.contains("injected")
        );
        assert!(
            text.contains("rustysoul_transfers{direction=\"download\",state=\"completed\"} 300\n")
        );
        assert!(text.contains("rustysoul_payload_downloaded_bytes_total 987\n"));
        assert!(text.ends_with('\n'));
        let mut keys = std::collections::HashSet::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let (key, value) = line.rsplit_once(' ').unwrap();
            assert!(value.parse::<f64>().unwrap().is_finite());
            assert!(keys.insert(key));
        }
        assert!(keys.len() < 100);
        assert_eq!(
            db.connection
                .query_row("SELECT COUNT(*) FROM downloads", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            315
        );
    }
    #[test]
    fn missing_database_readings_are_absent_instead_of_reported_as_empty_history() {
        let dir = tempfile::tempdir().unwrap();
        let v = sample(dir.path(), &dir.path().join("missing"), Instant::now());
        assert!(v.download_transfers.is_none() && v.upload_transfers.is_none());
        assert!(v.database_error.is_some());
        assert!(v.downloads.available_bytes.is_none());
        assert!(v.database_bytes.is_none());
        assert!(
            !dir.path().join("rustysoul.sqlite3").exists(),
            "Sampling must never create a database"
        );
    }
    #[test]
    fn worker_coalesces_destination_updates_without_presenting_old_capacity_for_new_paths() {
        let dir = tempfile::tempdir().unwrap();
        let _db = crate::store::Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        let one = dir.path().join("one");
        let two = dir.path().join("two");
        std::fs::create_dir(&one).unwrap();
        std::fs::create_dir(&two).unwrap();
        let worker = Monitor::start(dir.path().into(), one, Instant::now()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while worker.latest().sampled_at == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        worker.set_downloads(two.clone());
        assert_eq!(worker.latest().downloads.path, two.to_string_lossy());
        while worker.latest().downloads.available_bytes.is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(worker.latest().process.threads.is_some());
    }
}
