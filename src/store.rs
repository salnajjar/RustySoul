//! The daemon is the sole SQLite writer. Only durable state is stored; transfer ticks stay in memory.
use crate::model::*;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedDownload {
    pub directory: String,
    pub local_path: String,
    pub username: String,
    pub path: String,
    pub size: u64,
    pub status: String,
}

pub struct Store {
    pub(crate) connection: Connection,
    searches: std::collections::BTreeMap<String, String>,
    rooms: String,
    messages: String,
    browse: String,
    download_cache: Option<
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, SavedDownload>>,
    >,
}

/// Acquire the writer slot before reading within a write transaction. WAL read-to-write
/// upgrades cannot wait when another connection has committed after the read snapshot.
pub fn write_transaction(db: &Connection) -> rusqlite::Result<rusqlite::Transaction<'_>> {
    rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
}

fn read_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_at_schema(path, 18)
    }
    // Used only to construct a trusted expected schema when validating older
    // backups. Never called on an untrusted backup before exact validation.
    pub(crate) fn open_at_schema(path: &Path, target: u32) -> Result<Self> {
        ensure!((11..=18).contains(&target), "Unsupported schema target");
        let connection = Connection::open(path).context("Cannot open SQLite database")?;
        #[cfg(unix)]
        if path != Path::new(":memory:") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        ensure!(
            version <= target,
            "Database was created by a newer RustySoul; refusing to downgrade"
        );
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE downloads(username TEXT NOT NULL, path TEXT NOT NULL, size INTEGER NOT NULL CHECK(size>0), status TEXT NOT NULL, PRIMARY KEY(username,path));
                CREATE TABLE searches(id TEXT PRIMARY KEY, query TEXT NOT NULL, collecting INTEGER NOT NULL);
                CREATE TABLE search_results(search_id TEXT NOT NULL REFERENCES searches(id) ON DELETE CASCADE, username TEXT NOT NULL, path TEXT NOT NULL, size INTEGER NOT NULL, free_slots INTEGER NOT NULL, speed INTEGER NOT NULL, attributes TEXT NOT NULL, PRIMARY KEY(search_id,username,path));
                CREATE INDEX results_path ON search_results(search_id,path);
                CREATE TABLE rooms(name TEXT PRIMARY KEY, users INTEGER NOT NULL, joined INTEGER NOT NULL);
                CREATE TABLE messages(id INTEGER PRIMARY KEY, target TEXT NOT NULL, username TEXT NOT NULL, text TEXT NOT NULL, private INTEGER NOT NULL, timestamp INTEGER NOT NULL);
                CREATE INDEX messages_conversation ON messages(private,target,id);
                CREATE TABLE local_files(path TEXT PRIMARY KEY, size INTEGER NOT NULL, attributes TEXT NOT NULL);
                CREATE TABLE browse(username TEXT PRIMARY KEY, files TEXT NOT NULL, truncated INTEGER NOT NULL);
                PRAGMA user_version=1;
                COMMIT;")?;
        }
        if version < 2 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE credentials(id INTEGER PRIMARY KEY CHECK(id=1), encrypted BLOB NOT NULL); PRAGMA user_version=2; COMMIT;")?;
        }
        if version < 3 {
            connection.execute_batch("BEGIN IMMEDIATE;
                ALTER TABLE downloads ADD COLUMN directory TEXT NOT NULL DEFAULT '';
                CREATE TABLE file_locations(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);
                CREATE TABLE library_cache(real_path TEXT PRIMARY KEY,virtual_path TEXT NOT NULL,size INTEGER NOT NULL,modified TEXT NOT NULL,changed TEXT NOT NULL,attributes TEXT NOT NULL);
                PRAGMA user_version=3;COMMIT;")?;
        }
        if version < 4 {
            connection.execute_batch("BEGIN IMMEDIATE; ALTER TABLE rooms ADD COLUMN last_activity INTEGER; CREATE TABLE connection_settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL); CREATE TABLE installation(id INTEGER PRIMARY KEY CHECK(id=1),identity TEXT NOT NULL); PRAGMA user_version=4;COMMIT;")?;
        }
        if version < 5 {
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE browse_files(username TEXT NOT NULL,path TEXT NOT NULL,folder TEXT NOT NULL,size INTEGER NOT NULL,attributes TEXT NOT NULL,PRIMARY KEY(username,path));
                CREATE INDEX browse_folder ON browse_files(username,folder,path COLLATE NOCASE,path);
                CREATE INDEX browse_path ON browse_files(username,path COLLATE NOCASE,path);
                CREATE TABLE buddies(username TEXT PRIMARY KEY);
                CREATE TABLE library_settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);
                PRAGMA user_version=5;COMMIT;")?;
        }
        if version < 6 {
            connection.execute_batch("BEGIN IMMEDIATE;
                ALTER TABLE search_results ADD COLUMN folder TEXT NOT NULL DEFAULT '';
                ALTER TABLE search_results ADD COLUMN format TEXT NOT NULL DEFAULT 'unknown';
                ALTER TABLE search_results ADD COLUMN search_key TEXT NOT NULL DEFAULT '';
                ALTER TABLE browse_files ADD COLUMN search_key TEXT NOT NULL DEFAULT '';
                CREATE INDEX result_user ON search_results(search_id,username,folder COLLATE NOCASE,folder,path COLLATE NOCASE,path);
                CREATE TABLE search_user_stats(search_id TEXT NOT NULL REFERENCES searches(id) ON DELETE CASCADE,username TEXT NOT NULL,matches INTEGER NOT NULL,folders INTEGER NOT NULL,speed INTEGER NOT NULL,slots INTEGER NOT NULL,PRIMARY KEY(search_id,username));
                CREATE INDEX result_path_order ON search_results(search_id,path COLLATE NOCASE,path);
                CREATE TABLE search_options(search_id TEXT PRIMARY KEY REFERENCES searches(id) ON DELETE CASCADE,scope TEXT NOT NULL,wishlist_id TEXT);
                CREATE TABLE browse_folders(username TEXT NOT NULL,path TEXT NOT NULL,parent TEXT NOT NULL,name TEXT NOT NULL,files INTEGER NOT NULL,direct_files INTEGER NOT NULL,search_key TEXT NOT NULL,PRIMARY KEY(username,path));
                ALTER TABLE downloads ADD COLUMN file_name TEXT NOT NULL DEFAULT '';
                ALTER TABLE downloads ADD COLUMN parent_folder TEXT NOT NULL DEFAULT '';
                ALTER TABLE downloads ADD COLUMN transferred INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE downloads ADD COLUMN speed REAL NOT NULL DEFAULT 0;
                ALTER TABLE downloads ADD COLUMN queue_position INTEGER;
                ALTER TABLE downloads ADD COLUMN added_at INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE downloads ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
                CREATE TABLE uploads(username TEXT NOT NULL,path TEXT NOT NULL,size INTEGER NOT NULL,transferred INTEGER NOT NULL,speed REAL NOT NULL,status TEXT NOT NULL,queue_position INTEGER,added_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,file_name TEXT NOT NULL DEFAULT '',parent_folder TEXT NOT NULL DEFAULT '',PRIMARY KEY(username,path));
                CREATE INDEX uploads_activity ON uploads(updated_at DESC);
                CREATE TABLE retry_policy(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);
                CREATE TABLE download_retries(username TEXT NOT NULL,path TEXT NOT NULL,attempts INTEGER NOT NULL,next_at INTEGER,PRIMARY KEY(username,path));
                CREATE TABLE user_rules(username TEXT PRIMARY KEY COLLATE NOCASE,ignored INTEGER NOT NULL,banned INTEGER NOT NULL);
                CREATE TABLE wishlist(id TEXT PRIMARY KEY,query TEXT NOT NULL UNIQUE,enabled INTEGER NOT NULL,last_run INTEGER,search_id TEXT REFERENCES searches(id) ON DELETE SET NULL,last_count INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE wishlist_clock(id INTEGER PRIMARY KEY CHECK(id=1),last_sent INTEGER NOT NULL);
                INSERT INTO wishlist_clock VALUES(1,0);
                CREATE INDEX browse_children ON browse_folders(username,parent,name COLLATE NOCASE,name);
                PRAGMA user_version=6;COMMIT;")?;
        }
        if version < 7 {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE own_profile(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);
            CREATE TABLE traffic_users(username TEXT PRIMARY KEY COLLATE NOCASE,downloaded INTEGER NOT NULL,uploaded INTEGER NOT NULL,first_at INTEGER NOT NULL,last_at INTEGER NOT NULL);
            CREATE TABLE traffic_user_clock(id INTEGER PRIMARY KEY CHECK(id=1),since INTEGER NOT NULL);
            INSERT INTO traffic_user_clock VALUES(1,strftime('%s','now'));
            PRAGMA user_version=7;COMMIT;")?;
        }
        if version < 8 {
            connection.execute_batch("BEGIN IMMEDIATE;
            ALTER TABLE downloads ADD COLUMN local_path TEXT NOT NULL DEFAULT '';
            ALTER TABLE uploads ADD COLUMN detail TEXT NOT NULL DEFAULT '';
            CREATE TABLE user_pictures(username TEXT PRIMARY KEY,data BLOB NOT NULL,revision TEXT NOT NULL,updated_at INTEGER NOT NULL);
            CREATE TABLE sharing_policy(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);
            CREATE TABLE room_subscriptions(room TEXT PRIMARY KEY);
            INSERT INTO room_subscriptions SELECT name FROM rooms WHERE joined=1;
            CREATE TABLE room_reads(room TEXT PRIMARY KEY,last_id INTEGER NOT NULL);
            INSERT INTO room_reads SELECT target,MAX(id) FROM messages WHERE private=0 GROUP BY target;
            CREATE TABLE message_clock(id INTEGER PRIMARY KEY CHECK(id=1),next_id INTEGER NOT NULL);
            INSERT INTO message_clock SELECT 1,COALESCE(MAX(id),0)+1 FROM messages;
            PRAGMA user_version=8;COMMIT;")?;
        }
        if version < 9 {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE activity_log(id INTEGER PRIMARY KEY,at INTEGER NOT NULL,level TEXT NOT NULL,source TEXT NOT NULL,message TEXT NOT NULL);
            CREATE INDEX activity_log_level ON activity_log(level,id);
            ALTER TABLE browse_folders ADD COLUMN subfolders INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE browse_folders ADD COLUMN direct_folders INTEGER NOT NULL DEFAULT 0;
            DELETE FROM browse_folders;
            PRAGMA user_version=9;COMMIT;")?;
        }
        if version < 10 {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE discovery_cache(account TEXT NOT NULL,scope TEXT NOT NULL,data TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(account,scope));
            CREATE TABLE user_interests_cache(username TEXT PRIMARY KEY COLLATE NOCASE,data TEXT NOT NULL,updated_at INTEGER NOT NULL);
            PRAGMA user_version=10;COMMIT;")?;
        }
        if version < 11 {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE room_controls(account TEXT PRIMARY KEY,accept_invitations INTEGER NOT NULL);
            CREATE TABLE room_control_cache(account TEXT NOT NULL,room TEXT NOT NULL,data TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(account,room));
            CREATE TABLE room_control_entries(account TEXT NOT NULL,room TEXT NOT NULL,kind TEXT NOT NULL,username TEXT NOT NULL COLLATE NOCASE,text TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(account,room,kind,username));
            CREATE TABLE room_preferences(account TEXT NOT NULL,room TEXT NOT NULL,ticker TEXT NOT NULL,PRIMARY KEY(account,room));
            CREATE TABLE private_room_subscriptions(account TEXT NOT NULL,room TEXT NOT NULL,PRIMARY KEY(account,room));
            PRAGMA user_version=11;COMMIT;")?;
        }
        crate::transfers::migrate_names(&connection)?;
        crate::traffic::initialize(&connection)?;
        crate::search::migrate_keys(&connection)?;
        crate::search::ensure_stats(&connection)?;
        crate::browse::migrate_keys(&connection)?;
        if version < 12 && target >= 12 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE transfer_queue(direction TEXT NOT NULL CHECK(direction IN ('downloads','uploads')),username TEXT NOT NULL,path TEXT NOT NULL,priority INTEGER NOT NULL CHECK(priority BETWEEN -1 AND 1),position INTEGER NOT NULL CHECK(position>=0),PRIMARY KEY(direction,username,path)); PRAGMA user_version=12; COMMIT;")?;
        }
        if version < 13 && target >= 13 {
            // Preserve rowid: it is the default queue's arrival order. No
            // external table references downloads; its PK is the only index.
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE downloads_v13(username TEXT NOT NULL,path TEXT NOT NULL,size INTEGER NOT NULL CHECK(size>=0),status TEXT NOT NULL,directory TEXT NOT NULL DEFAULT '',file_name TEXT NOT NULL DEFAULT '',parent_folder TEXT NOT NULL DEFAULT '',transferred INTEGER NOT NULL DEFAULT 0,speed REAL NOT NULL DEFAULT 0,queue_position INTEGER,added_at INTEGER NOT NULL DEFAULT 0,updated_at INTEGER NOT NULL DEFAULT 0,local_path TEXT NOT NULL DEFAULT '',PRIMARY KEY(username,path));
                INSERT INTO downloads_v13(rowid,username,path,size,status,directory,file_name,parent_folder,transferred,speed,queue_position,added_at,updated_at,local_path) SELECT rowid,username,path,size,status,directory,file_name,parent_folder,transferred,speed,queue_position,added_at,updated_at,local_path FROM downloads;
                DROP TABLE downloads;
                ALTER TABLE downloads_v13 RENAME TO downloads;
                PRAGMA user_version=13;COMMIT;")?;
        }
        if version < 14 && target >= 14 {
            crate::transfer_timing::migrate(&connection)?;
        }
        if version < 15 && target >= 15 {
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE private_reads(username TEXT PRIMARY KEY COLLATE NOCASE,last_id INTEGER NOT NULL);
                INSERT INTO private_reads SELECT lower(target),MAX(id) FROM messages WHERE private=1 GROUP BY target COLLATE NOCASE;
                PRAGMA user_version=15;COMMIT;")?;
        }
        if version < 16 && target >= 16 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE upload_speed_estimate(id INTEGER PRIMARY KEY CHECK(id=1),rate REAL NOT NULL CHECK(rate>0)); PRAGMA user_version=16; COMMIT;")?;
        }
        if version < 17 && target >= 17 {
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE upload_speed_users(username TEXT PRIMARY KEY COLLATE NOCASE,payload_bytes INTEGER NOT NULL CHECK(payload_bytes>0),active_seconds REAL NOT NULL CHECK(active_seconds>0),last_sequence INTEGER NOT NULL CHECK(last_sequence>0));
                CREATE INDEX upload_speed_users_recent ON upload_speed_users(last_sequence DESC);
                DROP TABLE upload_speed_estimate;
                PRAGMA user_version=17; COMMIT;")?;
        }
        if version < 18 && target >= 18 {
            connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE upload_retries(username TEXT NOT NULL,path TEXT NOT NULL,attempts INTEGER NOT NULL CHECK(attempts BETWEEN 0 AND 10),next_at INTEGER,expires_at INTEGER NOT NULL,manual INTEGER NOT NULL CHECK(manual IN (0,1)),PRIMARY KEY(username,path),FOREIGN KEY(username,path) REFERENCES uploads(username,path) ON DELETE CASCADE);
                PRAGMA user_version=18; COMMIT;")?;
        }
        Ok(Self {
            connection,
            searches: Default::default(),
            rooms: String::new(),
            messages: String::new(),
            browse: String::new(),
            download_cache: None,
        })
    }
    pub fn network(&self) -> Result<Option<NetworkSettings>> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row("SELECT json FROM connection_settings WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn library_settings(&self) -> Result<LibrarySettings> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row("SELECT json FROM library_settings WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .unwrap_or_else(|| Ok(LibrarySettings::default()))
    }
    pub fn save_library_settings(&self, settings: &LibrarySettings) -> Result<()> {
        self.connection.execute("INSERT INTO library_settings VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(settings)?])?;
        Ok(())
    }
    pub fn buddies(&self) -> Result<Vec<String>> {
        Ok(self
            .connection
            .prepare("SELECT username FROM buddies ORDER BY username COLLATE NOCASE,username")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn set_buddy(&self, username: &str, buddy: bool) -> Result<()> {
        if buddy {
            self.connection
                .execute("INSERT OR IGNORE INTO buddies VALUES(?1)", [username])?;
        } else {
            self.connection
                .execute("DELETE FROM buddies WHERE username=?1", [username])?;
        }
        Ok(())
    }
    pub fn save_network(&self, settings: &NetworkSettings) -> Result<()> {
        self.connection.execute("INSERT INTO connection_settings VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(settings)?])?;
        Ok(())
    }
    pub fn identity(&self) -> Result<String> {
        self.connection.execute(
            "INSERT OR IGNORE INTO installation VALUES(1,?1)",
            [uuid::Uuid::new_v4().to_string()],
        )?;
        Ok(self
            .connection
            .query_row("SELECT identity FROM installation WHERE id=1", [], |r| {
                r.get(0)
            })?)
    }

    pub fn encrypted_credentials(&self) -> Result<Option<Vec<u8>>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .connection
            .query_row("SELECT encrypted FROM credentials WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn save_credentials(&self, encrypted: &[u8]) -> Result<()> {
        self.connection.execute("INSERT INTO credentials VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET encrypted=excluded.encrypted", [encrypted])?;
        Ok(())
    }

    pub fn downloads(&self) -> Result<Vec<SavedDownload>> {
        Ok(self
            .connection
            .prepare("SELECT username,path,size,status,directory,local_path FROM downloads ORDER BY rowid")?
            .query_map([], |r| {
                Ok(SavedDownload {
                    directory: r.get(4)?,local_path:r.get(5)?,
                    username: r.get(0)?,
                    path: r.get(1)?,
                    size: read_u64(r, 2)?,
                    status: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// Reset only the chosen failed records and their automatic timers together.
    pub(crate) fn retry_downloads(&mut self, keys: &[(String, String)]) -> Result<()> {
        let tx = write_transaction(&self.connection)?;
        for (user, path) in keys {
            tx.execute("UPDATE downloads SET status='waiting',speed=0,queue_position=NULL,updated_at=strftime('%s','now') WHERE username=?1 AND path=?2",params![user,path])?;
            tx.execute(
                "DELETE FROM download_retries WHERE username=?1 AND path=?2",
                params![user, path],
            )?;
        }
        tx.commit()?;
        self.download_cache = None;
        Ok(())
    }
    pub fn save_downloads(&mut self, downloads: &[SavedDownload]) -> Result<()> {
        if self.download_cache.is_none() {
            let mut cache = std::collections::BTreeMap::<
                String,
                std::collections::BTreeMap<String, SavedDownload>,
            >::new();
            for row in self.downloads()? {
                cache
                    .entry(row.username.clone())
                    .or_default()
                    .insert(row.path.clone(), row);
            }
            self.download_cache = Some(cache);
        }
        let cache = self.download_cache.as_ref().unwrap();
        let present: std::collections::HashSet<_> = downloads
            .iter()
            .map(|d| (d.username.as_str(), d.path.as_str()))
            .collect();
        let changed: Vec<_> = downloads
            .iter()
            .filter(|d| cache.get(&d.username).and_then(|m| m.get(&d.path)) != Some(*d))
            .collect();
        let removed: Vec<_> = cache
            .iter()
            .flat_map(|(u, files)| {
                files
                    .keys()
                    .filter(|p| !present.contains(&(u.as_str(), p.as_str())))
                    .map(|p| (u.clone(), p.clone()))
            })
            .collect();
        if changed.is_empty() && removed.is_empty() {
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (u, p) in &removed {
            tx.execute(
                "DELETE FROM downloads WHERE username=?1 AND path=?2",
                params![u, p],
            )?;
        }
        for d in &changed {
            tx.execute("INSERT INTO downloads(username,path,size,status,directory,local_path,added_at,updated_at,file_name,parent_folder) VALUES(?1,?2,?3,?4,?5,?8,strftime('%s','now'),strftime('%s','now'),?6,?7) ON CONFLICT(username,path) DO UPDATE SET size=excluded.size,status=excluded.status,directory=excluded.directory,local_path=excluded.local_path,updated_at=excluded.updated_at,file_name=excluded.file_name,parent_folder=excluded.parent_folder",params![d.username,d.path,i64::try_from(d.size)?,d.status,d.directory,crate::transfers::names(&d.path).0,crate::transfers::names(&d.path).1,d.local_path])?;
        }
        tx.commit()?;
        let cache = self.download_cache.as_mut().unwrap();
        for (u, p) in removed {
            if let Some(m) = cache.get_mut(&u) {
                m.remove(&p);
            }
        }
        for d in changed {
            cache
                .entry(d.username.clone())
                .or_default()
                .insert(d.path.clone(), d.clone());
        }
        Ok(())
    }

    pub fn locations(&self) -> Result<Option<FileLocations>> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row("SELECT json FROM file_locations WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn save_locations(&self, locations: &FileLocations) -> Result<()> {
        self.connection.execute("INSERT INTO file_locations VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(locations)?])?;
        Ok(())
    }

    pub fn restore(&self, view: &mut Snapshot) -> Result<()> {
        let mut stmt = self
            .connection
            .prepare("SELECT id,query FROM searches WHERE NOT EXISTS(SELECT 1 FROM search_options o WHERE o.search_id=searches.id AND o.wishlist_id IS NOT NULL) ORDER BY rowid")?;
        let searches =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for search in searches {
            let (id, query) = search?;
            let total = self.connection.query_row(
                "SELECT COALESCE(SUM(matches),0) FROM search_user_stats WHERE search_id=?1",
                [&id],
                |r| r.get::<_, i64>(0),
            )? as usize;
            let scope = crate::search::scope(&self.connection, &id)?;
            view.searches.push(SearchView {
                id,
                query,
                collecting: false,
                total,
                scope,
                ..Default::default()
            });
        }

        let recent = view
            .searches
            .len()
            .saturating_sub(crate::search::SNAPSHOT_PREVIEWS);
        for search in &mut view.searches[recent..] {
            search.results = crate::search::preview(&self.connection, &search.id)?;
        }

        let mut stmt = self
            .connection
            .prepare("SELECT name,users,last_activity FROM rooms LIMIT 1000")?;
        view.rooms = stmt
            .query_map([], |r| {
                Ok(RoomView {
                    name: r.get(0)?,
                    users: r.get(1)?,
                    last_activity: r.get::<_, Option<i64>>(2)?.and_then(|v| v.try_into().ok()),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        // Live membership resets; persisted room subscriptions are rejoined after login.
        let mut stmt = self.connection.prepare(
            "SELECT target,username,text,private,timestamp,id FROM messages ORDER BY id DESC LIMIT 500",
        )?;
        view.messages = stmt
            .query_map([], |r| {
                Ok(MessageView {
                    id: read_u64(r, 5)?,
                    target: r.get(0)?,
                    username: r.get(1)?,
                    text: r.get(2)?,
                    private: r.get(3)?,
                    timestamp: read_u64(r, 4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        view.messages.reverse();
        let mut stmt = self
            .connection
            .prepare("SELECT username,files,truncated FROM browse ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?;
        for row in rows {
            let (username, files, truncated) = row?;
            let files: Vec<FileView> = serde_json::from_str(&files)?;
            let total = self.connection.query_row(
                "SELECT COUNT(*) FROM browse_files WHERE username=?1",
                [&username],
                |r| r.get::<_, u32>(0),
            )? as usize;
            let listing = if total == 0 && !files.is_empty() {
                let mut v = crate::browse::save(&self.connection, &username, files)?;
                v.truncated = truncated;
                v
            } else {
                BrowseView {
                    files,
                    total,
                    truncated,
                    ..Default::default()
                }
            };
            crate::browse::ensure_tree(&self.connection, &username)?;
            view.browse.insert(username, listing);
        }
        Ok(())
    }

    /// Called at a limited cadence; each section is committed only when its content changed.
    pub fn save_snapshot(&mut self, view: &Snapshot) -> Result<()> {
        let searches = view
            .searches
            .iter()
            .map(|s| Ok((s.id.clone(), serde_json::to_string(s)?)))
            .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
        if searches != self.searches {
            let tx = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let existing = tx
                .prepare("SELECT id FROM searches")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for id in existing {
                if !searches.contains_key(&id) && crate::search::wishlist_id(&tx, &id)?.is_none() {
                    tx.execute("DELETE FROM searches WHERE id=?1", [id])?;
                }
            }
            for search in &view.searches {
                if self.searches.get(&search.id) == searches.get(&search.id) {
                    continue;
                }
                crate::search::create(&tx, search)?;
                // Previews are bounded. Never replace a larger normalized result set.
                let has_results = search.results.is_empty()
                    || tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM search_results WHERE search_id=?1)",
                        [&search.id],
                        |r| r.get::<_, bool>(0),
                    )?;
                if !has_results {
                    for f in &search.results {
                        tx.execute("INSERT OR IGNORE INTO search_results(search_id,username,path,size,free_slots,speed,attributes,folder,format,search_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![search.id,f.username,f.path,i64::try_from(f.size)?,f.free_slots,f.speed,serde_json::to_string(&f.attributes)?,crate::search::folder(&f.path),crate::search::format(&f.path),format!("{} {}",f.username,f.path).to_lowercase()])?;
                    }
                    crate::search::update_stats(&tx, &search.id, None)?;
                }
            }
            tx.commit()?;
            self.searches = searches;
        }
        let rooms = serde_json::to_string(&(&view.rooms, &view.joined_rooms))?;
        if rooms != self.rooms {
            let tx = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("DELETE FROM rooms", [])?;
            for r in &view.rooms {
                tx.execute(
                    "INSERT INTO rooms VALUES(?1,?2,?3,?4)",
                    params![
                        r.name,
                        r.users,
                        view.joined_rooms.contains(&r.name),
                        r.last_activity.map(i64::try_from).transpose()?
                    ],
                )?;
            }
            tx.commit()?;
            self.rooms = rooms;
        }
        self.save_messages(&view.messages)?;
        let browse = serde_json::to_string(&view.browse)?;
        if browse != self.browse {
            let tx = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("DELETE FROM browse", [])?;
            for (username, listing) in &view.browse {
                tx.execute(
                    "INSERT INTO browse VALUES(?1,?2,?3)",
                    params![
                        username,
                        serde_json::to_string(&listing.files)?,
                        listing.truncated
                    ],
                )?;
            }
            tx.commit()?;
            self.browse = browse;
        }
        Ok(())
    }
    pub fn save_messages(&mut self, view: &[MessageView]) -> Result<bool> {
        let messages = serde_json::to_string(view)?;
        if messages == self.messages {
            return Ok(false);
        }
        let tx = crate::store::write_transaction(&self.connection)?;
        let mut added = 0;
        for m in view {
            let id = if m.id > 0 {
                i64::try_from(m.id)?
            } else {
                tx.query_row(
                    "UPDATE message_clock SET next_id=next_id+1 WHERE id=1 RETURNING next_id-1",
                    [],
                    |r| r.get::<_, i64>(0),
                )?
            };
            added += tx.execute("INSERT INTO messages(id,target,username,text,private,timestamp) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO NOTHING", params![id,m.target,m.username,m.text,m.private,i64::try_from(m.timestamp)?])?;
        }
        tx.execute("UPDATE message_clock SET next_id=MAX(next_id,COALESCE((SELECT MAX(id)+1 FROM messages),1)) WHERE id=1",[])?;
        tx.commit()?;
        self.messages = messages;
        Ok(added > 0)
    }
    pub fn clear_message_history(
        &mut self,
        target: &str,
        private: bool,
    ) -> Result<(usize, (u64, u64))> {
        self.clear_message_history_filtered(target, private, &crate::messages::DeleteRange::All, 0)
    }
    pub fn clear_message_history_filtered(
        &mut self,
        target: &str,
        private: bool,
        range: &crate::messages::DeleteRange,
        now: u64,
    ) -> Result<(usize, (u64, u64))> {
        crate::messages::validate(target, private)?;
        let (from, to) = range.bounds(now)?;
        let collation = if private { "COLLATE NOCASE" } else { "" };
        let tx = crate::store::write_transaction(&self.connection)?;
        let count = tx.execute(
            &format!("DELETE FROM messages WHERE private=?1 AND target=?2 {collation} AND (?3 IS NULL OR timestamp>=?3) AND (?4 IS NULL OR timestamp<?4)"),
            params![private, target,from.map(|v|v as i64),to.map(|v|v as i64)],
        )?;
        let start = tx.query_row(
            "UPDATE message_clock SET next_id=next_id+1024 WHERE id=1 RETURNING next_id-1024",
            [],
            |r| r.get::<_, i64>(0),
        )? as u64;
        tx.commit()?;
        self.messages.clear();
        Ok((count, (start, start + 1024)))
    }
    pub fn index_library(&mut self, directories: &[soulseek_rs::SharedDirectory]) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM local_files", [])?;
        {
            let mut insert =
                tx.prepare_cached("INSERT OR REPLACE INTO local_files VALUES(?1,?2,?3)")?;
            for dir in directories {
                for f in &dir.files {
                    insert.execute(params![
                        format!("{}\\{}", dir.name, f.name),
                        i64::try_from(f.size)?,
                        serde_json::to_string(&f.attributes)?
                    ])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn reserve_message_ids(&self) -> Result<(u64, u64)> {
        let start = self.connection.query_row(
            "UPDATE message_clock SET next_id=next_id+1024 WHERE id=1 RETURNING next_id-1024",
            [],
            |r| r.get::<_, i64>(0),
        )? as u64;
        Ok((start, start + 1024))
    }
    pub fn checkpoint(&self) -> Result<()> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_timer_write_failure_rolls_back_all_selected_status_and_progress_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("db")).unwrap();
        let files = ["a", "b"].map(|p| SavedDownload {
            username: "alice".into(),
            path: p.into(),
            size: 10,
            status: "failed".into(),
            directory: dir.path().to_string_lossy().into_owned(),
            local_path: String::new(),
        });
        store.save_downloads(&files).unwrap();
        store.connection.execute_batch("UPDATE downloads SET transferred=4;INSERT INTO download_retries VALUES('alice','b',2,2000000000);CREATE TRIGGER refuse_retry_timer BEFORE DELETE ON download_retries BEGIN SELECT RAISE(ABORT,'injected timer failure');END;").unwrap();
        assert!(
            store
                .retry_downloads(&[("alice".into(), "a".into()), ("alice".into(), "b".into())])
                .is_err()
        );
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM downloads WHERE status='failed' AND transferred=4",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT attempts FROM download_retries", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    #[test]
    fn schema_thirteen_keeps_arrival_ids_history_and_queue_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        let old = Store::open_at_schema(&path, 12).unwrap();
        let db = &old.connection;
        db.execute_batch("INSERT INTO downloads(rowid,username,path,size,status,directory,file_name,parent_folder,transferred,speed,queue_position,added_at,updated_at,local_path) VALUES(11,'alice','Music/a.flac',100,'paused','/downloads','a.flac','Music',45,0,7,100,110,''),(71,'bob','Music/b.flac',200,'completed','/downloads','b.flac','Music',200,0,NULL,200,220,'bob/Music/b.flac'); INSERT INTO transfer_queue VALUES('downloads','alice','Music/a.flac',1,0); INSERT INTO download_retries VALUES('alice','Music/a.flac',2,123);").unwrap();
        let snapshot = |db: &Connection| {
            db.prepare("SELECT rowid,username,path,size,status,directory,file_name,parent_folder,transferred,speed,queue_position,added_at,updated_at,local_path FROM downloads ORDER BY rowid")
                .unwrap()
                .query_map([], |r| {
                    (0..r.as_ref().column_count())
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let before = snapshot(db);
        drop(old);
        let upgraded = Store::open(&path).unwrap();
        let db = &upgraded.connection;
        assert_eq!(snapshot(db), before);
        assert_eq!(
            db.query_row("SELECT priority FROM transfer_queue", [], |r| r
                .get::<_, i8>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT attempts FROM download_retries", [], |r| r
                .get::<_, i32>(0))
                .unwrap(),
            2
        );
        db.execute("INSERT INTO downloads(username,path,size,status) VALUES('peer','empty.txt',0,'waiting')",[]).unwrap();
        assert!(db.execute("INSERT INTO downloads(username,path,size,status) VALUES('peer','negative.txt',-1,'waiting')",[]).is_err());
        assert_eq!(
            db.query_row(
                "SELECT rowid FROM downloads WHERE path='empty.txt'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            72
        );
        assert_eq!(
            db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert!(
            !db.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        drop(upgraded);
        assert_eq!(Store::open(&path).unwrap().downloads().unwrap().len(), 3);
    }

    #[test]
    fn concurrent_wal_writers_wait_before_reading_their_write_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        Store::open(&path).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut threads = vec![];
        for _ in 0..3 {
            let path = path.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                let db = Connection::open(path).unwrap();
                db.busy_timeout(Duration::from_secs(5)).unwrap();
                // Match Store::open's WAL/NORMAL policy. Raw connections default
                // to FULL: repeated fsyncs can exhaust the busy timeout on a
                // busy build VM before this snapshot-contention check runs.
                db.pragma_update(None, "synchronous", "NORMAL").unwrap();
                barrier.wait();
                for _ in 0..40 {
                    let tx = write_transaction(&db).unwrap();
                    let total = tx
                        .query_row("SELECT downloaded FROM traffic_totals", [], |r| {
                            r.get::<_, i64>(0)
                        })
                        .unwrap();
                    std::thread::sleep(Duration::from_millis(1));
                    tx.execute("UPDATE traffic_totals SET downloaded=?1", [total + 1])
                        .unwrap();
                    tx.commit().unwrap();
                }
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        let db = Connection::open(path).unwrap();
        assert_eq!(crate::traffic::totals(&db).unwrap().downloaded, 120);
    }
    #[test]
    fn database_survives_restart_and_preserves_untrusted_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let mut store = Store::open(&path).unwrap();
        let row = SavedDownload {
            local_path: String::new(),
            directory: String::new(),
            username: "a' OR 1=1;--".into(),
            path: "Music\\track.flac".into(),
            size: 123,
            status: "queued".into(),
        };
        store.save_downloads(std::slice::from_ref(&row)).unwrap();
        let mut view = Snapshot::default();
        view.messages.push(MessageView {
            id: 1,
            target: "room".into(),
            username: "<script>".into(),
            text: "'DROP TABLE downloads;'".into(),
            private: false,
            timestamp: 42,
        });
        view.searches.push(SearchView {
            id: "search-id".into(),
            query: "music".into(),
            collecting: true,
            results: vec![FileView {
                username: "peer".into(),
                path: "track.flac".into(),
                size: 123,
                free_slots: 1,
                speed: 100,
                attributes: Default::default(),
            }],
            ..Default::default()
        });
        view.rooms.push(RoomView {
            last_activity: Some(12345),
            name: "room".into(),
            users: 20,
        });
        store.save_snapshot(&view).unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.downloads().unwrap(), [row]);
        let mut restored = Snapshot::default();
        store.restore(&mut restored).unwrap();
        assert_eq!(restored.messages[0].text, "'DROP TABLE downloads;'");
        assert_eq!(restored.searches[0].results[0].path, "track.flac");
        assert!(!restored.searches[0].collecting);
        assert!(restored.joined_rooms.is_empty());
        assert_eq!(restored.rooms[0].last_activity, Some(12345));
    }
    #[test]
    fn connection_settings_and_mapping_identity_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let identity = store.identity().unwrap();
        let settings = crate::model::NetworkSettings {
            port: 3456,
            upnp: true,
            auto_reconnect: false,
            upload_slots: 3,
            max_peers: 32,
            download_limit_kib: 100,
            upload_limit_kib: 200,
            download_slots: 3,
            upload_per_user_limit_kib: 20,
            user_limits: vec![crate::model::UserLimit {
                username: "peer".into(),
                upload_limit_kib: 10,
                download_limit_kib: 30,
            }],
            ..Default::default()
        };
        store.save_network(&settings).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.network().unwrap(), Some(settings));
        assert_eq!(store.identity().unwrap(), identity);
    }
    #[test]
    fn unlimited_search_history_restores_in_order_with_bounded_previews_and_durable_results() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let mut store = Store::open(&path).unwrap();
        let mut view = Snapshot::default();
        for i in 0..257 {
            view.searches.push(SearchView {
                id: format!("search-{i}"),
                query: format!("album {i}"),
                total: 1,
                results: vec![FileView {
                    username: "peer".into(),
                    path: format!("album/{i}.flac"),
                    size: 123,
                    free_slots: 1,
                    speed: 100,
                    attributes: Default::default(),
                }],
                ..Default::default()
            });
        }
        store.save_snapshot(&view).unwrap();
        crate::search::trim_previews(&mut view.searches);
        assert_eq!(
            view.searches.iter().map(|s| s.results.len()).sum::<usize>(),
            crate::search::SNAPSHOT_PREVIEWS
        );
        assert_eq!(view.searches[0].results.capacity(), 0);
        store.save_snapshot(&view).unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        let mut restored = Snapshot::default();
        store.restore(&mut restored).unwrap();
        assert_eq!(restored.searches.len(), 257);
        for (i, search) in restored.searches.iter().enumerate() {
            assert_eq!(search.id, format!("search-{i}"));
            assert_eq!(search.total, 1);
            assert!(!search.collecting);
        }
        assert_eq!(
            restored
                .searches
                .iter()
                .map(|s| s.results.len())
                .sum::<usize>(),
            crate::search::SNAPSHOT_PREVIEWS
        );
        let oldest =
            crate::search::page(&store.connection, "search-0", &Default::default()).unwrap();
        assert_eq!(oldest.files[0].path, "album/0.flac");
        // Saving a bounded snapshot must retain every older normalized result.
        store.save_snapshot(&restored).unwrap();
        restored.searches.remove(1);
        store.save_snapshot(&restored).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let mut restored = Snapshot::default();
        store.restore(&mut restored).unwrap();
        assert_eq!(restored.searches.len(), 256);
        assert_eq!(restored.searches[0].id, "search-0");
        assert_eq!(restored.searches[1].id, "search-2");
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM search_results", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            256
        );
        assert!(
            store
                .connection
                .query_row("SELECT id FROM searches WHERE id='search-1'", [], |r| {
                    r.get::<_, String>(0)
                })
                .is_err()
        );
    }
    #[test]
    fn replacing_search_removes_orphaned_results() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let mut view = Snapshot::default();
        view.searches.push(SearchView {
            id: "id".into(),
            query: "q".into(),
            results: vec![],
            collecting: false,
            ..Default::default()
        });
        store.save_snapshot(&view).unwrap();
        view.searches.clear();
        store.save_snapshot(&view).unwrap();
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM searches", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn refuses_future_schema_without_modifying_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.sqlite3");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        assert!(Store::open(&path).is_err());
    }
}
