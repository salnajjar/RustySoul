//! Durable conversations, read in small cursor-based pages independently of SSE.
use crate::model::MessageView;
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeleteRange {
    #[default]
    All,
    OlderThan {
        days: u32,
    },
    Before {
        timestamp: u64,
    },
    Between {
        from: u64,
        to: u64,
    },
}
impl DeleteRange {
    pub fn bounds(&self, now: u64) -> Result<(Option<u64>, Option<u64>)> {
        let range = match *self {
            Self::All => (None, None),
            Self::OlderThan { days } => {
                ensure!((1..=36500).contains(&days), "Choose 1..36500 days");
                (None, Some(now.saturating_sub(u64::from(days) * 86400)))
            }
            Self::Before { timestamp } => {
                ensure!(timestamp > 0, "Choose a date and time");
                (None, Some(timestamp))
            }
            Self::Between { from, to } => {
                ensure!(from < to, "The end must be after the start");
                (Some(from), Some(to))
            }
        };
        ensure!(
            range
                .0
                .into_iter()
                .chain(range.1)
                .all(|v| v <= i64::MAX as u64),
            "Invalid history date"
        );
        Ok(range)
    }
    pub fn matches(&self, timestamp: u64, now: u64) -> bool {
        self.bounds(now).is_ok_and(|(from, to)| {
            from.is_none_or(|n| timestamp >= n) && to.is_none_or(|n| timestamp < n)
        })
    }
}

pub fn read_cursors(db: &Connection, private: bool) -> Result<BTreeMap<String, u64>> {
    let sql = if private {
        "SELECT lower(username),last_id FROM private_reads"
    } else {
        "SELECT room,last_id FROM room_reads"
    };
    Ok(db
        .prepare(sql)?
        .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?
        .collect::<rusqlite::Result<_>>()?)
}

