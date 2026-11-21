//! Bounded, non-recursive housekeeping of empty private staging folders.
use super::SavedDownload;
use anyhow::{Context, Result};
use cap_std::{
    ambient_authority,
    fs::{Dir, MetadataExt, ReadDir},
};
use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct StagingCleanup {
    next_run: Option<Instant>,
    round: Option<Round>,
}
struct Round {
    roots: VecDeque<PathBuf>,
    scan: Option<Scan>,
    removed: usize,
}
struct Scan {
    directory: Dir,
    entries: ReadDir,
}
impl StagingCleanup {
    /// At most 32 root/entry operations per control tick, once per minute.
    pub fn poll<'a>(
        &mut self,
        current: &Path,
        rows: &[SavedDownload],
        active: impl Iterator<Item = &'a Dir>,
    ) -> Result<Option<usize>> {
        self.poll_budget(current, rows, active, 32)
    }
    fn poll_budget<'a>(
        &mut self,
        current: &Path,
        rows: &[SavedDownload],
        active: impl Iterator<Item = &'a Dir>,
        budget: usize,
    ) -> Result<Option<usize>> {
        if self.round.is_none() {
            if self.next_run.is_some_and(|at| Instant::now() < at) {
                return Ok(None);
            }
            let mut roots: BTreeSet<_> = rows
                .iter()
                .map(|r| PathBuf::from(&r.directory))
                .filter(|p| p.is_absolute())
                .collect();
            if current.is_absolute() {
                roots.insert(current.to_path_buf());
            }
            self.round = Some(Round {
                roots: roots.into_iter().collect(),
                scan: None,
                removed: 0,
            });
        }
        // A live descriptor is authoritative, including renamed directories.
        let protected: Result<HashSet<_>> = active
            .map(|d| {
                let metadata =
                    rustix::fs::fstat(d).context("Cannot inspect an active download folder")?;
                Ok((metadata.st_dev, metadata.st_ino))
            })
            .collect();
        let protected = match protected {
            Ok(p) => p,
            Err(e) => {
                self.round = None;
                self.next_run = Some(Instant::now() + Duration::from_secs(60));
                return Err(e);
            }
        };
        let round = self.round.as_mut().unwrap();
        for _ in 0..budget {
            if round.scan.is_none() {
                let Some(root) = round.roots.pop_front() else {
                    let removed = round.removed;
                    self.round = None;
                    self.next_run = Some(Instant::now() + Duration::from_secs(60));
                    return Ok(Some(removed));
                };
                round.scan = open_staging(&root).with_context(|| {
                    format!("Cannot inspect download staging in {}", root.display())
                })?;
                continue;
            }
            let scan = round.scan.as_mut().unwrap();
            let Some(entry) = scan.entries.next() else {
                round.scan = None;
                continue;
            };
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    round.scan = None;
                    return Err(error.into());
                }
            };
            let name = entry.file_name();
            // This namespace contains SHA256 transfer keys, not user folders.
            if !name.to_str().is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                continue;
            }
            let metadata = match scan.directory.symlink_metadata(&name) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            if !metadata.is_dir() || protected.contains(&(metadata.dev(), metadata.ino())) {
                continue;
            }
            // rmdir is atomic and never removes a non-empty directory. No
            // recursion or file deletion: even a zero-byte partial is retained.
            match scan.directory.remove_dir(&name) {
                Ok(()) => round.removed += 1,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound
                            | std::io::ErrorKind::NotADirectory
                            | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(None)
    }
}
fn open_staging(root: &Path) -> Result<Option<Scan>> {
    let root = match Dir::open_ambient_dir(root, ambient_authority()) {
        Ok(d) => d,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };
    let descriptor = match rustix::fs::openat(
        &root,
        ".incomplete",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(d) => d,
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP) => {
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };
    let directory = Dir::from_std_file(std::fs::File::from(descriptor));
    let entries = directory.entries()?;
    Ok(Some(Scan { directory, entries }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hash(n: usize) -> String {
        format!("{n:064x}")
    }
    fn sweep(
        cleaner: &mut StagingCleanup,
        root: &Path,
        rows: &[SavedDownload],
        active: &[Dir],
        budget: usize,
    ) -> usize {
        for _ in 0..1000 {
            if let Some(n) = cleaner
                .poll_budget(root, rows, active.iter(), budget)
                .unwrap()
            {
                return n;
            }
        }
        panic!("bounded cleanup did not complete its pass");
    }
    #[test]
    fn only_empty_transfer_folders_are_removed_and_active_or_resumable_data_survives() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join(".incomplete");
        std::fs::create_dir(&staging).unwrap();
        for n in 0..6 {
            std::fs::create_dir(staging.join(hash(n))).unwrap();
        }
        std::fs::write(staging.join(hash(1)).join("song.part"), b"resumable").unwrap();
        std::fs::write(staging.join(hash(2)).join("empty.part"), b"").unwrap();
        std::fs::create_dir(staging.join(hash(3)).join("nested")).unwrap();
        std::fs::create_dir(staging.join("notes")).unwrap();
        let active =
            vec![Dir::open_ambient_dir(staging.join(hash(4)), ambient_authority()).unwrap()];
        let mut cleanup = StagingCleanup::default();
        assert_eq!(sweep(&mut cleanup, tmp.path(), &[], &active, 2), 2);
        assert_eq!(
            std::fs::read(staging.join(hash(1)).join("song.part")).unwrap(),
            b"resumable"
        );
        assert!(staging.join(hash(2)).join("empty.part").exists());
        assert!(staging.join(hash(3)).exists());
        assert!(staging.join(hash(4)).exists());
        assert!(staging.join("notes").exists());
        drop(active);
        cleanup.next_run = None;
        assert_eq!(sweep(&mut cleanup, tmp.path(), &[], &[], 2), 1);
        assert!(staging.exists());
    }
    #[test]
    fn an_incremental_pass_reaches_empty_folders_beyond_nonempty_ones_and_is_throttled() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join(".incomplete");
        std::fs::create_dir(&staging).unwrap();
        for n in 0..80 {
            std::fs::create_dir(staging.join(hash(n))).unwrap();
            if n < 40 {
                std::fs::write(staging.join(hash(n)).join("keep.part"), b"keep").unwrap();
            }
        }
        let mut cleanup = StagingCleanup::default();
        assert!(
            cleanup
                .poll_budget(tmp.path(), &[], std::iter::empty(), 4)
                .unwrap()
                .is_none()
        );
        assert!(std::fs::read_dir(&staging).unwrap().count() >= 77);
        assert_eq!(sweep(&mut cleanup, tmp.path(), &[], &[], 4), 40);
        std::fs::create_dir(staging.join(hash(99))).unwrap();
        assert!(
            cleanup
                .poll(tmp.path(), &[], std::iter::empty())
                .unwrap()
                .is_none()
        );
        assert!(staging.join(hash(99)).exists());
        cleanup.next_run = None;
        assert_eq!(sweep(&mut cleanup, tmp.path(), &[], &[], 4), 1);
    }
    #[test]
    fn staging_and_leaf_symlinks_cannot_redirect_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(outside.path().join(hash(0))).unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join(".incomplete")).unwrap();
        let mut cleanup = StagingCleanup::default();
        assert_eq!(sweep(&mut cleanup, tmp.path(), &[], &[], 4), 0);
        assert!(outside.path().join(hash(0)).exists());
        std::fs::remove_file(tmp.path().join(".incomplete")).unwrap();
        std::fs::create_dir(tmp.path().join(".incomplete")).unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join(".incomplete").join(hash(1)))
            .unwrap();
        cleanup.next_run = None;
        assert_eq!(sweep(&mut cleanup, tmp.path(), &[], &[], 4), 0);
        assert!(outside.path().join(hash(0)).exists());
        assert!(tmp.path().join(".incomplete").join(hash(1)).is_symlink());
    }
    #[test]
    fn a_file_arriving_during_a_pass_prevents_directory_removal() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join(".incomplete").join(hash(0));
        std::fs::create_dir_all(&folder).unwrap();
        let mut cleanup = StagingCleanup::default();
        assert!(
            cleanup
                .poll_budget(tmp.path(), &[], std::iter::empty(), 1)
                .unwrap()
                .is_none()
        );
        std::fs::write(folder.join("late.part"), b"new data").unwrap();
        assert_eq!(sweep(&mut cleanup, tmp.path(), &[], &[], 1), 0);
        assert_eq!(
            std::fs::read(folder.join("late.part")).unwrap(),
            b"new data"
        );
    }
    #[test]
    fn previous_download_locations_are_reconciled_without_recreating_missing_roots() {
        let current = tempfile::tempdir().unwrap();
        let previous = tempfile::tempdir().unwrap();
        let folder = previous.path().join(".incomplete").join(hash(0));
        std::fs::create_dir_all(&folder).unwrap();
        let row = SavedDownload {
            username: "peer".into(),
            path: "song.flac".into(),
            size: 1,
            status: "completed".into(),
            directory: previous.path().to_string_lossy().into_owned(),
            local_path: String::new(),
        };
        let mut cleanup = StagingCleanup::default();
        assert_eq!(
            sweep(
                &mut cleanup,
                &current.path().join("missing"),
                &[row],
                &[],
                2
            ),
            1
        );
        assert!(!current.path().join("missing").exists());
        assert!(!folder.exists());
    }
}
