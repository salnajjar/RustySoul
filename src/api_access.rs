//! Installation-scoped integration keys, separate from browser sessions and portable backups.
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    ReadOnly,
    Control,
}
impl Scope {
    pub fn allows(self, method: &str, path: &str) -> bool {
        let path = path.strip_prefix("/api/v1").unwrap_or(path);
        if matches!(method, "GET" | "HEAD") {
            return matches!(
                path,
                "/status"
                    | "/updates"
                    | "/messages"
                    | "/messages/conversations"
                    | "/metrics"
                    | "/events"
                    | "/logs"
                    | "/profile"
                    | "/traffic"
                    | "/folders"
                    | "/users/picture"
                    | "/downloads/file"
                    | "/browse/files"
                    | "/browse/tree"
                    | "/rooms/entries"
                    | "/transfers/{kind}"
                    | "/searches/{id}/results"
            );
        }
        self == Self::Control
            && method == "POST"
            && matches!(
                path,
                "/commands"
                    | "/downloads/batch"
                    | "/downloads/retry"
                    | "/profile"
                    | "/rooms/read"
                    | "/messages/read"
                    | "/updates/check"
            )
    }
}
#[derive(Clone, Serialize)]
pub struct Key {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub last_used_at: Option<u64>,
}
#[derive(Serialize)]
pub struct View {
    pub enabled: bool,
    pub keys: Vec<Key>,
    pub maximum_keys: usize,
}
#[derive(Serialize)]
pub struct Created {
    pub key: Key,
    pub token: String,
}
struct State {
    enabled: bool,
    keys: HashMap<[u8; 32], Key>,
    last_flush: Instant,
}
pub struct Manager {
    state: Mutex<State>,
    db: Mutex<Connection>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn digest(token: &str) -> [u8; 32] {
    // API keys contain 256 bits from the OS CSPRNG, not human passwords.
    // A fast digest is appropriate here; administrator passwords use Argon2id.
    Sha256::digest(token.as_bytes()).into()
}
impl Manager {
    pub fn open(path: &Path, owner: &str) -> Result<Self> {
        if path.exists() {
            crate::backup::regular(path)?;
        }
        let db = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        db.busy_timeout(Duration::from_secs(5))?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(
            version <= 1,
            "API access database was created by a newer RustySoul"
        );
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF; CREATE TABLE IF NOT EXISTS settings(id INTEGER PRIMARY KEY CHECK(id=1),owner TEXT NOT NULL,enabled INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS keys(hash BLOB PRIMARY KEY,id TEXT NOT NULL UNIQUE,name TEXT NOT NULL,scope TEXT NOT NULL,created_at INTEGER NOT NULL,expires_at INTEGER,last_used_at INTEGER); PRAGMA user_version=1;")?;
        let previous: Option<String> = db
            .query_row("SELECT owner FROM settings WHERE id=1", [], |r| r.get(0))
            .optional()?;
        if previous.as_deref() != Some(owner) {
            let tx = crate::store::write_transaction(&db)?;
            tx.execute("DELETE FROM keys", [])?;
            tx.execute("INSERT INTO settings VALUES(1,?1,0) ON CONFLICT(id) DO UPDATE SET owner=excluded.owner,enabled=0",[owner])?;
            tx.commit()?;
        }
        let enabled = db.query_row("SELECT enabled FROM settings WHERE id=1", [], |r| r.get(0))?;
        let mut keys = HashMap::new();
        {
            let mut stmt=db.prepare("SELECT hash,id,name,scope,created_at,expires_at,last_used_at FROM keys ORDER BY created_at DESC LIMIT 32")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let hash: Vec<u8> = r.get(0)?;
                let hash: [u8; 32] = hash
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Invalid stored API key hash"))?;
                let scope: String = r.get(3)?;
                keys.insert(
                    hash,
                    Key {
                        id: r.get(1)?,
                        name: r.get(2)?,
                        scope: serde_json::from_str(&scope)?,
                        created_at: r.get::<_, i64>(4)? as u64,
                        expires_at: r.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                        last_used_at: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                    },
                );
            }
        }
        Ok(Self {
            state: Mutex::new(State {
                enabled,
                keys,
                last_flush: Instant::now()
                    .checked_sub(Duration::from_secs(60))
                    .unwrap_or_else(Instant::now),
            }),
            db: Mutex::new(db),
        })
    }
    pub fn view(&self) -> View {
        let state = self.state.lock().unwrap();
        let mut keys: Vec<_> = state.keys.values().cloned().collect();
        keys.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.name.cmp(&b.name)));
        View {
            enabled: state.enabled,
            keys,
            maximum_keys: 32,
        }
    }
    pub fn set_enabled(&self, enabled: bool) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        self.db
            .lock()
            .unwrap()
            .execute("UPDATE settings SET enabled=?1 WHERE id=1", [enabled])?;
        state.enabled = enabled;
        Ok(())
    }
    pub fn create(&self, name: &str, scope: Scope, days: u16) -> Result<Created> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.len() <= 64 && !name.chars().any(char::is_control),
            "Use a name of 1..64 bytes without control characters"
        );
        ensure!(
            days <= 365,
            "Choose an expiry from 1 to 365 days, or 0 for no expiry"
        );
        let mut state = self.state.lock().unwrap();
        ensure!(
            state.enabled,
            "Enable external API access before creating a key"
        );
        ensure!(
            state.keys.len() < 32,
            "Revoke an unused key before creating another (maximum 32)"
        );
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("Random source unavailable"))?;
        let token = format!(
            "rsapi_{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let hash = digest(&token);
        let key = Key {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            scope,
            created_at: now(),
            expires_at: (days > 0).then(|| now() + days as u64 * 86400),
            last_used_at: None,
        };
        self.db.lock().unwrap().execute(
            "INSERT INTO keys VALUES(?1,?2,?3,?4,?5,?6,NULL)",
            params![
                hash.as_slice(),
                key.id,
                key.name,
                serde_json::to_string(&scope)?,
                key.created_at as i64,
                key.expires_at.map(|v| v as i64)
            ],
        )?;
        state.keys.insert(hash, key.clone());
        Ok(Created { key, token })
    }
    pub fn revoke(&self, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            uuid::Uuid::parse_str(id)?;
        }
        let mut state = self.state.lock().unwrap();
        let db = self.db.lock().unwrap();
        if let Some(id) = id {
            db.execute("DELETE FROM keys WHERE id=?1", [id])?;
            state.keys.retain(|_, key| key.id != id);
        } else {
            db.execute("DELETE FROM keys", [])?;
            state.keys.clear();
        }
        Ok(())
    }
    /// Pure memory authentication. At most one coalesced usage flush is requested per minute.
    pub fn authenticate(&self, token: &str, record: bool) -> Option<(Scope, bool)> {
        if token.len() != 70
            || !token.starts_with("rsapi_")
            || !token.as_bytes()[6..].iter().all(u8::is_ascii_hexdigit)
        {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        if !state.enabled {
            return None;
        }
        let key = state.keys.get_mut(&digest(token))?;
        if key.expires_at.is_some_and(|at| at <= now()) {
            return None;
        }
        let scope = key.scope;
        if record {
            key.last_used_at = Some(now());
        }
        let flush = record && state.last_flush.elapsed() >= Duration::from_secs(60);
        if flush {
            state.last_flush = Instant::now();
        }
        Some((scope, flush))
    }
    pub fn flush_usage(&self) -> Result<()> {
        let keys: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .keys
            .values()
            .map(|k| (k.id.clone(), k.last_used_at))
            .collect();
        let db = self.db.lock().unwrap();
        let tx = crate::store::write_transaction(&db)?;
        for (id, at) in keys {
            if let Some(at) = at {
                tx.execute(
                    "UPDATE keys SET last_used_at=MAX(COALESCE(last_used_at,0),?1) WHERE id=?2",
                    params![at as i64, id],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn reset_owner(&self, owner: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        let db = self.db.lock().unwrap();
        let tx = crate::store::write_transaction(&db)?;
        tx.execute("DELETE FROM keys", [])?;
        tx.execute("UPDATE settings SET owner=?1,enabled=0 WHERE id=1", [owner])?;
        tx.commit()?;
        state.keys.clear();
        state.enabled = false;
        Ok(())
    }
}
use rusqlite::OptionalExtension;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_are_hashed_scoped_revocable_and_survive_restart() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("keys.sqlite3");
        let m = Manager::open(&p, "owner").unwrap();
        assert!(!m.view().enabled);
        m.set_enabled(true).unwrap();
        let created = m.create("Integration", Scope::ReadOnly, 90).unwrap();
        assert_eq!(
            m.authenticate(&created.token, true).unwrap().0,
            Scope::ReadOnly
        );
        m.flush_usage().unwrap();
        drop(m);
        let m = Manager::open(&p, "owner").unwrap();
        assert!(m.authenticate(&created.token, false).is_some());
        assert!(m.view().keys[0].last_used_at.is_some());
        let db = Connection::open(&p).unwrap();
        let stored: Vec<u8> = db
            .query_row("SELECT hash FROM keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored.len(), 32);
        assert_ne!(stored, created.token.as_bytes());
        assert!(!m.view().keys[0].scope.allows("POST", "/commands"));
        assert!(!Scope::Control.allows("GET", "/api/v1/backups/id/file"));
        assert!(Scope::Control.allows("POST", "/api/v1/commands"));
        m.revoke(Some(&created.key.id)).unwrap();
        assert!(m.authenticate(&created.token, false).is_none());
    }
    #[test]
    fn disabled_expired_keys_and_changed_installation_are_rejected() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("keys.sqlite3");
        let m = Manager::open(&p, "owner").unwrap();
        m.set_enabled(true).unwrap();
        let key = m.create("key", Scope::Control, 0).unwrap();
        m.set_enabled(false).unwrap();
        assert!(m.authenticate(&key.token, false).is_none());
        m.set_enabled(true).unwrap();
        assert!(m.authenticate(&key.token, false).is_some());
        m.state
            .lock()
            .unwrap()
            .keys
            .values_mut()
            .next()
            .unwrap()
            .expires_at = Some(1);
        assert!(m.authenticate(&key.token, false).is_none());
        drop(m);
        let m = Manager::open(&p, "restored-owner").unwrap();
        assert!(m.view().keys.is_empty());
        assert!(!m.view().enabled);
    }
}
