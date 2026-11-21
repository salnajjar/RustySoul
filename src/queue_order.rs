//! Durable local transfer priorities. Remote uploaders retain control of their own queues.
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Default)]
pub struct Rule {
    pub priority: i8,
    pub position: u64,
}
pub type Rules = BTreeMap<(String, String, String), Rule>;
pub fn load(db: &Connection) -> Result<Rules> {
    let mut rules = Rules::new();
    for (index, row) in db
        .prepare("SELECT direction,username,path,priority,position FROM transfer_queue")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i8>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .enumerate()
    {
        let (kind, user, path, priority, position) = row?;
        ensure!(
            matches!(kind.as_str(), "downloads" | "uploads")
                && (-1..=1).contains(&priority)
                && position >= 0,
            "Invalid saved transfer queue preference"
        );
        crate::validation::username(&user).map_err(anyhow::Error::msg)?;
        crate::validation::remote_path(&path).map_err(anyhow::Error::msg)?;
        ensure!(index < 32768, "Too many saved transfer queue preferences");
        rules.insert(
            (kind, user.to_lowercase(), path),
            Rule {
                priority,
                position: position as u64,
            },
        );
    }
    Ok(rules)
}
pub fn rule(rules: &Rules, kind: &str, user: &str, path: &str, fallback: u64) -> Rule {
    rules
        .get(&(kind.into(), user.to_lowercase(), path.into()))
        .copied()
        .unwrap_or(Rule {
            priority: 0,
            position: fallback,
        })
}
pub fn update(
    db: &Connection,
    kind: &str,
    user: &str,
    path: &str,
    priority: Option<i8>,
    movement: Option<crate::model::QueueMove>,
) -> Result<Rules> {
    ensure!(
        matches!(kind, "downloads" | "uploads"),
        "Invalid transfer direction"
    );
    let tx = crate::store::write_transaction(db)?;
    let mut rules = load(&tx)?;
    let states = if kind == "downloads" {
        "'waiting','waiting_for_space','queued','paused','paused_waiting','retry_waiting','interrupted'"
    } else {
        "'queued','checking_shares'"
    };
    let upload_guard = if kind == "uploads" {
        " AND (queue_position IS NULL OR queue_position>0)"
    } else {
        ""
    };
    let mut waiting: Vec<(String, String, Rule)> = tx
        .prepare(&format!(
            "SELECT username,path,rowid FROM {kind} WHERE status IN ({states}){upload_guard} ORDER BY rowid"
        ))?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .map(|r| {
            let (u, p, index) = r?;
            let preference = rule(&rules, kind, &u, &p, index as u64);
            Ok((u, p, preference))
        })
        .collect::<rusqlite::Result<_>>()?;
    ensure!(
        waiting.len() <= 32768,
        "Too many waiting transfers to reorder"
    );
    let target=waiting.iter().position(|(u,p,_)|u.eq_ignore_ascii_case(user)&&p==path).ok_or_else(||anyhow::anyhow!("Only waiting or paused transfers can be reordered; active files continue uninterrupted"))?;
    if let Some(priority) = priority {
        waiting[target].2.priority = priority;
    }
    waiting.sort_by_key(|(_, _, r)| (std::cmp::Reverse(r.priority), r.position));
    let index = waiting
        .iter()
        .position(|(u, p, _)| u.eq_ignore_ascii_case(user) && p == path)
        .unwrap();
    if let Some(movement) = movement {
        let first = waiting
            .iter()
            .position(|(_, _, r)| r.priority == waiting[index].2.priority)
            .unwrap();
        let last = waiting
            .iter()
            .rposition(|(_, _, r)| r.priority == waiting[index].2.priority)
            .unwrap();
        let destination = match movement {
            crate::model::QueueMove::Top => first,
            crate::model::QueueMove::Up => index.saturating_sub(1).max(first),
            crate::model::QueueMove::Down => (index + 1).min(last),
            crate::model::QueueMove::Bottom => last,
        };
        let entry = waiting.remove(index);
        waiting.insert(destination, entry);
    }
    // Discard preferences for cleared/finished history, retaining active rows
    // until their next normal queue edit. New work appends to the chosen order.
    tx.execute(&format!("DELETE FROM transfer_queue WHERE direction=?1 AND NOT EXISTS(SELECT 1 FROM {kind} t WHERE t.username=transfer_queue.username AND t.path=transfer_queue.path AND t.status NOT IN ('completed','failed','cancelled','timed_out'))"),[kind])?;
    for (position, (u, p, r)) in waiting.iter().enumerate() {
        tx.execute("INSERT INTO transfer_queue VALUES(?1,?2,?3,?4,?5) ON CONFLICT(direction,username,path) DO UPDATE SET priority=excluded.priority,position=excluded.position",params![kind,u,p,r.priority,position as i64])?;
    }
    ensure!(
        tx.query_row("SELECT COUNT(*) FROM transfer_queue", [], |r| r
            .get::<_, i64>(0))?
            <= 32768,
        "Too many saved queue preferences"
    );
    tx.commit()?;
    rules = load(db)?;
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn priority_manual_order_restart_and_backup_rows_preserve_both_queues() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite3");
        let store = crate::store::Store::open(&path).unwrap();
        let db = &store.connection;
        for (u, p) in [("alice", "a.flac"), ("alice", "b.flac"), ("bob", "c.flac")] {
            db.execute(
                "INSERT INTO downloads(username,path,size,status) VALUES(?1,?2,100,'waiting')",
                params![u, p],
            )
            .unwrap();
            crate::transfers::record_upload(
                db,
                &crate::model::TransferView {
                    username: u.into(),
                    path: p.into(),
                    size: 100,
                    status: "queued".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        for kind in ["downloads", "uploads"] {
            update(db, kind, "bob", "c.flac", Some(1), None).unwrap();
            let first = crate::transfers::page(
                db,
                kind,
                &crate::transfers::Query {
                    order: "local_queue".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(first.transfers[0].path, "c.flac");
            assert_eq!(first.transfers[0].priority, 1);
            update(
                db,
                kind,
                "alice",
                "b.flac",
                None,
                Some(crate::model::QueueMove::Top),
            )
            .unwrap();
            let p = crate::transfers::page(
                db,
                kind,
                &crate::transfers::Query {
                    order: "local_queue".into(),
                    group_users: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                p.transfers
                    .iter()
                    .map(|t| t.path.as_str())
                    .collect::<Vec<_>>(),
                ["c.flac", "b.flac", "a.flac"]
            );
            update(db, kind, "bob", "c.flac", Some(-1), None).unwrap();
            let p = crate::transfers::page(
                db,
                kind,
                &crate::transfers::Query {
                    order: "local_queue".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                p.transfers
                    .iter()
                    .map(|t| t.path.as_str())
                    .collect::<Vec<_>>(),
                ["b.flac", "a.flac", "c.flac"]
            );
            update(
                db,
                kind,
                "alice",
                "b.flac",
                None,
                Some(crate::model::QueueMove::Bottom),
            )
            .unwrap();
            let p = crate::transfers::page(
                db,
                kind,
                &crate::transfers::Query {
                    order: "local_queue".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                p.transfers
                    .iter()
                    .map(|t| t.path.as_str())
                    .collect::<Vec<_>>(),
                ["a.flac", "b.flac", "c.flac"]
            );
        }
        drop(store);
        let store = crate::store::Store::open(&path).unwrap();
        assert_eq!(load(&store.connection).unwrap().len(), 6);
        store
            .connection
            .execute(
                "UPDATE downloads SET status='completed' WHERE path='a.flac'",
                [],
            )
            .unwrap();
        assert!(
            update(
                &store.connection,
                "downloads",
                "alice",
                "a.flac",
                Some(1),
                None
            )
            .is_err()
        );
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            17
        );
    }
}
