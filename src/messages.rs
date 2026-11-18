//! Durable conversations, read in small cursor-based pages independently of SSE.
use crate::model::MessageView;
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub target: String,
    pub private: bool,
    pub before_id: Option<u64>,
    pub after_id: Option<u64>,
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
    let mut stmt = db.prepare(&format!("SELECT id,target,username,text,private,timestamp FROM messages WHERE private=?1 AND target=?2 AND {condition} ORDER BY id {order} LIMIT 101"))?;
    let mut messages = stmt
        .query_map(params![q.private, q.target, cursor], |r| {
            Ok(MessageView {
                id: r.get::<_, i64>(0)? as u64,
                target: r.get(1)?,
                username: r.get(2)?,
                text: r.get(3)?,
                private: r.get(4)?,
                timestamp: r.get::<_, i64>(5)? as u64,
            })
        })?
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
}
pub fn conversations(db: &Connection) -> Result<Vec<Conversation>> {
    Ok(db
        .prepare(
            "SELECT target,private FROM messages GROUP BY private,target ORDER BY MAX(id) DESC",
        )?
        .query_map([], |r| {
            Ok(Conversation {
                target: r.get(0)?,
                private: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Snapshot, store::Store};
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
        store.restore(&mut view, 8).unwrap();
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