// Recount only conversations with new messages or a changed read cursor.
// Full history is visited once at startup and when ignore rules change.
pub fn unread(
    db: &Connection,
    own: &str,
    private: bool,
    target: Option<&str>,
) -> Result<BTreeMap<String, usize>> {
    let (table, column, key) = if private {
        ("private_reads", "username", "lower(m.target)")
    } else {
        ("room_reads", "room", "m.target")
    };
    let filter = match (target, private) {
        (Some(_), true) => " AND m.target=?3 COLLATE NOCASE",
        (Some(_), false) => " AND m.target=?3",
        _ => "",
    };
    let sql = format!(
        "SELECT {key},COUNT(*) FROM messages m WHERE m.private=?1 AND m.username<>?2 COLLATE NOCASE AND m.id>COALESCE((SELECT last_id FROM {table} WHERE {column}=m.target),0) AND NOT EXISTS(SELECT 1 FROM user_rules u WHERE u.ignored=1 AND u.username=m.username COLLATE NOCASE){filter} GROUP BY {key}"
    );
    let mut values = vec![
        rusqlite::types::Value::Integer(private as i64),
        rusqlite::types::Value::Text(own.into()),
    ];
    if let Some(target) = target {
        values.push(rusqlite::types::Value::Text(target.into()));
    }
    Ok(db
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(values), |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub target: String,
    pub private: bool,
    pub before_id: Option<u64>,
    pub after_id: Option<u64>,
    pub filter: String,
    pub username: String,
}
#[derive(Serialize)]
pub struct Page {
    pub messages: Vec<MessageView>,
    pub has_more: bool,
}
pub fn validate(target: &str, private: bool) -> Result<()> {
    if private {
        crate::validation::username(target)
    } else {
        crate::validation::room(target)
    }
    .map_err(anyhow::Error::msg)
}
pub fn page(db: &Connection, q: &Query) -> Result<Page> {
    validate(&q.target, q.private)?;
    ensure!(
        q.filter.len() <= 256 && !q.filter.chars().any(char::is_control),
        "Message search must be at most 256 bytes without control characters"
    );
    if !q.username.is_empty() {
        crate::validation::username(&q.username).map_err(anyhow::Error::msg)?;
    }
    ensure!(
        q.before_id.is_none() || q.after_id.is_none(),
        "Choose only one message cursor"
    );
    ensure!(
        q.before_id
            .into_iter()
            .chain(q.after_id)
            .all(|id| id <= i64::MAX as u64),
        "Invalid message cursor"
    );
    let ascending = q.after_id.is_some();
    let condition = if ascending { "id>?3" } else { "id<?3" };
    let order = if ascending { "ASC" } else { "DESC" };
    let cursor = q.after_id.or(q.before_id).unwrap_or(i64::MAX as u64) as i64;
    let collation = if q.private { "COLLATE NOCASE" } else { "" };
    let mut stmt = db.prepare(&format!("SELECT id,target,username,text,private,timestamp FROM messages m WHERE private=?1 AND target=?2 {collation} AND {condition} AND (?4='' OR instr(lower(text),lower(?4))>0) AND (?5='' OR username=?5 COLLATE NOCASE) AND NOT EXISTS(SELECT 1 FROM user_rules u WHERE u.ignored=1 AND u.username=m.username COLLATE NOCASE) ORDER BY id {order} LIMIT 101"))?;
    let mut messages = stmt
        .query_map(
            params![q.private, q.target, cursor, q.filter, q.username],
            |r| {
                Ok(MessageView {
                    id: r.get::<_, i64>(0)? as u64,
                    target: r.get(1)?,
                    username: r.get(2)?,
                    text: r.get(3)?,
                    private: r.get(4)?,
                    timestamp: r.get::<_, i64>(5)? as u64,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = messages.len() > 100;
    messages.truncate(100);
    if !ascending {
        messages.reverse();
    }
    Ok(Page { messages, has_more })
}
#[derive(Serialize)]
pub struct Conversation {
    pub target: String,
    pub private: bool,
    pub latest_id: u64,
}
pub fn conversations(db: &Connection) -> Result<Vec<Conversation>> {
    Ok(db
        .prepare(
            "SELECT target,private,MAX(id) FROM messages GROUP BY private,target ORDER BY MAX(id) DESC",
        )?
        .query_map([], |r| {
            Ok(Conversation {
                target: r.get(0)?,
                private: r.get(1)?,
                latest_id: r.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Snapshot, store::Store};
    #[test]
    fn search_filters_full_history_and_sender_with_scoped_stable_pages_and_ignores() {
        let mut store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let mut rows = Vec::new();
        for id in 1..=420 {
            rows.push(MessageView {
                id,
                target: if id == 420 { "other" } else { "club" }.into(),
                username: if id % 2 == 0 { "Alice" } else { "bob" }.into(),
                text: if id == 2 {
                    "literal %_ quote'"
                } else {
                    "Jazz recommendation"
                }
                .into(),
                private: id == 418,
                timestamp: id,
            });
        }
        store.save_messages(&rows).unwrap();
        let mut q = Query {
            target: "club".into(),
            filter: "JAZZ".into(),
            username: "aLiCe".into(),
            ..Default::default()
        };
        let first = page(&store.connection, &q).unwrap();
        assert!(first.has_more);
        assert_eq!(first.messages.len(), 100);
        assert_eq!(first.messages.last().unwrap().id, 416);
        q.before_id = first.messages.first().map(|m| m.id);
        let older = page(&store.connection, &q).unwrap();
        assert!(older.messages.iter().all(|m| m.id < first.messages[0].id));
        q.before_id = None;
        q.filter = "%_ quote'".into();
        assert_eq!(page(&store.connection, &q).unwrap().messages[0].id, 2);
        q.filter.clear();
        store
            .connection
            .execute("INSERT INTO user_rules VALUES('ALICE',1,0)", [])
            .unwrap();
        assert!(page(&store.connection, &q).unwrap().messages.is_empty());
        q.username = "bob".into();
        assert_eq!(page(&store.connection, &q).unwrap().messages.len(), 100);
        q.username = "bad\nuser".into();
        assert!(page(&store.connection, &q).is_err());
        q.username.clear();
        q.filter = "x".repeat(257);
        assert!(page(&store.connection, &q).is_err());
    }
    #[test]
    fn ranged_deletion_keeps_boundaries_other_conversations_and_fresh_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut store = Store::open(&path).unwrap();
        let messages: Vec<_> = (1..=6)
            .map(|id| MessageView {
                id,
                target: if id == 6 { "other" } else { "Alice" }.into(),
                username: "peer".into(),
                text: id.to_string(),
                private: id != 5,
                timestamp: id * 100,
            })
            .collect();
        store.save_messages(&messages).unwrap();
        let (removed, _) = store
            .clear_message_history_filtered(
                "ALICE",
                true,
                &DeleteRange::Between { from: 200, to: 400 },
                1000,
            )
            .unwrap();
        assert_eq!(removed, 2);
        assert!(
            store
                .clear_message_history_filtered(
                    "Alice",
                    true,
                    &DeleteRange::Between { from: 400, to: 300 },
                    1000
                )
                .is_err()
        );
        drop(store);
        let mut store = Store::open(&path).unwrap();
        let q = Query {
            target: "alice".into(),
            private: true,
            ..Default::default()
        };
        assert_eq!(
            page(&store.connection, &q)
                .unwrap()
                .messages
                .iter()
                .map(|m| m.timestamp)
                .collect::<Vec<_>>(),
            vec![100, 400]
        );
        assert_eq!(conversations(&store.connection).unwrap().len(), 3);
        assert!(store.reserve_message_ids().unwrap().0 > 6);
        assert_eq!(
            store
                .clear_message_history_filtered(
                    "Alice",
                    true,
                    &DeleteRange::OlderThan { days: 1 },
                    86500
                )
                .unwrap()
                .0,
            0
        );
        assert_eq!(
            store
                .clear_message_history_filtered(
                    "Alice",
                    true,
                    &DeleteRange::Before { timestamp: 400 },
                    0
                )
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            page(&store.connection, &q).unwrap().messages[0].timestamp,
            400
        );
        for range in [
            DeleteRange::OlderThan { days: 0 },
            DeleteRange::OlderThan { days: 36501 },
            DeleteRange::Before {
                timestamp: u64::MAX,
            },
        ] {
            assert!(range.bounds(1000).is_err());
        }
    }
    #[test]
    fn ignore_rules_filter_saved_pages_and_unread_counts_without_deleting_history() {
        let mut store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let mut rows = vec![];
        for id in 1..=1200 {
            rows.push(MessageView {
                id,
                target: "room".into(),
                username: if id % 2 == 0 { "NOISY" } else { "peer" }.into(),
                text: "saved".into(),
                private: false,
                timestamp: 1,
            });
        }
        rows.push(MessageView {
            id: 1201,
            target: "Noisy".into(),
            username: "noisy".into(),
            text: "private".into(),
            private: true,
            timestamp: 1,
        });
        rows.push(MessageView {
            id: 1202,
            target: "peer".into(),
            username: "SELF".into(),
            text: "sent".into(),
            private: true,
            timestamp: 1,
        });
        store.save_messages(&rows).unwrap();
        store
            .connection
            .execute("INSERT INTO user_rules VALUES('Noisy',1,0)", [])
            .unwrap();
        let q = Query {
            target: "room".into(),
            ..Default::default()
        };
        let page = page(&store.connection, &q).unwrap();
        assert_eq!(page.messages.len(), 100);
        assert!(page.has_more);
        assert!(page.messages.iter().all(|m| m.username == "peer"));
        assert_eq!(
            unread(&store.connection, "self", false, None).unwrap()["room"],
            600
        );
        assert!(
            unread(&store.connection, "self", true, None)
                .unwrap()
                .is_empty()
        );
        assert!(
            super::page(
                &store.connection,
                &Query {
                    target: "NOISY".into(),
                    private: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .messages
            .is_empty()
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1202
        );
        store
            .connection
            .execute("INSERT INTO room_reads VALUES('room',1100)", [])
            .unwrap();
        assert_eq!(
            unread(&store.connection, "self", false, Some("room")).unwrap()["room"],
            50
        );
        store
            .connection
            .execute("DELETE FROM user_rules", [])
            .unwrap();
        assert_eq!(
            unread(&store.connection, "self", true, None).unwrap()["noisy"],
            1
        );
        assert_eq!(
            super::page(
                &store.connection,
                &Query {
                    target: "NOISY".into(),
                    private: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .messages[0]
                .text,
            "private"
        );
    }
    #[test]
    fn private_read_migration_marks_existing_history_seen_without_changing_room_markers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut old = Store::open_at_schema(&path, 14).unwrap();
        old.save_messages(&[MessageView {
            id: 1,
            target: "Peer".into(),
            username: "peer".into(),
            text: "old history".into(),
            private: true,
            timestamp: 1,
        }])
        .unwrap();
        drop(old);
        let mut new = Store::open(&path).unwrap();
        assert_eq!(read_cursors(&new.connection, true).unwrap()["peer"], 1);
        assert!(
            unread(&new.connection, "self", true, None)
                .unwrap()
                .is_empty()
        );
        new.save_messages(&[MessageView {
            id: 2,
            target: "PEER".into(),
            username: "peer".into(),
            text: "new".into(),
            private: true,
            timestamp: 2,
        }])
        .unwrap();
        assert_eq!(
            unread(&new.connection, "self", true, Some("Peer")).unwrap()["peer"],
            1
        );
        assert!(read_cursors(&new.connection, false).unwrap().is_empty());
    }
    #[test]
    fn history_outlives_the_live_preview_and_pages_without_losing_or_replacing_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut store = Store::open(&path).unwrap();
        let mut view = Snapshot::default();
        for id in 1..=1601 {
            view.messages.push(MessageView {
                id,
                target: if id == 1 {
                    "archived-peer"
                } else {
                    "busy room"
                }
                .into(),
                username: "self".into(),
                text: format!("saved {id}"),
                private: id == 1,
                timestamp: 1,
            });
        }
        store.save_snapshot(&view).unwrap();
        view.messages.drain(..1101);
        store.save_snapshot(&view).unwrap();
        view.messages.clear();
        store.save_snapshot(&view).unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        store.restore(&mut view).unwrap();
        assert_eq!(view.messages.len(), 500);
        assert_eq!(view.messages[0].id, 1102);
        assert_eq!(
            page(
                &store.connection,
                &Query {
                    target: "archived-peer".into(),
                    private: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .messages[0]
                .text,
            "saved 1"
        );
        let names = conversations(&store.connection).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0].latest_id, 1601);
        assert_eq!(names[1].latest_id, 1);
        let mut q = Query {
            target: "busy room".into(),
            ..Default::default()
        };
        let mut ids = std::collections::BTreeSet::new();
        loop {
            let p = page(&store.connection, &q).unwrap();
            assert!(p.messages.windows(2).all(|p| p[0].id < p[1].id));
            q.before_id = p.messages.first().map(|m| m.id);
            for m in p.messages {
                assert!(ids.insert(m.id));
            }
            if !p.has_more {
                break;
            }
        }
        assert_eq!(ids.len(), 1600);
        let mut changed = view.messages[0].clone();
        changed.text = "must not replace history".into();
        store.save_messages(&[changed]).unwrap();
        let p = page(
            &store.connection,
            &Query {
                target: "busy room".into(),
                after_id: Some(1101),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.messages[0].text, "saved 1102");
        assert_eq!(p.messages.len(), 100);
        assert!(p.has_more);
    }
    #[test]
    fn explicit_deletion_is_scoped_and_survives_restart_without_reusing_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut store = Store::open(&path).unwrap();
        let mut view = Snapshot::default();
        for (id, target, private) in [(1, "same", true), (2, "same", false), (3, "other", true)] {
            view.messages.push(MessageView {
                id,
                target: target.into(),
                username: "self".into(),
                text: "history".into(),
                private,
                timestamp: 1,
            });
        }
        store.save_snapshot(&view).unwrap();
        assert!(store.clear_message_history("", true).is_err());
        assert_eq!(store.clear_message_history("same", true).unwrap().0, 1);
        view.messages.retain(|m| !(m.private && m.target == "same"));
        store.save_snapshot(&view).unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(
            page(
                &store.connection,
                &Query {
                    target: "same".into(),
                    private: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .messages
            .is_empty()
        );
        assert_eq!(conversations(&store.connection).unwrap().len(), 2);
        let id = store.reserve_message_ids().unwrap().0;
        assert!(id > 3);
        store
            .save_messages(&[MessageView {
                id,
                target: "same".into(),
                username: "peer".into(),
                text: "new after deletion".into(),
                private: true,
                timestamp: 2,
            }])
            .unwrap();
        assert_eq!(
            page(
                &store.connection,
                &Query {
                    target: "same".into(),
                    private: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .messages[0]
                .text,
            "new after deletion"
        );
        for q in [
            Query {
                target: "same".into(),
                private: true,
                before_id: Some(2),
                after_id: Some(1),
                ..Default::default()
            },
            Query {
                target: "same".into(),
                after_id: Some(u64::MAX),
                ..Default::default()
            },
        ] {
            assert!(page(&store.connection, &q).is_err());
        }
    }
}
