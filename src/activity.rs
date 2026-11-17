//! Bounded, private operational history. Never record command bodies or credentials.
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, OnceLock, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static SINK: OnceLock<mpsc::SyncSender<Entry>> = OnceLock::new();
static TRANSITIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
#[derive(Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    pub at: u64,
    pub level: String,
    pub source: String,
    pub message: String,
}
pub fn record(level: &str, source: &str, message: &str) {
    if message.is_empty() {
        return;
    }
    if let Some(sink) = SINK.get() {
        let _ = sink.try_send(Entry {
            id: 0,
            at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            level: level.into(),
            source: source.into(),
            message: message
                .chars()
                .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
                .take(4096)
                .collect(),
        });
    }
}
pub fn transition(level: &str, source: &str, message: Option<&str>) {
    let states = TRANSITIONS.get_or_init(Default::default);
    if let Ok(mut states) = states.lock() {
        let key = format!("{level}:{source}");
        if let Some(message) = message {
            if states.get(&key).is_none_or(|old| old != message) {
                record(level, source, message);
                states.insert(key, message.into());
            }
        } else {
            states.remove(&key);
        }
    }
}
pub struct Writer {
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub fn start(path: &Path) -> Result<Writer> {
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(5))?;
    let (tx, rx) = mpsc::sync_channel::<Entry>(2048);
    let (stop, stopping) = mpsc::channel();
    ensure!(SINK.set(tx).is_ok(), "Activity logger already started");
    let thread = thread::Builder::new()
        .name("activity-writer".into())
        .spawn(move || {
            loop {
                let done = stopping.try_recv().is_ok();
                let mut batch = Vec::new();
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(e) => batch.push(e),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(_) => {}
                }
                batch.extend(rx.try_iter().take(2047));
                if !batch.is_empty()
                    && let Err(e) = append(&db, &batch)
                {
                    eprintln!("Could not persist operational history: {e}");
                }
                if done {
                    break;
                }
            }
        })?;
    Ok(Writer {
        stop,
        thread: Some(thread),
    })
}
pub fn append(db: &Connection, entries: &[Entry]) -> Result<()> {
    let tx = crate::store::write_transaction(db)?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO activity_log(at,level,source,message) VALUES(?1,?2,?3,?4)",
        )?;
        for e in entries {
            insert.execute(params![e.at as i64, e.level, e.source, e.message])?;
        }
    }
    tx.execute("DELETE FROM activity_log WHERE id<COALESCE((SELECT id FROM activity_log ORDER BY id DESC LIMIT 1 OFFSET 9999),0) OR at<strftime('%s','now')-604800",[])?;
    tx.commit()?;
    Ok(())
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub level: String,
    pub source: String,
    pub filter: String,
    pub before: i64,
}
#[derive(Serialize)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub more: bool,
}
pub fn page(db: &Connection, q: &Query) -> Result<Page> {
    ensure!(
        matches!(
            q.level.as_str(),
            "" | "all" | "error" | "warn" | "info" | "debug" | "trace"
        ) && q.source.len() <= 64
            && q.filter.len() <= 256
            && q.before >= 0,
        "Invalid log filter"
    );
    let mut stmt=db.prepare("SELECT id,at,level,source,message FROM activity_log WHERE (?1 IN ('','all') OR level=?1) AND (?2 IN ('','all') OR source=?2) AND (?3='' OR instr(lower(message),lower(?3))>0) AND (?4=0 OR id<?4) ORDER BY id DESC LIMIT 101")?;
    let mut entries = stmt
        .query_map(params![q.level, q.source, q.filter, q.before], |r| {
            Ok(Entry {
                id: r.get(0)?,
                at: r.get::<_, i64>(1)? as u64,
                level: r.get(2)?,
                source: r.get(3)?,
                message: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = entries.len() > 100;
    entries.truncate(100);
    Ok(Page { entries, more })
}
#[derive(Default)]
pub struct TraceLayer;
impl<S> tracing_subscriber::Layer<S> for TraceLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        #[derive(Default)]
        struct Visitor(String);
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if matches!(
                    field.name(),
                    "password" | "token" | "credentials" | "authorization" | "cookie"
                ) {
                    return;
                }
                if !self.0.is_empty() {
                    self.0.push(' ');
                }
                self.0.push_str(&format!("{}={value:?}", field.name()));
            }
        }
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        record(
            &event.metadata().level().to_string().to_lowercase(),
            "daemon",
            &visitor.0,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_private_paged_filterable_bounded_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = crate::store::Store::open(&path).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let entries = (0..10020)
            .map(|i| Entry {
                id: 0,
                at: now,
                level: if i % 2 == 0 { "error" } else { "info" }.into(),
                source: "system".into(),
                message: format!("event {i} <script>literal</script>"),
            })
            .collect::<Vec<_>>();
        append(&store.connection, &entries).unwrap();
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM activity_log", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            10000
        );
        drop(store);
        let store = crate::store::Store::open(&path).unwrap();
        let first = page(
            &store.connection,
            &Query {
                level: "error".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first.entries.len(), 100);
        assert!(first.more);
        assert!(first.entries.iter().all(|e| e.level == "error"));
        let next = page(
            &store.connection,
            &Query {
                before: first.entries.last().unwrap().id,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            next.entries
                .iter()
                .all(|e| e.id < first.entries.last().unwrap().id)
        );
        assert!(
            page(
                &store.connection,
                &Query {
                    filter: "event 10019".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .entries[0]
                .message
                .contains("<script>")
        );
    }
}
