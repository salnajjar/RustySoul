//! Private staging, readable completed paths, and capability-scoped file access.
use crate::store::SavedDownload;
use anyhow::{Context, Result, ensure};
use cap_std::{ambient_authority, fs::Dir};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

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
    dir.open_dir(parent)
        .with_context(|| format!("Cannot open download staging folder {}", parent.display()))
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
    let meta = dir.symlink_metadata(&source)?;
    ensure!(
        meta.is_file() && meta.len() == row.size,
        "The downloaded file is incomplete or unsafe"
    );
    let preferred = readable_relative(row)?;
    dir.create_dir_all(preferred.parent().unwrap())?;
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
