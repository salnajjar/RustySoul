//! Private staging, readable completed paths, and capability-scoped file access.
use crate::store::SavedDownload;
use anyhow::{Context, Result, ensure};
use cap_std::{ambient_authority, fs::Dir};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod cleanup;
#[cfg(target_os = "linux")]
pub use cleanup::StagingCleanup;

pub fn key(user: &str, path: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{user}\0{path}").as_bytes()))
}
pub fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}
pub fn legacy_relative(row: &SavedDownload) -> PathBuf {
    PathBuf::from(key(&row.username, &row.path)).join(basename(&row.path))
}
pub fn staging_relative(row: &SavedDownload) -> PathBuf {
    PathBuf::from(".incomplete")
        .join(key(&row.username, &row.path))
        .join(basename(&row.path))
}
fn user_folder(user: &str) -> String {
    if let Some(rest) = user.strip_prefix('.') {
        return format!("%2E{}", user_folder(rest));
    }
    user.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b" _-.()[]".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
pub fn readable_relative(row: &SavedDownload) -> Result<PathBuf> {
    crate::validation::username(&row.username).map_err(anyhow::Error::msg)?;
    crate::validation::remote_path(&row.path).map_err(anyhow::Error::msg)?;
    let mut result = PathBuf::from(user_folder(&row.username));
    for part in row.path.split(['\\', '/']) {
        ensure!(
            !part.is_empty() && part.len() <= 255,
            "A remote folder or filename is too long or empty"
        );
        result.push(part);
    }
    Ok(result)
}
pub fn root(row: &SavedDownload) -> Result<Dir> {
    ensure!(
        !row.directory.is_empty(),
        "Download destination was not recorded"
    );
    Dir::open_ambient_dir(&row.directory, ambient_authority())
        .context("Download folder is unavailable")
}
pub fn work_directory(row: &SavedDownload) -> Result<Dir> {
    let dir = root(row)?;
    let legacy = legacy_relative(row);
    let relative = if dir
        .try_exists(legacy.with_file_name(format!("{}.part", basename(&row.path))))?
        || dir.try_exists(&legacy)?
    {
        legacy
    } else {
        staging_relative(row)
    };
    let parent = relative.parent().unwrap();
    dir.create_dir_all(parent)
        .with_context(|| format!("Cannot create download staging folder {}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        dir.set_permissions(
            parent,
            cap_std::fs::Permissions::from_std(std::fs::Permissions::from_mode(0o700)),
        )
        .with_context(|| format!("Cannot secure download staging folder {}", parent.display()))?;
    }
    #[cfg(target_os = "linux")]
    prepare_partial(&dir, row, &relative)?;
    dir.open_dir(parent)
        .with_context(|| format!("Cannot open download staging folder {}", parent.display()))
}
#[cfg(target_os = "linux")]
fn access_acl(file: &std::fs::File) -> Result<Option<Vec<u8>>> {
    let mut bytes = vec![0; 65536];
    match rustix::fs::fgetxattr(file, "system.posix_acl_access", bytes.as_mut_slice()) {
        Ok(count) => {
            bytes.truncate(count);
            Ok(Some(bytes))
        }
        Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// An empty inode created in the destination gets the kernel's normal ACL,
/// umask and setgid inheritance. Keep it private before placing any data in it.
#[cfg(target_os = "linux")]
struct DestinationFile {
    directory: Dir,
    name: PathBuf,
    file: std::fs::File,
    permissions: std::fs::Permissions,
    acl: Option<Vec<u8>>,
}
#[cfg(target_os = "linux")]
impl DestinationFile {
    fn create(root: &Dir, parent: &std::path::Path) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        root.create_dir_all(parent)?;
        let directory = root.open_dir(parent)?;
        let name = PathBuf::from(format!(".rustysoul-complete-{}.part", uuid::Uuid::new_v4()));
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        let file = directory.open_with(&name, &options)?.into_std();
        let permissions = file.metadata()?.permissions();
        let mut result = Self {
            directory,
            name,
            file,
            permissions,
            acl: None,
        };
        result.acl = access_acl(&result.file)?;
        result
            .file
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(result)
    }
    fn link_into(&self, directory: &Dir, name: &std::path::Path) -> Result<()> {
        use std::os::fd::AsRawFd;
        ensure!(
            name.components().count() == 1
                && matches!(
                    name.components().next(),
                    Some(std::path::Component::Normal(_))
                ),
            "Invalid private staging filename"
        );
        // Linking the open inode avoids trusting a temporary pathname in a
        // group-writable destination. /proc is already required by transfers.
        rustix::fs::linkat(
            rustix::fs::CWD,
            format!("/proc/self/fd/{}", self.file.as_raw_fd()),
            directory,
            name,
            rustix::fs::AtFlags::SYMLINK_FOLLOW,
        )?;
        Ok(())
    }
    fn remove_named(&self) {
        use cap_std::fs::MetadataExt as _;
        use std::os::unix::fs::MetadataExt as _;
        if let (Ok(expected), Ok(current)) = (
            self.file.metadata(),
            self.directory.symlink_metadata(&self.name),
        ) && current.is_file()
            && expected.dev() == current.dev()
            && expected.ino() == current.ino()
        {
            let _ = self.directory.remove_file(&self.name);
        }
    }
    fn move_to_private(&mut self, root: &Dir, parent: &std::path::Path) -> Result<()> {
        let directory = root.open_dir(parent)?;
        let name = PathBuf::from(format!(".rustysoul-complete-{}.part", uuid::Uuid::new_v4()));
        self.link_into(&directory, &name)?;
        self.remove_named();
        self.directory = directory;
        self.name = name;
        Ok(())
    }
    fn apply(&self, file: &std::fs::File) -> Result<()> {
        if let Some(acl) = &self.acl {
            rustix::fs::fsetxattr(
                file,
                "system.posix_acl_access",
                acl,
                rustix::fs::XattrFlags::empty(),
            )?;
        } else {
            match rustix::fs::fremovexattr(file, "system.posix_acl_access") {
                Ok(()) | Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => (),
                Err(error) => return Err(error.into()),
            }
        }
        file.set_permissions(self.permissions.clone())?;
        Ok(())
    }
}
#[cfg(target_os = "linux")]
impl Drop for DestinationFile {
    fn drop(&mut self) {
        self.remove_named();
    }
}

/// Allocate the empty partial in its eventual parent first, retaining the
/// inherited group while the file remains private in staging during transfer.
#[cfg(target_os = "linux")]
fn prepare_partial(root: &Dir, row: &SavedDownload, relative: &std::path::Path) -> Result<()> {
    let part = relative.with_file_name(format!("{}.part", basename(&row.path)));
    if root.try_exists(&part)? {
        return Ok(());
    }
    let target = readable_relative(row)?;
    let empty = DestinationFile::create(root, target.parent().unwrap())?;
    let directory = root.open_dir(part.parent().unwrap())?;
    match empty.link_into(&directory, std::path::Path::new(part.file_name().unwrap())) {
        Ok(()) => Ok(()),
        Err(error)
            if error.downcast_ref::<rustix::io::Errno>() == Some(&rustix::io::Errno::EXIST) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn inherit_completed(root: &Dir, source: &std::path::Path, parent: &std::path::Path) -> Result<()> {
    use cap_std::fs::MetadataExt as _;
    use std::io::Read as _;
    use std::os::unix::fs::MetadataExt as _;
    let expected = root.symlink_metadata(source)?;
    let mut original = root.open(source)?.into_std();
    let metadata = original.metadata()?;
    ensure!(
        expected.is_file() && expected.dev() == metadata.dev() && expected.ino() == metadata.ino(),
        "Download changed before finalization"
    );
    let mut destination = DestinationFile::create(root, parent)?;
    let inherited = destination.file.metadata()?;
    if metadata.uid() == inherited.uid()
        && metadata.gid() == inherited.gid()
        && metadata.nlink() == 1
    {
        // Normal downloads already have the correct group: metadata only.
        destination.apply(&original)?;
        return Ok(());
    }
    // Keep any payload copy inside the existing private transfer directory.
    destination.move_to_private(root, source.parent().unwrap())?;
    // Existing partials or a changed destination group may require a new
    // inode. Reflink shares extents when supported; fallback copies are bounded.
    match rustix::fs::ioctl_ficlone(&destination.file, &original) {
        Ok(()) => (),
        Err(
            rustix::io::Errno::OPNOTSUPP
            | rustix::io::Errno::NOTTY
            | rustix::io::Errno::INVAL
            | rustix::io::Errno::XDEV,
        ) => {
            let copied = std::io::copy(
                &mut (&mut original).take(metadata.len()),
                &mut destination.file,
            )?;
            ensure!(
                copied == metadata.len(),
                "Download changed during finalization"
            );
        }
        Err(error) => return Err(error.into()),
    }
    ensure!(
        destination.file.metadata()?.len() == metadata.len(),
        "Incomplete finalized download"
    );
    destination.file.set_modified(metadata.modified()?)?;
    destination.apply(&destination.file)?;
    destination.file.sync_all()?;
    let current = root.symlink_metadata(source)?;
    ensure!(
        current.dev() == metadata.dev()
            && current.ino() == metadata.ino()
            && current.len() == metadata.len(),
        "Download changed during finalization"
    );
    // Keep the existing hard-link/crash-recovery protocol: replace the private
    // source atomically before publishing its inode at a collision-free name.
    destination
        .directory
        .rename(&destination.name, root, source)?;
    Ok(())
}

pub fn completed_relative(row: &SavedDownload) -> Result<PathBuf> {
    if !row.local_path.is_empty() {
        let p = PathBuf::from(&row.local_path);
        ensure!(
            !p.is_absolute()
                && p.components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Invalid recorded download path"
        );
        Ok(p)
    } else {
        Ok(legacy_relative(row))
    }
}
pub fn open_completed(row: &SavedDownload) -> Result<std::fs::File> {
    ensure!(
        row.status == "completed",
        "Only completed downloads can be played"
    );
    let dir = root(row)?;
    let path = completed_relative(row)?;
    ensure!(
        dir.symlink_metadata(&path)?.is_file(),
        "The completed download is not a regular file"
    );
    let file = dir.open(&path)?.into_std();
    ensure!(
        file.metadata()?.len() == row.size,
        "The completed file has changed size"
    );
    Ok(file)
}
fn same_file(a: &cap_std::fs::Metadata, b: &cap_std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}
/// Link without overwriting, record the chosen destination before removing the source.
/// A crash after linking is recoverable by inode comparison on the next attempt.
pub fn organize(row: &SavedDownload) -> Result<Option<PathBuf>> {
    let dir = root(row)?;
    if !row.local_path.is_empty() && dir.try_exists(completed_relative(row)?)? {
        return Ok(Some(completed_relative(row)?));
    }
    let legacy = legacy_relative(row);
    let staging = staging_relative(row);
    let source = if dir.try_exists(&staging)? {
        staging
    } else if dir.try_exists(&legacy)? {
        legacy
    } else {
        return Ok(None);
    };
    let mut meta = dir.symlink_metadata(&source)?;
    ensure!(
        meta.is_file() && meta.len() == row.size,
        "The downloaded file is incomplete or unsafe"
    );
    let preferred = readable_relative(row)?;
    dir.create_dir_all(preferred.parent().unwrap())?;
    let mut prepared = false;
    for attempt in 0..10_000 {
        let target = if attempt == 0 {
            preferred.clone()
        } else {
            let stem = preferred.file_stem().unwrap().to_string_lossy();
            let ext = preferred
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            preferred.with_file_name(format!("{stem} ({}){ext}", attempt + 1))
        };
        if let Ok(existing) = dir.symlink_metadata(&target) {
            if same_file(&meta, &existing) {
                return Ok(Some(target));
            }
            continue;
        }
        if !prepared {
            #[cfg(target_os = "linux")]
            inherit_completed(&dir, &source, target.parent().unwrap())?;
            meta = dir.symlink_metadata(&source)?;
            prepared = true;
        }
        match dir.hard_link(&source, &dir, &target) {
            Ok(()) => return Ok(Some(target)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("Too many files with the same name in the destination folder")
}
pub fn remove_staging(row: &SavedDownload) -> Result<()> {
    let dir = root(row)?;
    for source in [legacy_relative(row), staging_relative(row)] {
        if let Ok(meta) = dir.symlink_metadata(&source) {
            let target = dir.symlink_metadata(completed_relative(row)?)?;
            ensure!(
                same_file(&meta, &target),
                "The original file changed; it was kept"
            );
            dir.remove_file(&source)?;
            let _ = dir.remove_dir(source.parent().unwrap());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn finalization_temporary_payload_is_never_shared_when_downloads_are_indexed() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let root = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let mut temporary = DestinationFile::create(&root, std::path::Path::new("shared")).unwrap();
        temporary
            .file
            .write_all(b"private unfinished data")
            .unwrap();
        std::fs::write(tmp.path().join("shared/public.flac"), b"public").unwrap();
        let db = tmp.path().join("index.sqlite3");
        crate::store::Store::open(&db).unwrap();
        let mut library =
            crate::library::Library::start(db, vec![tmp.path().join("shared")], true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(update) = library.updates.borrow().as_ref()
                && let Some(index) = &update.index
            {
                assert_eq!(index.file_count(), 1);
                assert_eq!(index.files()[0].size, 6);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        library.shutdown();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn replacing_a_temporary_name_cannot_substitute_payload_or_delete_another_file() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let root = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();
        let mut file = DestinationFile::create(&root, std::path::Path::new("public")).unwrap();
        let name = tmp.path().join("public").join(&file.name);
        std::fs::rename(&name, tmp.path().join("moved.part")).unwrap();
        std::fs::write(&name, b"unrelated").unwrap();
        root.create_dir("private").unwrap();
        let private = root.open_dir("private").unwrap();
        file.link_into(&private, std::path::Path::new("safe.part"))
            .unwrap();
        file.file.write_all(b"correct").unwrap();
        drop(file);
        assert_eq!(
            std::fs::read(tmp.path().join("private/safe.part")).unwrap(),
            b"correct"
        );
        assert_eq!(std::fs::read(name).unwrap(), b"unrelated");
    }
    #[cfg(target_os = "linux")]
    fn shared_default_acl(path: &std::path::Path) {
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, mode, id) in [
            (1u16, 7u16, u32::MAX),
            (2, 6, 42424),
            (4, 7, u32::MAX),
            (16, 7, u32::MAX),
            (32, 5, u32::MAX),
        ] {
            acl.extend(tag.to_le_bytes());
            acl.extend(mode.to_le_bytes());
            acl.extend(id.to_le_bytes());
        }
        rustix::fs::fsetxattr(
            std::fs::File::open(path).unwrap(),
            "system.posix_acl_default",
            &acl,
            rustix::fs::XattrFlags::empty(),
        )
        .unwrap();
    }
    #[cfg(target_os = "linux")]
    #[cfg(target_os = "linux")]
    #[test]
    fn completed_permissions_without_default_acl_match_an_ordinary_new_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let row = SavedDownload {
            directory: tmp.path().to_string_lossy().into_owned(),
            local_path: String::new(),
            username: "alice".into(),
            path: r"Music\plain.flac".into(),
            size: 4,
            status: "completed".into(),
        };
        work_directory(&row).unwrap();
        let source = tmp.path().join(staging_relative(&row));
        let part = source.with_file_name("plain.flac.part");
        std::fs::write(&part, b"data").unwrap();
        std::fs::rename(part, &source).unwrap();
        let reference = std::fs::File::create(tmp.path().join("alice/Music/reference")).unwrap();
        let completed = organize(&row).unwrap().unwrap();
        let actual = std::fs::File::open(tmp.path().join(completed)).unwrap();
        assert_eq!(
            actual.metadata().unwrap().permissions().mode(),
            reference.metadata().unwrap().permissions().mode()
        );
        assert!(access_acl(&actual).unwrap().is_none());
    }
    #[test]
    fn failed_publication_keeps_source_and_existing_destination_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let row = SavedDownload {
            directory: tmp.path().to_string_lossy().into_owned(),
            local_path: String::new(),
            username: "alice".into(),
            path: r"Music\song.flac".into(),
            size: 4,
            status: "completed".into(),
        };
        let source = tmp.path().join(staging_relative(&row));
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"data").unwrap();
        std::fs::create_dir(tmp.path().join("alice")).unwrap();
        std::fs::write(tmp.path().join("alice/Music"), b"keep").unwrap();
        assert!(organize(&row).is_err());
        assert_eq!(std::fs::read(source).unwrap(), b"data");
        assert_eq!(
            std::fs::read(tmp.path().join("alice/Music")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn partials_stay_private_and_completed_files_inherit_destination_acl() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let tmp = tempfile::tempdir().unwrap();
        shared_default_acl(tmp.path());
        let mut row = SavedDownload {
            directory: tmp.path().to_string_lossy().into_owned(),
            local_path: String::new(),
            username: "alice".into(),
            path: r"Music\song.flac".into(),
            size: 4,
            status: "completed".into(),
        };
        let work = work_directory(&row).unwrap();
        let part = tmp
            .path()
            .join(staging_relative(&row))
            .with_file_name("song.flac.part");
        assert_eq!(
            std::fs::metadata(&part).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(part.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        std::fs::write(&part, b"data").unwrap();
        let inode = std::fs::metadata(&part).unwrap().ino();
        let source = tmp.path().join(staging_relative(&row));
        std::fs::rename(&part, &source).unwrap();
        let expected = tmp.path().join("alice/Music/reference");
        let reference = std::fs::File::create(&expected).unwrap();
        let actual = organize(&row).unwrap().unwrap();
        let result = std::fs::File::open(tmp.path().join(&actual)).unwrap();
        assert_eq!(
            result.metadata().unwrap().ino(),
            inode,
            "normal finalization should not copy payload"
        );
        assert_eq!(
            result.metadata().unwrap().permissions().mode(),
            reference.metadata().unwrap().permissions().mode()
        );
        assert_eq!(
            result.metadata().unwrap().gid(),
            reference.metadata().unwrap().gid()
        );
        assert_eq!(
            access_acl(&result).unwrap(),
            access_acl(&reference).unwrap()
        );
        assert_eq!(organize(&row).unwrap().unwrap(), actual);
        row.local_path = actual.to_string_lossy().into_owned();
        remove_staging(&row).unwrap();
        assert_eq!(std::fs::read(tmp.path().join(&actual)).unwrap(), b"data");
        drop(work);
        assert!(
            !std::fs::read_dir(expected.parent().unwrap())
                .unwrap()
                .any(|e| e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".rustysoul-complete-"))
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn finalization_does_not_change_an_external_hardlink_and_remains_crash_recoverable() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let tmp = tempfile::tempdir().unwrap();
        shared_default_acl(tmp.path());
        let mut row = SavedDownload {
            directory: tmp.path().to_string_lossy().into_owned(),
            local_path: String::new(),
            username: "alice".into(),
            path: r"Music\song.flac".into(),
            size: 4,
            status: "completed".into(),
        };
        let source = tmp.path().join(staging_relative(&row));
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"data").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let linked = outside.path().join("untouched");
        std::fs::hard_link(&source, &linked).unwrap();
        let target = organize(&row).unwrap().unwrap();
        assert_ne!(
            std::fs::metadata(tmp.path().join(&target)).unwrap().ino(),
            std::fs::metadata(&linked).unwrap().ino()
        );
        assert_eq!(
            std::fs::metadata(&linked).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read(&linked).unwrap(), b"data");
        assert_eq!(organize(&row).unwrap().unwrap(), target);
        row.local_path = target.to_string_lossy().into_owned();
        remove_staging(&row).unwrap();
        assert_eq!(std::fs::read(tmp.path().join(target)).unwrap(), b"data");
    }
    #[test]
    fn a_name_collision_gets_a_readable_suffix_and_keeps_the_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let row = SavedDownload {
            directory: tmp.path().to_string_lossy().into_owned(),
            local_path: String::new(),
            username: "alice".into(),
            path: "Music\\song.flac".into(),
            size: 4,
            status: "completed".into(),
        };
        let source = tmp.path().join(legacy_relative(&row));
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"new!").unwrap();
        let target = tmp.path().join("alice/Music/song.flac");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"old!").unwrap();
        let relative = organize(&row).unwrap().unwrap();
        assert_eq!(relative, PathBuf::from("alice/Music/song (2).flac"));
        assert_eq!(std::fs::read(target).unwrap(), b"old!");
        assert_eq!(std::fs::read(tmp.path().join(relative)).unwrap(), b"new!");
    }
    #[test]
    fn files_are_grouped_by_user_and_album_without_overwriting_or_losing_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut row = SavedDownload {
            directory: tmp.path().to_string_lossy().into_owned(),
            local_path: String::new(),
            username: "a/b".into(),
            path: "Music\\Artist\\Album\\Track.flac".into(),
            size: 4,
            status: "completed".into(),
        };
        let legacy = tmp.path().join(legacy_relative(&row));
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"data").unwrap();
        let target = organize(&row).unwrap().unwrap();
        assert_eq!(target, PathBuf::from("a%2Fb/Music/Artist/Album/Track.flac"));
        assert!(legacy.exists());
        assert_eq!(organize(&row).unwrap().unwrap(), target);
        row.local_path = target.to_string_lossy().into_owned();
        remove_staging(&row).unwrap();
        assert!(!legacy.exists());
        assert_eq!(open_completed(&row).unwrap().metadata().unwrap().len(), 4);
    }
    #[test]
    fn outside_symlinks_and_unfinished_files_are_never_served() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"data").unwrap();
        let mut row = SavedDownload {
            directory: tmp.path().to_string_lossy().into_owned(),
            local_path: "escape/secret".into(),
            username: "a".into(),
            path: "secret".into(),
            size: 4,
            status: "completed".into(),
        };
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("escape")).unwrap();
        assert!(open_completed(&row).is_err());
        row.local_path = "../secret".into();
        assert!(open_completed(&row).is_err());
        row.status = "downloading".into();
        assert!(open_completed(&row).is_err());
    }
}
