use anyhow::{Context, Result, ensure};
use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString},
};
use rusqlite::{Connection, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

#[derive(Clone)]
struct Account {
    username: String,
    hash: String,
    must_change: bool,
    generation: i64,
}
struct Session {
    generation: i64,
    expires: u64,
}
#[derive(Clone, Serialize)]
pub struct AccountView {
    pub username: String,
    pub must_change: bool,
}
#[derive(Serialize)]
pub struct Login {
    pub token: String,
    pub account: AccountView,
}
pub struct Auth {
    account: Mutex<Account>,
    pub api_keys: crate::api_access::Manager,
    installation: String,
    sessions: Mutex<HashMap<[u8; 32], Session>>,
    db: Mutex<Connection>,
    update: Mutex<()>,
    pub revision: watch::Sender<u64>,
    rate: Mutex<(f64, Instant)>,
}
fn key_owner(account: &Account, installation: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{}\0{}\0{}\0{}",
                account.username, account.hash, account.generation, installation
            )
            .as_bytes()
        )
    )
}
pub const SESSION_SECONDS: u64 = 30 * 24 * 3600;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hash(password: &str) -> Result<String> {
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|_| anyhow::anyhow!("Random source unavailable"))?;
    Ok(Argon2::default()
        .hash_password(
            password.as_bytes(),
            &SaltString::encode_b64(&salt).map_err(|_| anyhow::anyhow!("Invalid salt"))?,
        )
        .map_err(|_| anyhow::anyhow!("Password hashing failed"))?
        .to_string())
}
fn verify(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).ok().is_some_and(|h| {
        Argon2::default()
            .verify_password(password.as_bytes(), &h)
            .is_ok()
    })
}
pub fn validate(username: &str, password: &str) -> Result<()> {
    ensure!(
        (3..=64).contains(&username.len())
            && username
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
        "Username must be 3..64 ASCII letters, numbers, dots, underscores or hyphens"
    );
    ensure!(
        password.chars().count() >= 8
            && password.len() <= 1024
            && !password.chars().any(char::is_control),
        "Password must have at least 8 characters, at most 1024 bytes, and no control characters"
    );
    Ok(())
}
pub(crate) const BACKUP_AUTH_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS admin_account(id INTEGER PRIMARY KEY CHECK(id=1),username TEXT NOT NULL,hash TEXT NOT NULL,must_change INTEGER NOT NULL,generation INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS admin_sessions(hash BLOB PRIMARY KEY,generation INTEGER NOT NULL,expires INTEGER NOT NULL);";
impl Auth {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA trusted_schema=OFF; CREATE TABLE IF NOT EXISTS admin_account(id INTEGER PRIMARY KEY CHECK(id=1),username TEXT NOT NULL,hash TEXT NOT NULL,must_change INTEGER NOT NULL,generation INTEGER NOT NULL);")?;
        let account = match db.query_row(
            "SELECT username,hash,must_change,generation FROM admin_account WHERE id=1",
            [],
            |r| {
                Ok(Account {
                    username: r.get(0)?,
                    hash: r.get(1)?,
                    must_change: r.get(2)?,
                    generation: r.get(3)?,
                })
            },
        ) {
            Ok(a) => a,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let username =
                    std::env::var("RUSTYSOUL_DEFAULT_USERNAME").unwrap_or_else(|_| "admin".into());
                let password = std::env::var("RUSTYSOUL_DEFAULT_PASSWORD")
                    .unwrap_or_else(|_| "change-me-now".into());
                validate(&username, &password)?;
                let a = Account {
                    username,
                    hash: hash(&password)?,
                    must_change: true,
                    generation: 1,
                };
                db.execute(
                    "INSERT INTO admin_account VALUES(1,?1,?2,1,1)",
                    params![a.username, a.hash],
                )?;
                a
            }
            Err(e) => return Err(e.into()),
        };
        db.execute_batch("CREATE TABLE IF NOT EXISTS admin_sessions(hash BLOB PRIMARY KEY,generation INTEGER NOT NULL,expires INTEGER NOT NULL);")?;
        db.execute(
            "DELETE FROM admin_sessions WHERE expires<=?1 OR generation<>?2",
            params![now() as i64, account.generation],
        )?;
        let mut sessions = HashMap::new();
        {
            let mut stmt = db.prepare(
                "SELECT hash,generation,expires FROM admin_sessions ORDER BY expires DESC LIMIT 32",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (hash, generation, expires) = row?;
                if let Ok(hash) = <[u8; 32]>::try_from(hash) {
                    sessions.insert(
                        hash,
                        Session {
                            generation,
                            expires: expires as u64,
                        },
                    );
                }
            }
        }
        let installation: String = db
            .query_row("SELECT identity FROM installation WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap_or_default();
        let api_keys = crate::api_access::Manager::open(
            &path.with_file_name("api-access.sqlite3"),
            &key_owner(&account, &installation),
        )?;
        let (revision, _) = watch::channel(0);
        Ok(Self {
            account: Mutex::new(account),
            api_keys,
            installation,
            sessions: Mutex::new(sessions),
            db: Mutex::new(db),
            update: Mutex::new(()),
            revision,
            rate: Mutex::new((10., Instant::now())),
        })
    }
    fn issue(&self, a: &Account) -> Result<Login> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("Random source unavailable"))?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, s| s.expires > now());
        ensure!(
            sessions.len() < 32,
            "Too many active sessions; log out or wait for expiry"
        );
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let expires = now() + SESSION_SECONDS;
        self.db.lock().unwrap().execute(
            "DELETE FROM admin_sessions WHERE expires<=?1",
            [now() as i64],
        )?;
        self.db.lock().unwrap().execute(
            "INSERT INTO admin_sessions VALUES(?1,?2,?3)",
            params![hash.as_slice(), a.generation, expires as i64],
        )?;
        sessions.insert(
            hash,
            Session {
                generation: a.generation,
                expires,
            },
        );
        Ok(Login {
            token,
            account: AccountView {
                username: a.username.clone(),
                must_change: a.must_change,
            },
        })
    }
    pub fn admit_login(&self) -> bool {
        let mut rate = self.rate.lock().unwrap();
        rate.0 = (rate.0 + rate.1.elapsed().as_secs_f64() / 6.).min(10.);
        rate.1 = Instant::now();
        if rate.0 < 1. {
            false
        } else {
            rate.0 -= 1.;
            true
        }
    }
    pub fn confirm<T>(&self, password: &str, action: impl FnOnce() -> Result<T>) -> Result<T> {
        ensure!(password.len() <= 1024, "Invalid current password");
        ensure!(
            self.admit_login(),
            "Too many password attempts; wait and try again"
        );
        let _update = self.update.lock().unwrap();
        let account = self.account.lock().unwrap().clone();
        ensure!(
            !account.must_change && verify(password, &account.hash),
            "Current administrator password is incorrect"
        );
        action()
    }
    pub fn login(&self, username: &str, password: &str) -> Result<Login> {
        ensure!(
            username.len() <= 64 && password.len() <= 1024,
            "Invalid username or password"
        );
        let a = self.account.lock().unwrap().clone();
        let valid_password = verify(password, &a.hash);
        ensure!(
            valid_password && username == a.username,
            "Invalid username or password"
        );
        let current = self.account.lock().unwrap();
        ensure!(
            current.generation == a.generation,
            "Account changed; sign in again"
        );
        self.issue(&current)
    }
    pub fn session(&self, token: &str) -> Option<AccountView> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let a = self.account.lock().unwrap();
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(&<[u8; 32]>::from(Sha256::digest(token.as_bytes())))
            .filter(|s| s.generation == a.generation && s.expires > now())
            .map(|_| AccountView {
                username: a.username.clone(),
                must_change: a.must_change,
            })
    }
    pub fn change(
        &self,
        token: &str,
        current_password: &str,
        username: &str,
        password: &str,
    ) -> Result<Login> {
        let _update = self.update.lock().unwrap();
        ensure!(self.session(token).is_some(), "Sign in again");
        validate(username, password)?;
        let previous = self.account.lock().unwrap().clone();
        ensure!(
            verify(current_password, &previous.hash),
            "Current password is incorrect"
        );
        if previous.must_change {
            ensure!(
                username != previous.username && !verify(password, &previous.hash),
                "Change both the deployment username and password"
            );
        }
        let next = Account {
            username: username.into(),
            hash: hash(password)?,
            must_change: false,
            generation: previous.generation + 1,
        };
        let mut a = self.account.lock().unwrap();
        self.api_keys
            .reset_owner(&key_owner(&next, &self.installation))?;
        {
            let mut db = self.db.lock().unwrap();
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("UPDATE admin_account SET username=?1,hash=?2,must_change=0,generation=?3 WHERE id=1",params![next.username,next.hash,next.generation]).context("Cannot save account")?;
            tx.execute("DELETE FROM admin_sessions", [])?;
            tx.commit()?;
        }
        *a = next;
        self.sessions.lock().unwrap().clear();
        self.revision.send_modify(|v| *v += 1);
        self.issue(&a)
    }
    pub fn logout(&self, token: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "DELETE FROM admin_sessions WHERE hash=?1",
            [Sha256::digest(token.as_bytes()).as_slice()],
        )?;
        self.sessions
            .lock()
            .unwrap()
            .remove(&<[u8; 32]>::from(Sha256::digest(token.as_bytes())));
        self.revision.send_modify(|v| *v += 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_restored_installation_identity_revokes_integration_keys() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("db");
        let store = crate::store::Store::open(&p).unwrap();
        store.identity().unwrap();
        let auth = Auth::open(&p).unwrap();
        let initial = auth.login("admin", "change-me-now").unwrap();
        auth.change(&initial.token, "change-me-now", "owner", "eightpwd")
            .unwrap();
        auth.api_keys.set_enabled(true).unwrap();
        let key = auth
            .api_keys
            .create("integration", crate::api_access::Scope::Control, 0)
            .unwrap();
        drop(auth);
        store
            .connection
            .execute(
                "UPDATE installation SET identity='restored-instance' WHERE id=1",
                [],
            )
            .unwrap();
        let restored = Auth::open(&p).unwrap();
        assert!(!restored.api_keys.view().enabled);
        assert!(restored.api_keys.authenticate(&key.token, false).is_none());
    }
    #[test]
    fn sessions_survive_restart_without_storing_tokens_and_revocations_are_durable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let auth = Auth::open(&path).unwrap();
        let first = auth.login("admin", "change-me-now").unwrap();
        let current = auth
            .change(&first.token, "change-me-now", "owner", "eightpwd")
            .unwrap();
        drop(auth);
        let auth = Auth::open(&path).unwrap();
        assert!(auth.session(&current.token).is_some());
        assert!(auth.session(&first.token).is_none());
        let second = auth.login("owner", "eightpwd").unwrap();
        auth.logout(&current.token).unwrap();
        drop(auth);
        let auth = Auth::open(&path).unwrap();
        assert!(auth.session(&current.token).is_none());
        assert!(auth.session(&second.token).is_some());
        let db = Connection::open(&path).unwrap();
        let stored: Vec<u8> = db
            .query_row("SELECT hash FROM admin_sessions", [], |r| r.get(0))
            .unwrap();
        assert_ne!(stored, second.token.as_bytes());
        assert_eq!(stored.len(), 32);
        db.execute("UPDATE admin_sessions SET expires=0", [])
            .unwrap();
        drop(auth);
        let auth = Auth::open(&path).unwrap();
        assert!(auth.session(&second.token).is_none());
    }
    #[test]
    fn first_login_rotation_persistence_and_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let auth = Auth::open(&path).unwrap();
        assert!(auth.login("admin", "incorrect").is_err());
        let first = auth.login("admin", "change-me-now").unwrap();
        assert!(first.account.must_change);
        assert!(
            auth.change(
                &first.token,
                "change-me-now",
                "admin",
                "new-secure-password"
            )
            .is_err()
        );
        assert!(
            auth.change(&first.token, "change-me-now", "newadmin", "change-me-now")
                .is_err()
        );
        let next = auth
            .change(
                &first.token,
                "change-me-now",
                "newadmin",
                "new-secure-password",
            )
            .unwrap();
        assert!(!next.account.must_change);
        assert!(auth.session(&first.token).is_none());
        auth.logout(&next.token).unwrap();
        assert!(auth.session(&next.token).is_none());
        drop(auth);
        let auth = Auth::open(&path).unwrap();
        assert!(auth.login("admin", "change-me-now").is_err());
        assert!(auth.login("newadmin", "new-secure-password").is_ok());
        let text = std::fs::read(&path).unwrap();
        assert!(!text.windows(19).any(|s| s == b"new-secure-password"));
    }
}
