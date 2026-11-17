//! Encrypted streaming backups and locked, journalled restore. No media is archived.
use anyhow::{Context, Result, bail, ensure};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 16] = b"RustySoulBackup1";
const CHUNK: usize = 64 * 1024;
pub const MAX_DATABASE: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_UPLOAD: usize = (MAX_DATABASE + 4 * 1024 * 1024) as usize;
const SCHEMA: u32 = 13;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub version: u32,
    pub schema: u32,
    pub created_at: u64,
    pub application: String,
    pub database_bytes: u64,
    pub administrator: String,
    pub downloads: u64,
    pub uploads: u64,
    pub indexed_files: u64,
    pub locations: crate::model::FileLocations,
}
pub struct Prepared {
    pub id: String,
    pub summary: Summary,
    pub directory: tempfile::TempDir,
    pub export: bool,
}
impl Prepared {
    pub fn file(&self) -> PathBuf {
        self.directory.path().join(if self.export {
            "backup.rsb"
        } else {
            "state.sqlite3"
        })
    }
}
#[derive(Clone)]
pub struct Cancel(pub Arc<AtomicBool>);
impl Default for Cancel {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}
impl Cancel {
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.0.load(Ordering::Relaxed),
            "Backup operation cancelled"
        );
        Ok(())
    }
}
pub fn passphrase(value: &str) -> Result<()> {
    ensure!(
        value.chars().count() >= 8 && value.len() <= 1024 && !value.chars().any(char::is_control),
        "Backup passphrase must have at least 8 characters, at most 1024 bytes and no control characters"
    );
    Ok(())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn private_file(path: &Path) -> Result<File> {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    Ok(o.open(path)?)
}
pub(crate) fn regular(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "Backup state must use regular files"
    );
    Ok(())
}
pub(crate) fn workspace(data: &Path) -> Result<tempfile::TempDir> {
    let dir = tempfile::Builder::new()
        .prefix(".rustysoul-backup-")
        .tempdir_in(data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))?;
    }
    private_file(&dir.path().join(".workspace-v1"))?.write_all(MAGIC)?;
    Ok(dir)
}

