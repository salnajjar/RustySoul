//! Persistent incremental metadata index. Filesystem hooks are backed by periodic reconciliation.
use crate::model::{FileLocations, LibraryStatus};
use anyhow::{Context, Result, ensure};
use notify::{EventKind, RecursiveMode, Watcher};
use rusqlite::{Connection, params};
use soulseek_rs::shares::{SharedFile, Shares};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};
use tokio::sync::watch;

#[derive(Clone)]
pub struct Update {
    pub generation: u64,
    pub index: Option<Arc<Shares>>,
    pub status: LibraryStatus,
    pub error: Option<String>,
}
pub struct Library {
    pub updates: watch::Receiver<Option<Update>>,
    requests: watch::Sender<(u64, Vec<PathBuf>)>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
#[derive(Clone)]
struct Cached {
    virtual_path: String,
    size: u64,
    modified: String,
    changed: String,
    attributes: Vec<(u32, u32)>,
}
fn load(db: &Connection) -> Result<BTreeMap<String, Cached>> {
    let mut stmt = db.prepare(
        "SELECT real_path,virtual_path,size,modified,changed,attributes FROM library_cache",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut cache = BTreeMap::new();
    for row in rows {
        let (real, virtual_path, size, modified, changed, attrs) = row?;
        cache.insert(
            real,
            Cached {
                virtual_path,
                size: u64::try_from(size)?,
                modified,
                changed,
                attributes: serde_json::from_str(&attrs)?,
            },
        );
    }
    Ok(cache)
}
fn snapshot(cache: &BTreeMap<String, Cached>) -> Arc<Shares> {
    let folders: HashSet<_> = cache
        .values()
        .filter_map(|c| c.virtual_path.rsplit_once('\\').map(|v| v.0))
        .collect();
    Arc::new(Shares::from_files(
        cache
            .iter()
            .map(|(real, c)| {
                SharedFile::from_cached(
                    c.virtual_path.clone(),
                    real.into(),
                    c.size,
                    c.attributes.clone(),
                )
            })
            .collect(),
        folders.len(),
    ))
}
pub fn cached_index(db: &Connection, roots: &[PathBuf]) -> Result<Arc<Shares>> {
    let mut cache = load(db)?;
    cache.retain(|real, _| roots.iter().any(|root| Path::new(real).starts_with(root)));
    Ok(snapshot(&cache))
}
pub fn validate_locations(locations: &FileLocations, data_dir: &Path) -> Result<FileLocations> {
    ensure!(
        Path::new(&locations.downloads).is_absolute(),
        "Download folder must be an absolute container/server path"
    );
    let download = std::fs::canonicalize(&locations.downloads)
        .context("Download folder must already exist")?;
    ensure!(download.is_dir(), "Download path must be a directory");
    let probe = download.join(format!(".rustysoul-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .context("Download folder is not writable by the daemon")?;
    std::fs::remove_file(probe)?;
    let private = std::fs::canonicalize(data_dir)?;
    let mut roots = vec![];
    for value in &locations.shares {
        ensure!(
            Path::new(value).is_absolute(),
            "Share folders must be absolute container/server paths"
        );
        let root = std::fs::canonicalize(value).context("Share folder must exist")?;
        ensure!(
            root.is_dir() && root.parent().is_some(),
            "Cannot share the filesystem root"
        );
        ensure!(
            !private.starts_with(&root)
                && (!root.starts_with(&private) || root.starts_with(private.join("downloads"))),
            "Do not share the daemon's private data or credential files"
        );
        let _ = std::fs::read_dir(&root).context("Share folder is not readable")?;
        ensure!(
            !roots
                .iter()
                .any(|p: &PathBuf| root.starts_with(p) || p.starts_with(&root)),
            "Share roots must not duplicate or overlap one another"
        );
        roots.push(root);
    }
    Ok(FileLocations {
        downloads: download.to_string_lossy().into_owned(),
        shares: roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    })
}
pub fn check_location_roots(locations: &FileLocations, roots: &[PathBuf]) -> Result<()> {
    if roots.is_empty() {
        return Ok(());
    }
    let roots = roots
        .iter()
        .map(std::fs::canonicalize)
        .collect::<std::io::Result<Vec<_>>>()?;
    for path in std::iter::once(&locations.downloads).chain(locations.shares.iter()) {
        let path = std::fs::canonicalize(path)?;
        ensure!(
            roots.iter().any(|r| path.starts_with(r)),
            "Select a folder inside the storage mounts configured at installation"
        );
    }
    Ok(())
}
fn scan(
    db: &mut Connection,
    roots: &[PathBuf],
    stop: &AtomicBool,
    mut report: impl FnMut(&LibraryStatus),
) -> Result<(Arc<Shares>, LibraryStatus)> {
    let start = Instant::now();
    let old = load(db)?;
    let mut next = BTreeMap::new();
    let mut stats = LibraryStatus {
        scanning: true,
        phase: "scanning".into(),
        indexed: old.len(),
        ..Default::default()
    };
    report(&stats);
    let mut reported = Instant::now();
    let mut names: HashMap<String, usize> = HashMap::new();
    for root in roots {
        let base = root
            .file_name()
            .context("Invalid share root")?
            .to_string_lossy()
            .into_owned();
        let count = names.entry(base.clone()).or_default();
        *count += 1;
        let name = if *count == 1 {
            base
        } else {
            format!("{base} ({count})")
        };
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            stats.folders += 1;
            ensure!(!stop.load(Ordering::Relaxed), "Library scan stopped");
            for entry in std::fs::read_dir(&dir)
                .with_context(|| format!("Cannot read share directory {}", dir.display()))?
            {
                ensure!(!stop.load(Ordering::Relaxed), "Library scan stopped");
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    continue;
                }
                let path = entry.path();
                if kind.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                let real = path.to_string_lossy().into_owned();
                if real.ends_with(".part") || real.ends_with(".slsk-part") {
                    continue;
                }
                let meta = match std::fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                if meta.file_type().is_symlink() {
                    continue;
                }
                let virtual_path = format!(
                    "{}\\{}",
                    name,
                    path.strip_prefix(root)?
                        .components()
                        .map(|p| p.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("\\")
                );
                let modified = meta
                    .modified()?
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_string();
                #[cfg(unix)]
                let changed = {
                    use std::os::unix::fs::MetadataExt;
                    format!(
                        "{}:{}:{}:{}",
                        meta.ctime(),
                        meta.ctime_nsec(),
                        meta.ino(),
                        meta.dev()
                    )
                };
                #[cfg(not(unix))]
                let changed = meta
                    .created()
                    .unwrap_or(UNIX_EPOCH)
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_string();
                // Reprobe an older metadata extractor once, then retain the fast
                // unchanged-file path on all subsequent incremental scans.
                let extractor = if path
                    .extension()
                    .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("mp3"))
                {
                    "audio-mp3-v3"
                } else {
                    "audio-v2"
                };
                let changed = format!("{extractor}:{changed}");
                let attrs = if let Some(c) = old.get(&real).filter(|c| {
                    c.size == meta.len()
                        && c.modified == modified
                        && c.changed == changed
                        && c.virtual_path == virtual_path
                }) {
                    stats.skipped += 1;
                    c.attributes.clone()
                } else {
                    stats.changed += 1;
                    SharedFile::probe(&path)
                };
                next.insert(
                    real,
                    Cached {
                        virtual_path,
                        size: meta.len(),
                        modified,
                        changed,
                        attributes: attrs,
                    },
                );
                stats.processed = next.len();
                stats.bytes = stats.bytes.saturating_add(meta.len());
                if reported.elapsed() >= Duration::from_millis(250) {
                    stats.elapsed_ms = start.elapsed().as_millis() as u64;
                    report(&stats);
                    reported = Instant::now();
                }
            }
        }
    }
    stats.removed = old.keys().filter(|p| !next.contains_key(*p)).count();
    stats.phase = "saving".into();
    stats.elapsed_ms = start.elapsed().as_millis() as u64;
    report(&stats);
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    // Remove superseded virtual paths first, including root-name swaps and renames.
    for (real, previous) in &old {
        if !next.get(real).is_some_and(|n| {
            n.virtual_path == previous.virtual_path
                && n.size == previous.size
                && n.modified == previous.modified
                && n.changed == previous.changed
        }) {
            tx.execute(
                "DELETE FROM local_files WHERE path=?1",
                [&previous.virtual_path],
            )?;
        }
        if !next.contains_key(real) {
            tx.execute("DELETE FROM library_cache WHERE real_path=?1", [real])?;
        }
    }
    {
        let mut insert=tx.prepare_cached("INSERT INTO library_cache VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(real_path) DO UPDATE SET virtual_path=excluded.virtual_path,size=excluded.size,modified=excluded.modified,changed=excluded.changed,attributes=excluded.attributes")?;
        let mut inventory=tx.prepare_cached("INSERT INTO local_files VALUES(?1,?2,?3) ON CONFLICT(path) DO UPDATE SET size=excluded.size,attributes=excluded.attributes")?;
        for (real, c) in &next {
            ensure!(!stop.load(Ordering::Relaxed), "Library scan stopped");
            if old.get(real).is_some_and(|o| {
                o.size == c.size
                    && o.modified == c.modified
                    && o.changed == c.changed
                    && o.virtual_path == c.virtual_path
            }) {
                continue;
            }
            let attrs = serde_json::to_string(&c.attributes)?;
            insert.execute(params![
                real,
                c.virtual_path,
                i64::try_from(c.size)?,
                c.modified,
                c.changed,
                attrs
            ])?;
            inventory.execute(params![c.virtual_path, i64::try_from(c.size)?, attrs])?;
        }
        // Remove old v2 inventory rows without a corresponding cached file.
        tx.execute(
            "DELETE FROM local_files WHERE path NOT IN (SELECT virtual_path FROM library_cache)",
            [],
        )?;
    }
    tx.commit()?;
    stats.phase = "building".into();
    report(&stats);
    let index = snapshot(&next);
    stats.indexed = next.len();
    stats.scanning = false;
    stats.phase = "idle".into();
    stats.elapsed_ms = start.elapsed().as_millis() as u64;
    Ok((index, stats))
}
impl Library {
    pub fn start(path: PathBuf, roots: Vec<PathBuf>, index_on_startup: bool) -> Result<Self> {
        Self::start_with_interval(path, roots, index_on_startup, Duration::from_secs(60))
    }
    fn start_with_interval(
        path: PathBuf,
        roots: Vec<PathBuf>,
        index_on_startup: bool,
        interval: Duration,
    ) -> Result<Self> {
        let (tx, updates) = watch::channel(None);
        let (requests, mut rx) = watch::channel((0, roots.clone()));
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread=std::thread::Builder::new().name("library-index".into()).spawn(move||{
            let run=||->Result<()>{
                let mut db=Connection::open(path)?;db.busy_timeout(Duration::from_secs(5))?;
                let dirty=Arc::new(AtomicBool::new(false));let hook=dirty.clone();
                let mut watcher=notify::recommended_watcher(move|event:notify::Result<notify::Event>|{if event.is_err()||event.is_ok_and(|e|!matches!(e.kind,EventKind::Access(_))){hook.store(true,Ordering::Relaxed);}}).ok();
                let mut roots=roots;let mut generation=0;let mut revision=0;let mut latest=LibraryStatus{phase:"idle".into(),indexed:cached_index(&db,&roots)?.file_count() as usize,..Default::default()};let mut pending=index_on_startup.then(||(Instant::now(),"startup"));let mut last=Instant::now();let mut watched:Option<Vec<PathBuf>>=None;
                let mut mode="Filesystem events + 60-second reconciliation".to_string();
                loop{
                    if stopping.load(Ordering::Relaxed){break;}
                    if rx.has_changed().unwrap_or(false){let(g,new)=rx.borrow_and_update().clone();generation=g;let trigger=if roots==new {"manual"}else{"locations"};roots=new;pending=Some((Instant::now(),trigger));}
                    if dirty.swap(false,Ordering::Relaxed) && pending.is_none(){pending=Some((Instant::now()+Duration::from_secs(1),"filesystem"));}
                    if last.elapsed()>=interval&&pending.is_none(){pending=Some((Instant::now(),"scheduled"));}
                    if watched.as_ref()!=Some(&roots) {
mode="Filesystem events + 60-second reconciliation".to_string();if let Some(w)=watcher.as_mut(){if let Some(old)=&watched{for p in old{let _=w.unwatch(p);}}for p in &roots{if w.watch(p,RecursiveMode::Recursive).is_err(){mode="Periodic reconciliation (some filesystem hooks unavailable)".into();}}}else{mode="Periodic reconciliation (filesystem hooks unavailable)".into();}watched=Some(roots.clone());
latest.watcher=mode.clone();tx.send_replace(Some(Update{generation,index:None,status:latest.clone(),error:None}));
                    }
                    if pending.is_some_and(|(at,_)|at<=Instant::now()){
                        let trigger=pending.take().unwrap().1;

                        let scan_id=std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                        latest=LibraryStatus{scanning:true,phase:"preparing".into(),scan_id,trigger:trigger.into(),indexed:latest.indexed,watcher:mode.clone(),revision,..Default::default()};
                        tx.send_replace(Some(Update{generation,index:None,status:latest.clone(),error:None}));
                        let result=scan(&mut db,&roots,&stopping,|progress|{
                            latest=progress.clone();latest.trigger=trigger.into();latest.scan_id=scan_id;latest.revision=revision;latest.watcher=mode.clone();
                            tx.send_replace(Some(Update{generation,index:None,status:latest.clone(),error:None}));
                        });
                        match result {
                            Ok((index,mut status))=>{status.trigger=trigger.into();revision+=1;status.scan_id=scan_id;status.revision=revision;status.watcher=mode.clone();latest=status.clone();tx.send_replace(Some(Update{generation,index:Some(index),status,error:None}));},
                            Err(e)=>{latest.scanning=false;latest.phase="failed".into();tx.send_replace(Some(Update{generation,index:None,status:latest.clone(),error:Some(e.to_string())}));}
                        }
                        last=Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }Ok(())
            };if let Err(e)=run(){tx.send_replace(Some(Update{generation:0,index:None,status:LibraryStatus::default(),error:Some(e.to_string())}));}
        })?;
        Ok(Self {
            updates,
            requests,
            stop,
            thread: Some(thread),
        })
    }
    pub fn request(&self, generation: u64, roots: Vec<PathBuf>) -> Result<()> {
        ensure!(!self.requests.is_closed(), "Library worker is unavailable");
        self.requests.send_replace((generation, roots));
        Ok(())
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn periodic_and_requested_scans_have_distinct_triggers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.flac"), b"fLaC").unwrap();
        let db = dir.path().join("db");
        let _store = crate::store::Store::open(&db).unwrap();
        let library =
            Library::start_with_interval(db, vec![root.clone()], false, Duration::from_millis(400))
                .unwrap();
        let end = Instant::now() + Duration::from_secs(4);
        while !library
            .updates
            .borrow()
            .as_ref()
            .is_some_and(|u| u.index.is_some() && u.status.trigger == "scheduled")
        {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        library.request(1, vec![root]).unwrap();
        let end = Instant::now() + Duration::from_secs(4);
        while !library
            .updates
            .borrow()
            .as_ref()
            .is_some_and(|u| u.generation == 1 && u.index.is_some() && u.status.trigger == "manual")
        {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn startup_index_can_be_skipped_without_disabling_filesystem_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("old.flac"), b"old").unwrap();
        let db = dir.path().join("db");
        let _store = crate::store::Store::open(&db).unwrap();
        let library = Library::start(db, vec![root.clone()], false).unwrap();
        let end = Instant::now() + Duration::from_secs(3);
        while library.updates.borrow().is_none() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        {
            let update = library.updates.borrow();
            let update = update.as_ref().unwrap();
            assert!(!update.status.scanning);
            assert_eq!(update.status.scan_id, 0);
            assert_eq!(update.status.indexed, 0);
            assert!(!update.status.watcher.is_empty());
        }
        std::fs::write(root.join("new.flac"), b"new").unwrap();
        let end = Instant::now() + Duration::from_secs(8);
        loop {
            if library
                .updates
                .borrow()
                .as_ref()
                .is_some_and(|u| u.index.is_some() && u.status.indexed == 2)
            {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    #[test]
    fn scan_reports_live_totals_before_committing_a_new_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        std::fs::create_dir(&root).unwrap();
        for i in 0..8 {
            std::fs::write(root.join(format!("{i}.flac")), b"fLaC").unwrap();
        }
        let path = dir.path().join("db");
        let mut store = crate::store::Store::open(&path).unwrap();
        let reader = Connection::open(&path).unwrap();
        let mut progress = vec![];
        let (index, status) = scan(
            &mut store.connection,
            &[root],
            &AtomicBool::new(false),
            |s| {
                if s.phase == "saving" {
                    assert_eq!(
                        reader
                            .query_row("SELECT COUNT(*) FROM library_cache", [], |r| r
                                .get::<_, u32>(0))
                            .unwrap(),
                        0
                    );
                }
                progress.push(s.clone());
            },
        )
        .unwrap();
        assert_eq!(
            progress
                .iter()
                .map(|s| s.phase.as_str())
                .collect::<Vec<_>>(),
            ["scanning", "saving", "building"]
        );
        assert_eq!(
            (
                progress[1].processed,
                progress[1].folders,
                progress[1].bytes
            ),
            (8, 1, 32)
        );
        assert!(progress.iter().all(|s| s.scanning));
        assert!(!status.scanning);
        assert_eq!(status.phase, "idle");
        assert_eq!(status.indexed, 8);
        assert_eq!(index.file_count(), 8);
    }
    #[test]
    fn cancelled_scan_keeps_the_previous_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.flac"), b"first").unwrap();
        let mut store = crate::store::Store::open(&dir.path().join("db")).unwrap();
        let stop = AtomicBool::new(false);
        scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        std::fs::write(root.join("b.flac"), b"second").unwrap();
        assert!(
            scan(&mut store.connection, &[root], &stop, |s| {
                if s.phase == "saving" {
                    stop.store(true, Ordering::Relaxed);
                }
            })
            .is_err()
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM library_cache", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn installed_mounts_restrict_settings_to_their_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("downloads");
        let library = dir.path().join("library");
        let other = dir.path().join("other");
        for path in [&downloads, &library, &other] {
            std::fs::create_dir(path).unwrap();
        }
        let roots = vec![downloads.clone(), library.clone()];
        let mut locations = FileLocations {
            downloads: downloads.to_string_lossy().into_owned(),
            shares: vec![library.to_string_lossy().into_owned()],
        };
        check_location_roots(&locations, &roots).unwrap();
        locations.shares = vec![other.to_string_lossy().into_owned()];
        assert!(check_location_roots(&locations, &roots).is_err());
    }
    #[test]
    fn old_cached_properties_are_refreshed_once_without_reopening_unchanged_media_later() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        std::fs::create_dir(&root).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("vendor/soulseek-rs-lib/tests/fixtures/audio/tone.wav"),
            root.join("tone.wav"),
        )
        .unwrap();
        let mut store = crate::store::Store::open(&dir.path().join("db")).unwrap();
        let stop = AtomicBool::new(false);
        scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        store.connection.execute_batch("UPDATE library_cache SET changed=substr(changed,10),attributes='[]'; UPDATE local_files SET attributes='[]';").unwrap();
        let (_, refreshed) = scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        assert_eq!((refreshed.changed, refreshed.skipped), (1, 0));
        let attrs: String = store
            .connection
            .query_row("SELECT attributes FROM local_files", [], |r| r.get(0))
            .unwrap();
        let attrs: Vec<(u32, u32)> = serde_json::from_str(&attrs).unwrap();
        assert!(attrs.contains(&(4, 48000)));
        assert!(attrs.contains(&(5, 24)));
        let (_, unchanged) = scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        assert_eq!((unchanged.changed, unchanged.skipped), (0, 1));
    }
    #[test]
    fn incremental_scan_skips_unchanged_and_tracks_add_modify_remove() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.flac"), b"one").unwrap();
        let mut store = crate::store::Store::open(&dir.path().join("db")).unwrap();
        let stop = AtomicBool::new(false);
        let (_, first) = scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        assert_eq!((first.changed, first.skipped), (1, 0));
        let (_, second) = scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        assert_eq!((second.changed, second.skipped), (0, 1));
        std::fs::write(root.join("a.flac"), b"modified").unwrap();
        std::fs::write(root.join("b.mp3"), b"new").unwrap();
        let (_, third) = scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        assert_eq!((third.changed, third.indexed), (2, 2));
        std::fs::remove_file(root.join("a.flac")).unwrap();
        let (index, last) = scan(&mut store.connection, &[root], &stop, |_| {}).unwrap();
        assert_eq!((last.skipped, last.removed, index.file_count()), (1, 1, 1));
    }

    #[test]
    fn mp3_metadata_upgrade_corrects_cbr_and_keeps_other_formats_cached() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        std::fs::create_dir(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor/soulseek-rs-lib/tests/fixtures/audio");
        for name in ["tone.flac", "tone.mp3"] {
            std::fs::copy(fixtures.join(name), root.join(name)).unwrap();
        }
        let mut store = crate::store::Store::open(&dir.path().join("db")).unwrap();
        let stop = AtomicBool::new(false);
        scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        store.connection.execute_batch("UPDATE library_cache SET changed=replace(changed,'audio-mp3-v3:','audio-v2:'),attributes='[[0,195],[1,1],[2,0],[4,48000]]' WHERE real_path LIKE '%.mp3'; UPDATE local_files SET attributes='[[0,195],[1,1],[2,0],[4,48000]]' WHERE path LIKE '%.mp3';").unwrap();
        let (index, status) = scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        assert_eq!((status.changed, status.skipped), (1, 1));
        let mp3 = index
            .files()
            .iter()
            .find(|f| f.virtual_path.ends_with("tone.mp3"))
            .unwrap();
        assert!(mp3.attributes.contains(&(0, 192)));
        let attrs: String = store
            .connection
            .query_row(
                "SELECT attributes FROM local_files WHERE path LIKE '%.mp3'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            serde_json::from_str::<Vec<(u32, u32)>>(&attrs)
                .unwrap()
                .contains(&(0, 192))
        );
        let (_, unchanged) = scan(
            &mut store.connection,
            std::slice::from_ref(&root),
            &stop,
            |_| {},
        )
        .unwrap();
        assert_eq!((unchanged.changed, unchanged.skipped), (0, 2));
    }
    #[test]
    fn filesystem_hooks_automatically_detect_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("watched");
        std::fs::create_dir(&root).unwrap();
        let db = dir.path().join("db");
        let _store = crate::store::Store::open(&db).unwrap();
        let library = Library::start(db, vec![root.clone()], true).unwrap();
        let wait = |count, revision| {
            let end = Instant::now() + Duration::from_secs(8);
            loop {
                if library.updates.borrow().as_ref().is_some_and(|u| {
                    u.index.is_some() && u.status.indexed == count && u.status.revision >= revision
                }) {
                    break;
                }
                assert!(Instant::now() < end, "Filesystem update timed out");
                std::thread::sleep(Duration::from_millis(25));
            }
        };
        wait(0, 1);
        std::fs::write(root.join("new.flac"), b"new file").unwrap();
        wait(1, 2);
        library.request(0, vec![root]).unwrap();
        wait(1, 3);
        assert_eq!(library.updates.borrow().as_ref().unwrap().status.skipped, 1);
    }
}
