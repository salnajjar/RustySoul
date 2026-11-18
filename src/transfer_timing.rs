//! Persist observed transfer lifecycle boundaries, independently of progress ticks.
use anyhow::Result;
use rusqlite::Connection;

pub(crate) fn migrate(db: &Connection) -> Result<()> {
    let tx = crate::store::write_transaction(db)?;
    for table in ["downloads", "uploads"] {
        // Deliberately leave existing history unknown: added/updated timestamps
        // cannot establish when payload transfer actually began.
        tx.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN started_at INTEGER CHECK(started_at IS NULL OR started_at>=0);
             ALTER TABLE {table} ADD COLUMN finished_at INTEGER CHECK(finished_at IS NULL OR (started_at IS NOT NULL AND finished_at>=started_at));
             CREATE TRIGGER {table}_timing_insert AFTER INSERT ON {table}
             WHEN NEW.status IN ('downloading','uploading','completed')
             BEGIN
               UPDATE {table} SET started_at=COALESCE(started_at,strftime('%s','now')),
                 finished_at=CASE WHEN NEW.status='completed' THEN COALESCE(finished_at,MAX(COALESCE(started_at,0),CAST(strftime('%s','now') AS INTEGER))) ELSE NULL END
               WHERE username=NEW.username AND path=NEW.path;
             END;
             CREATE TRIGGER {table}_timing_status AFTER UPDATE OF status ON {table}
             WHEN NEW.status<>OLD.status
             BEGIN
               UPDATE {table} SET
                 started_at=CASE
                   WHEN OLD.status IN ('completed','failed','timed_out','cancelled','interrupted','blocked') AND NEW.status NOT IN ('completed','failed','timed_out','cancelled','interrupted','blocked')
                     THEN CASE WHEN NEW.status IN ('downloading','uploading') THEN CAST(strftime('%s','now') AS INTEGER) ELSE NULL END
                   WHEN NEW.status IN ('downloading','uploading','completed') THEN COALESCE(started_at,CAST(strftime('%s','now') AS INTEGER))
                   ELSE started_at END,
                 finished_at=CASE
                   WHEN NEW.status IN ('completed','failed','timed_out','cancelled','interrupted','blocked') AND (started_at IS NOT NULL OR NEW.status='completed')
                     THEN COALESCE(finished_at,MAX(COALESCE(started_at,0),CAST(strftime('%s','now') AS INTEGER)))
                   ELSE NULL END
               WHERE username=NEW.username AND path=NEW.path;
             END;"
        ))?;
    }
    tx.execute_batch("PRAGMA user_version=14;")?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{store::Store, transfers};
    use std::path::Path;

    fn timing(db: &rusqlite::Connection, table: &str) -> (Option<u64>, Option<u64>) {
        db.query_row(
            &format!("SELECT started_at,finished_at FROM {table}"),
            [],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?.map(|v| v as u64),
                    r.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                ))
            },
        )
        .unwrap()
    }

    #[test]
    fn both_directions_track_start_pause_completion_and_a_fresh_retry() {
        for (table, active) in [("downloads", "downloading"), ("uploads", "uploading")] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.sqlite3");
            let store = Store::open(&path).unwrap();
            let db = &store.connection;
            db.execute(&format!("INSERT INTO {table}(username,path,size,status,transferred,speed,added_at,updated_at) VALUES('peer','file',100,'queued',0,0,1,1)"), []).unwrap();
            assert_eq!(timing(db, table), (None, None));
            db.execute(&format!("UPDATE {table} SET status=?1"), [active])
                .unwrap();
            assert!(timing(db, table).0.unwrap() > 0);
            db.execute(
                &format!("UPDATE {table} SET started_at=100,status='paused'"),
                [],
            )
            .unwrap();
            assert_eq!(timing(db, table), (Some(100), None));
            db.execute(&format!("UPDATE {table} SET status=?1"), [active])
                .unwrap();
            assert_eq!(timing(db, table), (Some(100), None));
            db.execute(
                &format!("UPDATE {table} SET status='completed',transferred=100"),
                [],
            )
            .unwrap();
            let finished = timing(db, table);
            assert_eq!(finished.0, Some(100));
            assert!(finished.1.unwrap() >= 100);
            db.execute(
                &format!("UPDATE {table} SET updated_at=999999,speed=5,status='completed'"),
                [],
            )
            .unwrap();
            assert_eq!(timing(db, table), finished);
            for grouped in [false, true] {
                let page = transfers::page(
                    db,
                    table,
                    &transfers::Query {
                        group_users: grouped,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    (page.transfers[0].started_at, page.transfers[0].finished_at),
                    finished
                );
            }
            drop(store);
            let reopened = Store::open(&path).unwrap();
            let db = &reopened.connection;
            assert_eq!(timing(db, table), finished);
            db.execute(&format!("UPDATE {table} SET status='queued'"), [])
                .unwrap();
            assert_eq!(timing(db, table), (None, None));
            db.execute(&format!("UPDATE {table} SET status=?1"), [active])
                .unwrap();
            assert!(timing(db, table).0.unwrap() > 100);
            assert_eq!(timing(db, table).1, None);
        }
    }

    #[test]
    fn terminal_results_freeze_and_queued_cancellation_never_invents_a_start() {
        for table in ["downloads", "uploads"] {
            for status in ["failed", "timed_out", "cancelled", "interrupted", "blocked"] {
                let store = Store::open(Path::new(":memory:")).unwrap();
                let db = &store.connection;
                db.execute(&format!("INSERT INTO {table}(username,path,size,status,transferred,speed,added_at,updated_at) VALUES('peer','file',100,'queued',0,0,1,1)"), []).unwrap();
                db.execute(&format!("UPDATE {table} SET status=?1"), [status])
                    .unwrap();
                assert_eq!(timing(db, table), (None, None));
                db.execute(&format!("UPDATE {table} SET status='downloading'"), [])
                    .unwrap();
                db.execute(
                    &format!("UPDATE {table} SET started_at=100,status=?1"),
                    [status],
                )
                .unwrap();
                let ended = timing(db, table);
                assert!(ended.1.is_some());
                db.execute(
                    &format!("UPDATE {table} SET status=?1,updated_at=999999"),
                    [status],
                )
                .unwrap();
                assert_eq!(timing(db, table), ended);
            }
        }
    }

    #[test]
    fn schema_thirteen_history_remains_unknown_and_recovery_uses_last_live_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        let old = Store::open_at_schema(&path, 13).unwrap();
        old.connection.execute("INSERT INTO downloads(username,path,size,status,added_at,updated_at) VALUES('peer','file',100,'completed',10,90)", []).unwrap();
        drop(old);
        let store = Store::open(&path).unwrap();
        let db = &store.connection;
        assert_eq!(timing(db, "downloads"), (None, None));
        db.execute("UPDATE downloads SET status='downloading'", [])
            .unwrap();
        db.execute("UPDATE downloads SET started_at=100,updated_at=145", [])
            .unwrap();
        transfers::recover_downloads(db).unwrap();
        assert_eq!(timing(db, "downloads"), (Some(100), Some(145)));
        transfers::recover_downloads(db).unwrap();
        assert_eq!(timing(db, "downloads"), (Some(100), Some(145)));
        db.execute("INSERT INTO uploads(username,path,size,status,transferred,speed,added_at,updated_at) VALUES('peer','file',100,'uploading',0,0,1,1)", []).unwrap();
        db.execute("UPDATE uploads SET started_at=100,updated_at=150", [])
            .unwrap();
        transfers::recover_uploads(db).unwrap();
        assert_eq!(timing(db, "uploads"), (Some(100), Some(150)));
    }

    #[test]
    fn completion_without_an_intermediate_progress_tick_has_finite_timing() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let db = &store.connection;
        db.execute(
            "INSERT INTO downloads(username,path,size,status) VALUES('peer','empty',0,'waiting')",
            [],
        )
        .unwrap();
        db.execute("UPDATE downloads SET status='completed'", [])
            .unwrap();
        let (start, end) = timing(db, "downloads");
        assert!(start.is_some());
        assert_eq!(start, end);
    }
}