fn open_read(path: &Path, cancel: &Cancel) -> Result<Connection> {
    regular(path)?;
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
        16 * 1024 * 1024,
    )?;
    db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;")?;

    let stop = cancel.0.clone();
    let deadline = Instant::now() + Duration::from_secs(180);
    db.progress_handler(
        10000,
        Some(move || stop.load(Ordering::Relaxed) || Instant::now() > deadline),
    )?;
    Ok(db)
}
pub(crate) fn space(data: &Path, needed: u64) -> Result<()> {
    #[cfg(unix)]
    {
        let s = rustix::fs::fstatvfs(File::open(data)?)?;
        let free = s.f_bavail.saturating_mul(s.f_frsize);
        ensure!(
            free >= needed.saturating_add(64 * 1024 * 1024),
            "Not enough free space in RustySoul's data folder for this backup operation"
        );
    }
    Ok(())
}
fn snapshot(source: &Path, target: &Path, cancel: &Cancel) -> Result<()> {
    let source = open_read(source, cancel)?;
    let pages = source.pragma_query_value(None, "page_count", |r| r.get::<_, i64>(0))? as u64;
    let page_size = source.pragma_query_value(None, "page_size", |r| r.get::<_, i64>(0))? as u64;
    ensure!(
        pages.saturating_mul(page_size) <= MAX_DATABASE,
        "Application database exceeds the 2 GiB backup limit"
    );
    let mut dest = Connection::open(target)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(target, fs::Permissions::from_mode(0o600))?;
    }
    let copy = Backup::new(&source, &mut dest)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        cancel.check()?;
        ensure!(
            Instant::now() < deadline,
            "Database snapshot timed out; retry when indexing is idle"
        );
        space(
            target.parent().context("Missing snapshot directory")?,
            CHUNK as u64,
        )?;
        match copy.step(128)? {
            StepResult::Done => break,
            StepResult::More => {}
            StepResult::Busy | StepResult::Locked => std::thread::sleep(Duration::from_millis(10)),
            _ => bail!("Database snapshot could not complete"),
        }
    }
    drop(copy);
    drop(dest);
    File::open(target)?.sync_all()?;
    Ok(())
}
fn derive(pass: &str, salt: &[u8; 16]) -> Result<ChaCha20Poly1305> {
    passphrase(pass)?;
    let mut key = [0; 32];
    argon2::Argon2::default()
        .hash_password_into(pass.as_bytes(), salt, &mut key)
        .map_err(|_| anyhow::anyhow!("Could not derive backup key"))?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    key.fill(0);
    Ok(cipher)
}
fn nonce(prefix: &[u8; 8], index: u32) -> [u8; 12] {
    let mut n = [0; 12];
    n[..8].copy_from_slice(prefix);
    n[8..].copy_from_slice(&index.to_le_bytes());
    n
}
fn header(pass: &str, mut out: impl Write) -> Result<(ChaCha20Poly1305, Vec<u8>, [u8; 8])> {
    let mut salt = [0; 16];
    let mut prefix = [0; 8];
    getrandom::fill(&mut salt).map_err(|_| anyhow::anyhow!("Random source unavailable"))?;
    getrandom::fill(&mut prefix).map_err(|_| anyhow::anyhow!("Random source unavailable"))?;
    let h = [MAGIC.as_slice(), &salt, &prefix].concat();
    out.write_all(&h)?;
    Ok((derive(pass, &salt)?, h, prefix))
}
fn encrypt(
    pass: &str,
    key: &[u8; 32],
    summary: &Summary,
    db: &Path,
    out: &Path,
    cancel: &Cancel,
) -> Result<()> {
    let meta = serde_json::to_vec(summary)?;
    ensure!(meta.len() <= 16384, "Backup metadata is too large");
    let mut output = private_file(out)?;
    let (cipher, h, prefix) = header(pass, &mut output)?;
    let prefix_data = [(meta.len() as u32).to_le_bytes().as_slice(), &meta, key].concat();
    let mut input = std::io::Cursor::new(prefix_data).chain(File::open(db)?);
    let mut index = 0u32;
    let mut buffer = vec![0; CHUNK];
    loop {
        cancel.check()?;
        space(
            out.parent().context("Missing backup directory")?,
            CHUNK as u64,
        )?;
        let n = input.read(&mut buffer)?;
        let final_frame = u8::from(n == 0);
        let aad = [h.as_slice(), &index.to_le_bytes(), &[final_frame]].concat();
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce(&prefix, index)),
                Payload {
                    msg: &buffer[..n],
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("Backup encryption failed"))?;
        output.write_all(&[final_frame])?;
        output.write_all(&(encrypted.len() as u32).to_le_bytes())?;
        output.write_all(&encrypted)?;
        if final_frame == 1 {
            break;
        }
        index = index.checked_add(1).context("Backup is too large")?;
    }
    output.sync_all()?;
    Ok(())
}
struct Decrypt<R: Read> {
    input: R,
    cipher: ChaCha20Poly1305,
    h: Vec<u8>,
    prefix: [u8; 8],
    index: u32,
    buffer: std::io::Cursor<Vec<u8>>,
    done: bool,
    cancel: Cancel,
    total: u64,
}
impl<R: Read> Decrypt<R> {
    fn new(mut input: R, pass: &str, cancel: Cancel) -> Result<Self> {
        let mut h = vec![0; 40];
        input
            .read_exact(&mut h)
            .context("Not a complete RustySoul backup")?;
        ensure!(&h[..16] == MAGIC, "Unsupported RustySoul backup format");
        let salt = h[16..32].try_into()?;
        let prefix = h[32..40].try_into()?;
        let cipher = derive(pass, &salt)?;
        Ok(Self {
            input,
            cipher,
            h,
            prefix,
            index: 0,
            buffer: std::io::Cursor::new(vec![]),
            done: false,
            cancel,
            total: 0,
        })
    }
    fn frame(&mut self) -> Result<()> {
        self.cancel.check()?;
        let mut flag = [0];
        self.input
            .read_exact(&mut flag)
            .context("Backup is truncated")?;
        ensure!(flag[0] <= 1, "Invalid backup frame");
        let mut length = [0; 4];
        self.input.read_exact(&mut length)?;
        let length = u32::from_le_bytes(length) as usize;
        ensure!(
            (16..=CHUNK + 16).contains(&length),
            "Invalid backup frame length"
        );
        let mut data = vec![0; length];
        self.input
            .read_exact(&mut data)
            .context("Backup is truncated")?;
        let aad = [self.h.as_slice(), &self.index.to_le_bytes(), &flag].concat();
        let plain = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce(&self.prefix, self.index)),
                Payload {
                    msg: &data,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("Wrong passphrase or damaged backup"))?;
        self.total += plain.len() as u64;
        ensure!(
            self.total <= MAX_DATABASE + 16420,
            "Decrypted backup exceeds the 2 GiB limit"
        );
        if flag[0] == 1 {
            ensure!(plain.is_empty(), "Invalid backup ending");
            let mut tail = [0];
            ensure!(
                self.input.read(&mut tail)? == 0,
                "Unexpected data after backup ending"
            );
            self.done = true;
        } else {
            ensure!(!plain.is_empty(), "Invalid empty backup frame");
            self.index = self
                .index
                .checked_add(1)
                .context("Backup has too many frames")?;
        }
        self.buffer = std::io::Cursor::new(plain);
        Ok(())
    }
}
impl<R: Read> Read for Decrypt<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.buffer.read(out)?;
            if n > 0 || self.done {
                return Ok(n);
            }
            self.frame().map_err(std::io::Error::other)?;
        }
    }
}
type SchemaRow = (String, String, String, Option<String>);
fn schema(db: &Connection) -> Result<Vec<SchemaRow>> {
    Ok(db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<rusqlite::Result<_>>()?)
}
fn inspect(
    db_path: &Path,
    key_path: &Path,
    created: u64,
    config: &crate::config::Config,
    cancel: &Cancel,
) -> Result<Summary> {
    cancel.check()?;
    let db = open_read(db_path, cancel)?;
    let saved_schema = db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))?;
    ensure!(
        (11..=SCHEMA).contains(&saved_schema),
        "Backup schema is not supported by this version of RustySoul"
    );
    // Only the application's exact schema is accepted; no injected triggers/views.
    let expected = workspace(&config.data_dir)?;
    let expected_store =
        crate::store::Store::open_at_schema(&expected.path().join("schema.sqlite3"), saved_schema)?;
    expected_store
        .connection
        .execute_batch(crate::auth::BACKUP_AUTH_SCHEMA)?;
    ensure!(
        schema(&db)? == schema(&expected_store.connection)?,
        "Backup database schema does not match RustySoul"
    );
    ensure!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok",
        "Backup database failed its integrity check"
    );
    ensure!(
        !db.prepare("PRAGMA foreign_key_check")?.exists([])?,
        "Backup database has broken relationships"
    );
    let (admin, hash): (String, String) = db.query_row(
        "SELECT username,hash FROM admin_account WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    crate::auth::validate(&admin, "validation-password")?;
    let parsed = argon2::password_hash::PasswordHash::new(&hash)
        .map_err(|_| anyhow::anyhow!("Invalid administrator password hash"))?;
    ensure!(
        parsed.version == Some(19)
            && parsed.hash.is_some_and(|h| h.len() == 32)
            && parsed.algorithm.as_str() == "argon2id"
            && parsed.params.get_decimal("m") == Some(19456)
            && parsed.params.get_decimal("t") == Some(2)
            && parsed.params.get_decimal("p") == Some(1),
        "Unsupported administrator password hash parameters"
    );
    let locations: crate::model::FileLocations = db
        .query_row("SELECT json FROM file_locations WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or(crate::model::FileLocations {
            downloads: config
                .download_dir
                .clone()
                .unwrap_or_else(|| config.data_dir.join("downloads"))
                .to_string_lossy()
                .into_owned(),
            shares: config
                .soulseek
                .shares
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        });
    crate::model::Command::UpdateLocations {
        downloads: locations.downloads.clone(),
        shares: locations.shares.clone(),
    }
    .validate()
    .map_err(anyhow::Error::msg)?;
    crate::profile::load(&db)?.validate()?;
    crate::sharing::load(&db)?
        .validate()
        .map_err(anyhow::Error::msg)?;
    let locations = crate::library::validate_locations(&locations, &config.data_dir)?;
    crate::library::check_location_roots(&locations, &config.location_roots)?;
    let vault = crate::vault::Vault::open(key_path, true)?;
    if let Some(blob) = db
        .query_row("SELECT encrypted FROM credentials WHERE id=1", [], |r| {
            r.get::<_, Vec<u8>>(0)
        })
        .optional()?
    {
        vault.decrypt(&blob)?;
    }
    // Restored path strings never get to bypass the normal path/installation checks.
    let mut rows =
        db.prepare("SELECT username,path,directory,local_path,size,status FROM downloads")?;
    let mut cursor = rows.query([])?;
    let mut unfinished = 0usize;
    while let Some(row) = cursor.next()? {
        cancel.check()?;
        let size: i64 = row.get(4)?;
        crate::model::Command::Download {
            username: row.get(0)?,
            path: row.get(1)?,
            size: u64::try_from(size).context("Invalid download size")?,
        }
        .validate()
        .map_err(anyhow::Error::msg)?;
        let status: String = row.get(5)?;
        if !matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted" | "timed_out"
        ) {
            unfinished += 1;
            ensure!(
                unfinished <= config.limits.transfers,
                "Backup queue exceeds the configured transfer limit"
            );
        }
        let directory: String = row.get(2)?;
        let path = Path::new(&directory);
        ensure!(
            path.is_absolute()
                && !path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "Invalid saved download destination"
        );
        let allowed = crate::model::FileLocations {
            downloads: directory,
            shares: vec![],
        };
        crate::library::check_location_roots(&allowed, &config.location_roots)?;
        let local: String = row.get(3)?;
        ensure!(
            local.is_empty()
                || (!Path::new(&local).is_absolute()
                    && Path::new(&local)
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_)))),
            "Invalid saved media path"
        );
    }
    if let Some(json) = db
        .query_row("SELECT json FROM connection_settings WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    {
        serde_json::from_str::<crate::model::NetworkSettings>(&json)?
            .validate()
            .map_err(anyhow::Error::msg)?;
    }
    if saved_schema >= 12 {
        crate::queue_order::load(&db)?;
    }
    let count = |table: &str| -> Result<u64> {
        Ok(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })? as u64,
        )
    };
    Ok(Summary {
        version: 1,
        schema: saved_schema,
        created_at: created,
        application: env!("CARGO_PKG_VERSION").into(),
        database_bytes: fs::metadata(db_path)?.len(),
        administrator: admin,
        downloads: count("downloads")?,
        uploads: count("uploads")?,
        indexed_files: count("local_files")?,
        locations,
    })
}
use rusqlite::OptionalExtension;
pub fn export(config: &crate::config::Config, pass: &str, cancel: &Cancel) -> Result<Prepared> {
    passphrase(pass)?;
    let data = &config.data_dir;
    let source = open_read(&data.join("rustysoul.sqlite3"), cancel)?;
    let pages = source.pragma_query_value(None, "page_count", |r| r.get::<_, i64>(0))? as u64;
    let page_size = source.pragma_query_value(None, "page_size", |r| r.get::<_, i64>(0))? as u64;
    let size = pages
        .checked_mul(page_size)
        .context("Database capacity overflow")?;
    drop(source);
    space(data, size.saturating_mul(3))?;
    let directory = workspace(data)?;
    let db = directory.path().join("state.sqlite3");
    snapshot(&data.join("rustysoul.sqlite3"), &db, cancel)?;
    crate::vault::Vault::open(&data.join("credentials.key"), true)?;
    let key = fs::read(data.join("credentials.key"))?;
    ensure!(key.len() == 32, "Invalid credential key");
    let key_path = directory.path().join("credentials.key");
    private_file(&key_path)?.write_all(&key)?;
    let mut summary = inspect(&db, &key_path, now(), config, cancel)?;
    let mut copy = Connection::open(&db)?;
    let cancelled = cancel.clone();
    let deadline = Instant::now() + Duration::from_secs(180);
    copy.progress_handler(
        1000,
        Some(move || cancelled.check().is_err() || Instant::now() >= deadline),
    )?;
    copy.execute_batch("PRAGMA journal_mode=DELETE; DELETE FROM admin_sessions; VACUUM;")?;
    drop(copy);
    summary.database_bytes = fs::metadata(&db)?.len();
    encrypt(
        pass,
        key.as_slice().try_into()?,
        &summary,
        &db,
        &directory.path().join("backup.rsb"),
        cancel,
    )?;
    fs::remove_file(&db)?;
    fs::remove_file(key_path)?;
    Ok(Prepared {
        id: uuid::Uuid::new_v4().to_string(),
        summary,
        directory,
        export: true,
    })
}
pub fn preview(
    input: &Path,
    pass: &str,
    config: &crate::config::Config,
    cancel: &Cancel,
) -> Result<Prepared> {
    passphrase(pass)?;
    regular(input)?;
    let length = fs::metadata(input)?.len();
    ensure!(
        length <= MAX_UPLOAD as u64,
        "Backup file exceeds the 2 GiB limit"
    );
    space(&config.data_dir, length.saturating_mul(2))?;
    let directory = workspace(&config.data_dir)?;
    let mut stream = Decrypt::new(File::open(input)?, pass, cancel.clone())?;
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(length <= 16384, "Invalid backup metadata length");
    let mut metadata = vec![0; length];
    stream.read_exact(&mut metadata)?;
    let declared: Summary = serde_json::from_slice(&metadata)?;
    ensure!(
        declared.version == 1 && declared.database_bytes <= MAX_DATABASE,
        "Unsupported or oversized backup"
    );
    let mut key = [0; 32];
    stream.read_exact(&mut key)?;
    let key_path = directory.path().join("credentials.key");
    private_file(&key_path)?.write_all(&key)?;
    key.fill(0);
    let db_path = directory.path().join("state.sqlite3");
    let mut db = private_file(&db_path)?;
    let mut copied = 0u64;
    let mut buffer = vec![0; CHUNK];
    loop {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        space(directory.path(), CHUNK as u64)?;
        db.write_all(&buffer[..n])?;
        copied += n as u64;
    }
    ensure!(
        copied == declared.database_bytes,
        "Backup database length does not match its manifest"
    );
    db.sync_all()?;
    drop(db);
    let summary = inspect(&db_path, &key_path, declared.created_at, config, cancel)?;
    ensure!(
        summary.schema == declared.schema
            && summary.administrator == declared.administrator
            && summary.downloads == declared.downloads
            && summary.uploads == declared.uploads
            && summary.indexed_files == declared.indexed_files,
        "Backup contents do not match their manifest"
    );
    Ok(Prepared {
        id: uuid::Uuid::new_v4().to_string(),
        summary,
        directory,
        export: false,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    id: String,
    database_hash: String,
    key_hash: String,
}
fn digest(path: &Path) -> Result<String> {
    regular(path)?;
    let mut input = File::open(path)?;
    let mut h = Sha256::new();
    let mut bytes = [0; CHUNK];
    loop {
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        h.update(&bytes[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub(crate) fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
pub(crate) fn copy_private(source: &Path, dest: &Path) -> Result<()> {
    regular(source)?;
    let mut input = File::open(source)?;
    let mut output = private_file(dest)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}
pub(crate) fn atomic_copy(source: &Path, dest: &Path) -> Result<()> {
    let tmp = dest.with_extension("restore-new");
    if tmp.exists() {
        regular(&tmp)?;
        fs::remove_file(&tmp)?;
    }
    copy_private(source, &tmp)?;
    fs::rename(tmp, dest)?;
    sync_dir(dest.parent().context("Missing state directory")?)?;
    Ok(())
}
pub(crate) fn lock(config: &crate::config::Config) -> Result<File> {
    let metadata = fs::symlink_metadata(&config.data_dir)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Data folder must be a real private directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "Data folder must be private (mode 700)"
        );
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let file = options.open(config.data_dir.join("daemon.lock"))?;
    file.try_lock()
        .context("Stop RustySoul before restoring; this data folder is in use")?;
    Ok(file)
}
pub fn stage(prepared: &Prepared, config: &crate::config::Config) -> Result<()> {
    ensure!(
        !prepared.export,
        "Choose and validate a backup before restoring"
    );
    ensure!(
        !config.data_dir.join("restore-pending").exists()
            && !crate::maintenance::pending(&config.data_dir),
        "A restore is already staged; restart RustySoul first"
    );
    let cancel = Cancel::default();
    inspect(
        &prepared.file(),
        &prepared.directory.path().join("credentials.key"),
        prepared.summary.created_at,
        config,
        &cancel,
    )?;
    let current = fs::metadata(config.data_dir.join("rustysoul.sqlite3"))?.len();
    space(
        &config.data_dir,
        prepared
            .summary
            .database_bytes
            .saturating_add(current.saturating_mul(2)),
    )?;
    let directory = workspace(&config.data_dir)?;
    let db = directory.path().join("state.sqlite3");
    copy_private(&prepared.file(), &db)?;
    copy_private(
        &prepared.directory.path().join("credentials.key"),
        &directory.path().join("credentials.key"),
    )?;
    let state = crate::store::Store::open(&db)?;
    let mut network = state.network()?.unwrap_or_default();
    network.connect_on_startup = false;
    network.upnp = false;
    state.save_network(&network)?;
    state.connection.execute_batch("DELETE FROM admin_sessions; DELETE FROM library_cache; DELETE FROM local_files; UPDATE downloads SET status='paused_waiting',speed=0 WHERE status IN ('waiting','waiting_for_space','queued','downloading','retry_waiting'); UPDATE uploads SET status='interrupted',speed=0 WHERE status IN ('queued','uploading','checking_shares'); UPDATE download_retries SET next_at=NULL;")?;
    state.connection.execute(
        "UPDATE installation SET identity=?1 WHERE id=1",
        [uuid::Uuid::new_v4().to_string()],
    )?;
    state.checkpoint()?;
    drop(state);
    // Store::open uses WAL; close/checkpoint before hashing/copying the database.
    let journal = Journal {
        id: uuid::Uuid::new_v4().to_string(),
        database_hash: digest(&db)?,
        key_hash: digest(&directory.path().join("credentials.key"))?,
    };
    let mut manifest = private_file(&directory.path().join("journal.json"))?;
    manifest.write_all(&serde_json::to_vec(&journal)?)?;
    manifest.sync_all()?;
    sync_dir(directory.path())?;
    fs::rename(directory.path(), config.data_dir.join("restore-pending"))?;
    sync_dir(&config.data_dir)?;
    Ok(())
}
fn install(dir: &Path, config: &crate::config::Config, prefix: &str) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let p = config.data_dir.join(format!("rustysoul.sqlite3{suffix}"));
        if p.exists() {
            regular(&p)?;
            fs::remove_file(p)?;
        }
    }
    atomic_copy(
        &dir.join(format!("{prefix}state.sqlite3")),
        &config.data_dir.join("rustysoul.sqlite3"),
    )?;
    atomic_copy(
        &dir.join(format!("{prefix}credentials.key")),
        &config.data_dir.join("credentials.key"),
    )?;
    Ok(())
}
fn apply_pending_inner(config: &crate::config::Config) -> Result<Option<String>> {
    let pending = config.data_dir.join("restore-pending");
    if !pending.exists() {
        return Ok(None);
    }
    let _lock = lock(config)?;
    ensure!(
        fs::symlink_metadata(&pending)?.is_dir()
            && !fs::symlink_metadata(&pending)?.file_type().is_symlink(),
        "Invalid pending restore directory"
    );
    regular(&pending.join("journal.json"))?;
    ensure!(
        fs::metadata(pending.join("journal.json"))?.len() < 4096,
        "Invalid restore journal"
    );
    let journal: Journal = serde_json::from_slice(&fs::read(pending.join("journal.json"))?)?;
    uuid::Uuid::parse_str(&journal.id)?;
    ensure!(
        digest(&pending.join("state.sqlite3"))? == journal.database_hash
            && digest(&pending.join("credentials.key"))? == journal.key_hash,
        "Staged restore changed; current state has not been replaced"
    );
    inspect(
        &pending.join("state.sqlite3"),
        &pending.join("credentials.key"),
        now(),
        config,
        &Cancel::default(),
    )?;
    let rollback = pending.join("rollback-ready");
    if !rollback.exists() {
        let temporary = pending.join("rollback-work");
        if temporary.exists() {
            ensure!(
                fs::symlink_metadata(&temporary)?.is_dir()
                    && !fs::symlink_metadata(&temporary)?.file_type().is_symlink(),
                "Invalid rollback directory"
            );
            fs::remove_dir_all(&temporary)?;
        }
        fs::create_dir(&temporary)?;
        snapshot(
            &config.data_dir.join("rustysoul.sqlite3"),
            &temporary.join("rollback-state.sqlite3"),
            &Cancel::default(),
        )?;
        copy_private(
            &config.data_dir.join("credentials.key"),
            &temporary.join("rollback-credentials.key"),
        )?;
        sync_dir(&temporary)?;
        fs::rename(&temporary, &rollback)?;
        sync_dir(&pending)?;
    }
    let applied = (|| -> Result<()> {
        install(&pending, config, "")?;
        inspect(
            &config.data_dir.join("rustysoul.sqlite3"),
            &config.data_dir.join("credentials.key"),
            now(),
            config,
            &Cancel::default(),
        )?;
        Ok(())
    })();
    if let Err(error) = applied {
        install(&rollback, config, "rollback-")?;
        bail!("Restore failed; previous database and credential key were recovered: {error:#}");
    }
    let media_safe = config.data_dir.join("restore-media-safe");
    if !media_safe.exists() {
        private_file(&media_safe)?.sync_all()?;
    }
    let marker = config.data_dir.join("restore-reindex");
    if !marker.exists() {
        private_file(&marker)?.sync_all()?;
    }
    let history = config.data_dir.join("restore-history");
    fs::create_dir_all(&history)?;
    ensure!(
        !fs::symlink_metadata(&history)?.file_type().is_symlink(),
        "Invalid restore history directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&history, fs::Permissions::from_mode(0o700))?;
    }
    fs::rename(&pending, history.join(&journal.id))?;
    sync_dir(&history)?;
    sync_dir(&config.data_dir)?;
    // Keep only the latest two application-managed rollback sets, never media.
    let mut entries = fs::read_dir(&history)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_dir())
                && uuid::Uuid::parse_str(&e.file_name().to_string_lossy()).is_ok()
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    let remove = entries.len().saturating_sub(2);
    for e in entries.into_iter().take(remove) {
        fs::remove_dir_all(e.path())?;
    }
    for name in ["state.sqlite3", "credentials.key"] {
        fs::remove_file(history.join(&journal.id).join(name))?;
    }
    Ok(Some(journal.id))
}
pub fn restore_file(input: &Path, pass: &str, config: &crate::config::Config) -> Result<String> {
    {
        let _lock = lock(config)?;
        let prepared = preview(input, pass, config, &Cancel::default())?;
        stage(&prepared, config)?;
    }
    apply_pending(config)?.context("Restore was not staged")
}

pub fn export_file(config: &crate::config::Config, pass: &str, out: &Path) -> Result<()> {
    let _lock = lock(config)?;
    ensure!(
        !out.exists(),
        "Backup output already exists; choose another filename"
    );
    let prepared = export(config, pass, &Cancel::default())?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut File::open(prepared.file())?, &mut temporary)?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(out).map_err(|e| e.error)?;
    sync_dir(parent)
}
pub fn inspect_file(config: &crate::config::Config, input: &Path, pass: &str) -> Result<Summary> {
    let _lock = lock(config)?;
    Ok(preview(input, pass, config, &Cancel::default())?.summary)
}
pub fn cleanup_temporary(data: &Path) -> Result<()> {
    for entry in fs::read_dir(data)? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".rustysoul-backup-")
            || !entry.file_type()?.is_dir()
        {
            continue;
        }
        let marker = entry.path().join(".workspace-v1");
        if regular(&marker).is_ok()
            && fs::metadata(&marker)?.len() == MAGIC.len() as u64
            && fs::read(&marker)? == MAGIC
        {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

fn result_record(config: &crate::config::Config, status: &str, message: &str) -> Result<()> {
    let path = config.data_dir.join("restore-last-result.json");
    let temporary = path.with_extension("new");
    if temporary.exists() {
        regular(&temporary)?;
        fs::remove_file(&temporary)?;
    }
    let mut file = private_file(&temporary)?;
    file.write_all(&serde_json::to_vec(&serde_json::json!({"id":uuid::Uuid::new_v4().to_string(),"status":status,"message":message,"at":now()}))?)?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    sync_dir(&config.data_dir)
}
pub(crate) fn last_result(data: &Path) -> Option<serde_json::Value> {
    let path = data.join("restore-last-result.json");
    regular(&path).ok()?;
    if fs::metadata(&path).ok()?.len() > 8192 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
pub fn apply_pending(config: &crate::config::Config) -> Result<Option<String>> {
    match apply_pending_inner(config) {
        Ok(value) => {
            if value.is_some() {
                result_record(
                    config,
                    "restored",
                    "Application state restored. Sign in with the backup account; Soulseek starts disconnected and the library index is rebuilt.",
                )?;
            }
            Ok(value)
        }
        Err(error) => {
            let _lock = lock(config)?;
            let pending = config.data_dir.join("restore-pending");
            if !pending.exists() {
                return Err(error);
            }
            let rollback = pending.join("rollback-ready");
            if rollback.exists() {
                let m = fs::symlink_metadata(&rollback)?;
                ensure!(
                    m.is_dir() && !m.file_type().is_symlink(),
                    "Invalid rollback set"
                );
                install(&rollback, config, "rollback-")
                    .context("Restore failed and rollback needs manual recovery")?;
            }
            inspect(
                &config.data_dir.join("rustysoul.sqlite3"),
                &config.data_dir.join("credentials.key"),
                now(),
                config,
                &Cancel::default(),
            )
            .context("Could not verify previous application state after rejected restore")?;
            let rejected = config
                .data_dir
                .join(format!("restore-rejected-{}", uuid::Uuid::new_v4()));
            fs::rename(&pending, &rejected)?;
            sync_dir(&config.data_dir)?;
            for name in ["state.sqlite3", "credentials.key"] {
                let p = rejected.join(name);
                if p.exists() {
                    regular(&p)?;
                    fs::remove_file(p)?;
                }
            }
            tracing::warn!("Restore was rejected; previous application state recovered: {error:#}");
            result_record(
                config,
                "rejected",
                "Restore could not be applied. The previous application state was recovered; validate the backup and folder mappings before trying again.",
            )?;
            Ok(None)
        }
    }
}

pub(crate) fn check_upload_space(data: &Path, bytes: u64) -> Result<()> {
    space(data, bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, crate::config::Config, String) {
        fixture_at_schema(SCHEMA)
    }
    fn fixture_at_schema(schema: u32) -> (tempfile::TempDir, crate::config::Config, String) {
        let temp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let downloads = temp.path().join("downloads");
        fs::create_dir(&downloads).unwrap();
        let config = crate::config::Config {
            data_dir: temp.path().into(),
            download_dir: Some(downloads),
            ..Default::default()
        };
        let store =
            crate::store::Store::open_at_schema(&temp.path().join("rustysoul.sqlite3"), schema)
                .unwrap();
        let auth = crate::auth::Auth::open(&temp.path().join("rustysoul.sqlite3")).unwrap();
        let login = auth.login("admin", "change-me-now").unwrap();
        let login = auth
            .change(
                &login.token,
                "change-me-now",
                "backup-admin",
                "new-password",
            )
            .unwrap();
        let vault = crate::vault::Vault::open(&temp.path().join("credentials.key"), false).unwrap();
        store
            .save_credentials(&vault.encrypt("soul-user", "soul-password").unwrap())
            .unwrap();
        store.set_buddy("original-buddy", true).unwrap();
        (temp, config, login.token)
    }
    #[test]
    fn startup_cleans_only_marked_backup_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let stale = workspace(temp.path()).unwrap().keep();
        fs::write(stale.join("plain-state"), b"private").unwrap();
        let unrelated = temp.path().join(".rustysoul-backup-unrelated");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join(".workspace-v1"), b"different").unwrap();
        let ordinary = temp.path().join("media");
        fs::create_dir(&ordinary).unwrap();
        fs::write(ordinary.join(".workspace-v1"), MAGIC).unwrap();
        cleanup_temporary(temp.path()).unwrap();
        assert!(!stale.exists());
        assert!(unrelated.is_dir());
        assert!(ordinary.is_dir());
    }
    #[test]
    fn encrypted_snapshot_roundtrip_strips_sessions_and_preserves_the_live_database() {
        let (_temp, config, token) = fixture();
        let store = crate::store::Store::open(&config.data_dir.join("rustysoul.sqlite3")).unwrap();
        for path in ["a.flac", "b.flac"] {
            store.connection.execute("INSERT INTO downloads(username,path,directory,size,status) VALUES('queue-user',?1,?2,CASE WHEN ?1='a.flac' THEN 0 ELSE 100 END,'waiting')", rusqlite::params![path, config.download_dir.as_ref().unwrap().to_str().unwrap()]).unwrap();
        }
        crate::queue_order::update(
            &store.connection,
            "downloads",
            "queue-user",
            "b.flac",
            Some(1),
            None,
        )
        .unwrap();
        let expected = crate::queue_order::load(&store.connection).unwrap();
        drop(store);
        let stop = Cancel::default();
        let backup = export(&config, "backup-password", &stop).unwrap();
        let bytes = fs::read(backup.file()).unwrap();
        assert!(!bytes.windows(13).any(|w| w == b"soul-password"));
        let verified = preview(&backup.file(), "backup-password", &config, &stop).unwrap();
        assert_eq!(verified.summary.administrator, "backup-admin");
        assert_eq!(verified.summary.schema, SCHEMA);
        let db = Connection::open(verified.file()).unwrap();
        let restored = crate::queue_order::load(&db).unwrap();
        assert_eq!(restored.len(), expected.len());
        for (key, rule) in expected {
            assert_eq!(restored[&key].priority, rule.priority);
            assert_eq!(restored[&key].position, rule.position);
        }

        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM admin_sessions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let auth = crate::auth::Auth::open(&config.data_dir.join("rustysoul.sqlite3")).unwrap();
        assert!(auth.session(&token).is_some());
    }

    #[test]
    fn legacy_schema_eleven_and_twelve_archives_restore_before_migration() {
        for legacy in [11, 12] {
            let (_temp, config, _) = fixture_at_schema(legacy);
            if legacy == 12 {
                let db = Connection::open(config.data_dir.join("rustysoul.sqlite3")).unwrap();
                db.execute("INSERT INTO downloads(username,path,size,status,directory) VALUES('peer','old.flac',100,'waiting',?1)",[config.download_dir.as_ref().unwrap().to_str().unwrap()]).unwrap();
                crate::queue_order::update(&db, "downloads", "peer", "old.flac", Some(1), None)
                    .unwrap();
            }
            let stop = Cancel::default();
            let backup = export(&config, "backup-password", &stop).unwrap();
            assert_eq!(backup.summary.schema, legacy);
            assert_eq!(
                preview(&backup.file(), "backup-password", &config, &stop)
                    .unwrap()
                    .summary
                    .schema,
                legacy
            );
            restore_file(&backup.file(), "backup-password", &config).unwrap();
            let store =
                crate::store::Store::open(&config.data_dir.join("rustysoul.sqlite3")).unwrap();
            assert_eq!(
                store
                    .connection
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                SCHEMA
            );
            let rules = crate::queue_order::load(&store.connection).unwrap();
            assert_eq!(rules.len(), if legacy == 12 { 1 } else { 0 });
            if legacy == 12 {
                assert_eq!(rules.values().next().unwrap().priority, 1);
            }
            assert_eq!(
                store
                    .connection
                    .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
        }
    }
    #[test]
    fn claiming_an_older_schema_does_not_bypass_exact_backup_validation() {
        let (_temp, config, _) = fixture();
        let db = Connection::open(config.data_dir.join("rustysoul.sqlite3")).unwrap();
        db.execute_batch("PRAGMA user_version=12;").unwrap();
        drop(db);
        assert!(export(&config, "backup-password", &Cancel::default()).is_err());
    }
    #[test]
    fn wrong_password_truncation_corruption_and_trailing_bytes_never_stage_a_restore() {
        let (_temp, config, _) = fixture();
        let stop = Cancel::default();
        let backup = export(&config, "backup-password", &stop).unwrap();
        assert!(preview(&backup.file(), "incorrect-password", &config, &stop).is_err());
        let source = fs::read(backup.file()).unwrap();
        for (index, mut bytes) in [
            (0, source[..source.len() - 1].to_vec()),
            (1, source.clone()),
            (2, source.clone()),
        ] {
            if index == 1 {
                bytes[55] ^= 1;
            }
            if index == 2 {
                bytes.push(0);
            }
            let file = config.data_dir.join(format!("bad-{index}"));
            fs::write(&file, bytes).unwrap();
            assert!(preview(&file, "backup-password", &config, &stop).is_err());
            assert!(!config.data_dir.join("restore-pending").exists());
        }
    }
    #[test]
    fn restore_keeps_rollback_and_media_revokes_sessions_and_disables_startup_connect() {
        let (_temp, config, token) = fixture();
        let media = config
            .download_dir
            .as_ref()
            .unwrap()
            .join("do-not-touch.flac");
        fs::write(&media, b"media").unwrap();
        let stop = Cancel::default();
        let backup = export(&config, "backup-password", &stop).unwrap();
        let store = crate::store::Store::open(&config.data_dir.join("rustysoul.sqlite3")).unwrap();
        store.set_buddy("later-buddy", true).unwrap();
        drop(store);
        let id = restore_file(&backup.file(), "backup-password", &config).unwrap();
        assert!(
            config
                .data_dir
                .join("restore-history")
                .join(id)
                .join("rollback-ready/rollback-state.sqlite3")
                .is_file()
        );
        assert_eq!(fs::read(media).unwrap(), b"media");
        let store = crate::store::Store::open(&config.data_dir.join("rustysoul.sqlite3")).unwrap();
        assert_eq!(store.buddies().unwrap(), ["original-buddy"]);
        assert!(!store.network().unwrap().unwrap().connect_on_startup);
        let auth = crate::auth::Auth::open(&config.data_dir.join("rustysoul.sqlite3")).unwrap();
        assert!(auth.session(&token).is_none());
        assert!(auth.login("backup-admin", "new-password").is_ok());
        let vault =
            crate::vault::Vault::open(&config.data_dir.join("credentials.key"), true).unwrap();
        assert_eq!(
            vault
                .decrypt(&store.encrypted_credentials().unwrap().unwrap())
                .unwrap(),
            ("soul-user".into(), "soul-password".into())
        );
    }

    #[test]
    fn restore_recovers_across_a_database_key_swap_interruption_and_rejects_changed_staging() {
        let (_source, source, _) = fixture();
        let backup = export(&source, "backup-password", &Cancel::default()).unwrap();
        let (_target, target, _) = fixture();
        let prepared = preview(
            &backup.file(),
            "backup-password",
            &target,
            &Cancel::default(),
        )
        .unwrap();
        stage(&prepared, &target).unwrap();
        let pending = target.data_dir.join("restore-pending");
        let rollback = pending.join("rollback-ready");
        fs::create_dir(&rollback).unwrap();
        snapshot(
            &target.data_dir.join("rustysoul.sqlite3"),
            &rollback.join("rollback-state.sqlite3"),
            &Cancel::default(),
        )
        .unwrap();
        copy_private(
            &target.data_dir.join("credentials.key"),
            &rollback.join("rollback-credentials.key"),
        )
        .unwrap();
        atomic_copy(
            &pending.join("state.sqlite3"),
            &target.data_dir.join("rustysoul.sqlite3"),
        )
        .unwrap();
        assert!(apply_pending(&target).unwrap().is_some());
        assert_eq!(
            fs::read(source.data_dir.join("credentials.key")).unwrap(),
            fs::read(target.data_dir.join("credentials.key")).unwrap()
        );
        let prepared = preview(
            &backup.file(),
            "backup-password",
            &target,
            &Cancel::default(),
        )
        .unwrap();
        stage(&prepared, &target).unwrap();
        let original = digest(&target.data_dir.join("rustysoul.sqlite3")).unwrap();
        fs::write(
            target.data_dir.join("restore-pending/credentials.key"),
            [7; 32],
        )
        .unwrap();
        assert!(apply_pending(&target).unwrap().is_none());
        assert_eq!(
            digest(&target.data_dir.join("rustysoul.sqlite3")).unwrap(),
            original
        );
        assert_eq!(last_result(&target.data_dir).unwrap()["status"], "rejected");
    }
    #[test]
    fn injected_schema_objects_are_rejected_before_live_state_changes() {
        let (_temp, config, _) = fixture();
        let original = digest(&config.data_dir.join("credentials.key")).unwrap();
        let db = Connection::open(config.data_dir.join("rustysoul.sqlite3")).unwrap();
        db.execute_batch(
            "CREATE TRIGGER evil AFTER INSERT ON buddies BEGIN DELETE FROM credentials; END;",
        )
        .unwrap();
        drop(db);
        assert!(export(&config, "backup-password", &Cancel::default()).is_err());
        assert_eq!(
            digest(&config.data_dir.join("credentials.key")).unwrap(),
            original
        );
        assert!(!config.data_dir.join("restore-pending").exists());
    }
    #[test]
    fn a_live_daemon_lock_refuses_restore_and_cancel_is_observed() {
        let (_temp, config, _) = fixture();
        let stop = Cancel::default();
        let backup = export(&config, "backup-password", &stop).unwrap();
        let _held = lock(&config).unwrap();
        assert!(restore_file(&backup.file(), "backup-password", &config).is_err());
        stop.0.store(true, Ordering::Relaxed);
        assert!(export(&config, "backup-password", &stop).is_err());
    }
}
