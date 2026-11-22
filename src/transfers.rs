//! Durable transfer history with incremental pages and indivisible user groups.
use crate::model::{HistoryStatus, TransferView};
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

pub fn names(path: &str) -> (&str, &str) {
    path.rsplit_once(['\\', '/'])
        .map_or((path, ""), |(folder, name)| (name, folder))
}
pub fn migrate_names(db: &Connection) -> Result<()> {
    for table in ["downloads", "uploads"] {
        let rows: Vec<(String, String)> = db
            .prepare(&format!(
                "SELECT username,path FROM {table} WHERE file_name=''"
            ))?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        if rows.is_empty() {
            continue;
        }
        let tx = crate::store::write_transaction(db)?;
        for (u, p) in rows {
            let (name, folder) = names(&p);
            tx.execute(
                &format!(
                    "UPDATE {table} SET file_name=?1,parent_folder=?2 WHERE username=?3 AND path=?4"
                ),
                params![name, folder, u, p],
            )?;
        }
        tx.commit()?;
    }
    Ok(())
}
pub fn record_upload(db: &Connection, t: &TransferView) -> Result<bool> {
    let speed = if t.status == "uploading" { t.speed } else { 0. };
    Ok(db.execute("INSERT INTO uploads(username,path,size,transferred,speed,status,queue_position,added_at,updated_at,file_name,parent_folder,detail) VALUES(?1,?2,?3,?4,?5,?6,?7,strftime('%s','now'),strftime('%s','now'),?8,?9,?10) ON CONFLICT(username,path) DO UPDATE SET size=excluded.size,transferred=excluded.transferred,speed=excluded.speed,status=excluded.status,queue_position=excluded.queue_position,updated_at=excluded.updated_at,file_name=excluded.file_name,parent_folder=excluded.parent_folder,detail=excluded.detail WHERE uploads.size<>excluded.size OR uploads.transferred<>excluded.transferred OR uploads.speed<>excluded.speed OR uploads.status<>excluded.status OR uploads.queue_position IS NOT excluded.queue_position OR uploads.detail<>excluded.detail",params![t.username,t.path,t.size as i64,t.transferred as i64,speed,t.status,t.queue_position,names(&t.path).0,names(&t.path).1,t.detail])?>0)
}
/// A complete SDK observation is authoritative for live states. Old databases
/// can contain uploads whose terminal event was lost before acknowledgement.
pub fn reconcile_uploads(db: &Connection, current: &[(String, String)]) -> Result<bool> {
    let current: std::collections::HashSet<_> = current
        .iter()
        .map(|(u, p)| (u.as_str(), p.as_str()))
        .collect();
    let candidates = db.prepare("SELECT username,path FROM uploads WHERE status IN ('uploading','queued','checking_shares')")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let missing: Vec<_> = candidates
        .into_iter()
        .filter(|(u, p)| !current.contains(&(u.as_str(), p.as_str())))
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }
    let tx = crate::store::write_transaction(db)?;
    for (u, p) in missing {
        tx.execute("UPDATE uploads SET status='failed',speed=0,queue_position=NULL,detail='The upload is no longer active; the receiving peer disconnected or became unreachable.',finished_at=CASE WHEN started_at IS NOT NULL THEN MAX(started_at,updated_at) END,updated_at=strftime('%s','now') WHERE username=?1 AND path=?2 AND status IN ('uploading','queued','checking_shares')",params![u,p])?;
    }
    tx.commit()?;
    Ok(true)
}
/// An unopened retry has unknown progress, so it must not erase saved bytes.
/// Known zero from a newly opened/truncated partial must reset them.
/// Queue status stays owned by save_downloads; progress captures only the first
/// observed active timestamp, so accounting cannot overwrite scheduling state.
pub fn record_download(db: &Connection, t: &TransferView, progress: Option<u64>) -> Result<bool> {
    let speed = if t.status == "downloading" {
        t.speed
    } else {
        0.
    };
    Ok(db.execute("UPDATE downloads SET transferred=COALESCE(?1,transferred),speed=?2,queue_position=?3,started_at=CASE WHEN ?6='downloading' THEN COALESCE(started_at,CAST(strftime('%s','now') AS INTEGER)) ELSE started_at END,updated_at=strftime('%s','now') WHERE username=?4 AND path=?5 AND (transferred<>COALESCE(?1,transferred) OR speed<>?2 OR queue_position IS NOT ?3 OR (?6='downloading' AND started_at IS NULL))",params![progress.map(|bytes| bytes.min(t.size) as i64),speed,t.queue_position,t.username,t.path,t.status])?>0)
}
pub fn clear_uploads(db: &Connection, status: &HistoryStatus) -> Result<usize> {
    let status = match status {
        HistoryStatus::Blocked => "blocked",
        HistoryStatus::All => "all",
        HistoryStatus::Completed => "completed",
        HistoryStatus::Failed => "failed",
        HistoryStatus::Cancelled => "cancelled",
        HistoryStatus::Interrupted => "interrupted",
        HistoryStatus::TimedOut => "timed_out",
    };
    Ok(db.execute("DELETE FROM uploads WHERE status IN ('completed','failed','cancelled','interrupted','timed_out','blocked') AND (?1='all' OR status=?1)",[status])?)
}
pub fn recover_uploads(db: &Connection) -> Result<()> {
    db.execute(
        "UPDATE uploads SET status='interrupted',speed=0,finished_at=CASE WHEN started_at IS NOT NULL THEN MAX(started_at,updated_at) END WHERE status IN ('queued','uploading','checking_shares')",
        [],
    )?;
    Ok(())
}
pub fn recover_downloads(db: &Connection) -> Result<()> {
    db.execute("UPDATE downloads SET status='interrupted',speed=0,finished_at=CASE WHEN started_at IS NOT NULL THEN MAX(started_at,updated_at) END WHERE status IN ('queued','downloading','finalizing')", [])?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub group_users: bool,
    pub limit: usize,
    pub user_sort: String,
    pub status: String,
    pub filter: String,
    pub order: String,
    pub reverse: bool,
    pub offset: usize,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            group_users: false,
            limit: 100,
            user_sort: String::new(),
            status: "all".into(),
            filter: String::new(),
            order: "activity".into(),
            reverse: false,
            offset: 0,
        }
    }
}
#[derive(Default, Serialize)]
pub struct Summary {
    pub total: usize,
    pub active: usize,
    pub queued: usize,
    pub speed: f64,
    pub clearable: std::collections::BTreeMap<String, usize>,
}
#[derive(Default, Serialize)]
pub struct UserStats {
    pub downloaded: u64,
    pub uploaded: u64,
    pub first_at: u64,
    pub last_at: u64,
    pub files: usize,
}
#[derive(Serialize)]
pub struct Page {
    pub users: std::collections::BTreeMap<String, UserStats>,
    pub users_since: u64,
    pub summary: Summary,
    pub transfers: Vec<TransferView>,
    pub total: usize,
    pub offset: usize,
    pub next_offset: usize,
}
pub fn page(db: &Connection, kind: &str, q: &Query) -> Result<Page> {
    ensure!(
        matches!(kind, "downloads" | "uploads")
            && q.filter.len() <= 256
            && q.status.len() <= 32
            && q.offset <= u32::MAX as usize
            && (1..=1000).contains(&q.limit),
        "Invalid transfer query"
    );
    let local_order = format!(
        "COALESCE((SELECT priority FROM transfer_queue q WHERE q.direction='{kind}' AND q.username={kind}.username AND q.path={kind}.path),0) DESC,COALESCE((SELECT position FROM transfer_queue q WHERE q.direction='{kind}' AND q.username={kind}.username AND q.path={kind}.path),rowid) ASC"
    );
    let order = match q.order.as_str() {
        "local_queue" => local_order.as_str(),
        "activity" => {
            "CASE status WHEN 'downloading' THEN 0 WHEN 'uploading' THEN 0 WHEN 'queued' THEN 1 WHEN 'waiting' THEN 2 WHEN 'waiting_for_space' THEN 2 WHEN 'retry_waiting' THEN 3 WHEN 'paused' THEN 3 WHEN 'paused_waiting' THEN 3 WHEN 'interrupted' THEN 4 WHEN 'failed' THEN 5 WHEN 'timed_out' THEN 5 WHEN 'cancelled' THEN 6 ELSE 7 END ASC,speed DESC"
        }
        "speed" => "speed DESC",
        "progress" => "CAST(transferred AS REAL)/MAX(size,1) DESC",
        "eta" => {
            "CASE WHEN status='completed' THEN 0 ELSE (size-transferred)/MAX(speed,0.0001) END ASC"
        }
        "remaining" => "MAX(0,size-transferred) ASC",
        "size" => "size DESC",
        "name" => "file_name COLLATE NOCASE ASC",
        "folder" => "parent_folder COLLATE NOCASE ASC,file_name COLLATE NOCASE ASC",
        "user" => "username COLLATE NOCASE ASC",
        "status" => "status ASC",
        "queue" => "queue_position ASC",
        "added" => "added_at ASC,rowid ASC",
        "updated" => "updated_at DESC",
        _ => anyhow::bail!("Invalid transfer order"),
    };
    let order = if q.reverse {
        order
            .replace(" ASC", " TEMP")
            .replace(" DESC", " ASC")
            .replace(" TEMP", " DESC")
    } else {
        order.into()
    };
    let nulls = match q.order.as_str() {
        "eta" => "(status<>'completed' AND speed<=0) ASC,",
        "queue" => "queue_position IS NULL ASC,",
        _ => "",
    };
    let direction = if kind == "uploads" {
        "uploaded"
    } else {
        "downloaded"
    };
    // Materialize each user's dates once, including older history without payload attribution.
    let cte = if q.user_sort.is_empty() {
        String::new()
    } else {
        format!(
            "WITH ranked_users AS MATERIALIZED (SELECT t.username,COALESCE(u.{direction},0) volume,COALESCE(u.first_at,MIN(t.added_at)) first_at,COALESCE(u.last_at,MAX(t.updated_at)) last_at FROM {kind} t LEFT JOIN traffic_users u ON u.username=t.username COLLATE NOCASE GROUP BY t.username) "
        )
    };
    let metric = format!("(SELECT volume FROM ranked_users r WHERE r.username={kind}.username)");
    let first = format!("(SELECT first_at FROM ranked_users r WHERE r.username={kind}.username)");
    let last = format!("(SELECT last_at FROM ranked_users r WHERE r.username={kind}.username)");
    let prefix = match q.user_sort.as_str() {
        "" => String::new(),
        "most" => format!("{metric} DESC,username COLLATE NOCASE,"),
        "least" => format!("{metric} ASC,username COLLATE NOCASE,"),
        "oldest" => format!("{first} ASC,username COLLATE NOCASE,"),
        "newest" => format!("{first} DESC,username COLLATE NOCASE,"),
        "recent" => format!("{last} DESC,username COLLATE NOCASE,"),
        "name" => "username COLLATE NOCASE,".into(),
        _ => anyhow::bail!("Invalid user order"),
    };
    let order = format!("{prefix}{nulls}{order}");
    let where_clause = "(?1='all' OR status=?1) AND instr(lower(username||' '||path),lower(?2))>0";
    let total = db.query_row(
        &format!("SELECT COUNT(*) FROM {kind} WHERE {where_clause}"),
        params![q.status, q.filter],
        |r| r.get::<_, i64>(0),
    )? as usize;
    // In grouped mode rank users by their first matching file in the requested
    // order, then take whole groups whose starts fall within the next row budget.
    // A single user may exceed the budget: never split that user's matching files.
    let preference = |source: &str| {
        format!(
            "COALESCE((SELECT priority FROM transfer_queue q WHERE q.direction='{kind}' AND q.username={source}.username AND q.path={source}.path),0),COALESCE((SELECT position FROM transfer_queue q WHERE q.direction='{kind}' AND q.username={source}.username AND q.path={source}.path),0)"
        )
    };
    let sql = if q.group_users {
        let comma = if cte.is_empty() { "WITH " } else { "," };
        format!(
            "{cte}{comma}matched AS MATERIALIZED (SELECT *,ROW_NUMBER() OVER (ORDER BY {order},username,path) position FROM {kind} WHERE {where_clause}), user_groups AS (SELECT username,MIN(position) first_position,COUNT(*) files FROM matched GROUP BY username), boundaries AS (SELECT *,COALESCE(SUM(files) OVER (ORDER BY first_position ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) start FROM user_groups), selected AS (SELECT username,first_position FROM boundaries WHERE start>=?3 AND start<?3+?4) SELECT username,path,size,CASE WHEN status='completed' THEN size ELSE transferred END,speed,status,queue_position,added_at,updated_at,started_at,finished_at,{},{},{} FROM matched JOIN selected USING(username) ORDER BY first_position,position",
            if kind == "downloads" {
                "local_path"
            } else {
                "''"
            },
            if kind == "uploads" {
                "detail"
            } else {
                "CASE WHEN status='waiting_for_space' THEN 'Waiting for destination storage. Partial files are kept and downloads resume automatically when space is available.' ELSE '' END"
            },
            preference("matched")
        )
    } else {
        format!(
            "{cte}SELECT username,path,size,CASE WHEN status='completed' THEN size ELSE transferred END,speed,status,queue_position,added_at,updated_at,started_at,finished_at,{},{},{} FROM {kind} WHERE {where_clause} ORDER BY {order},username,path LIMIT ?4 OFFSET ?3",
            if kind == "downloads" {
                "local_path"
            } else {
                "''"
            },
            if kind == "uploads" {
                "detail"
            } else {
                "CASE WHEN status='waiting_for_space' THEN 'Waiting for destination storage. Partial files are kept and downloads resume automatically when space is available.' ELSE '' END"
            },
            preference(kind)
        )
    };
    let mut stmt = db.prepare(&sql)?;
    let transfers: Vec<TransferView> = stmt
        .query_map(
            params![q.status, q.filter, q.offset as i64, q.limit as i64],
            |r| {
                Ok(TransferView {
                    username: r.get(0)?,
                    path: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                    transferred: r.get::<_, i64>(3)? as u64,
                    speed: if matches!(r.get::<_, String>(5)?.as_str(), "downloading" | "uploading")
                    {
                        r.get(4)?
                    } else {
                        0.
                    },
                    status: r.get(5)?,
                    queue_position: r.get(6)?,
                    retry_at: {
                        db.query_row(
                            if kind == "downloads" {
                                "SELECT next_at FROM download_retries WHERE username=?1 AND path=?2"
                            } else {
                                "SELECT next_at FROM upload_retries WHERE username=?1 AND path=?2"
                            },
                            params![r.get::<_, String>(0)?, r.get::<_, String>(1)?],
                            |r| r.get::<_, Option<i64>>(0),
                        )
                        .unwrap_or_default()
                        .map(|v| v as u64)
                    },
                    added_at: r.get::<_, i64>(7)? as u64,
                    updated_at: r.get::<_, i64>(8)? as u64,
                    started_at: r.get::<_, Option<i64>>(9)?.map(|v| v as u64),
                    finished_at: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    local_path: r.get(11)?,
                    detail: r.get(12)?,
                    priority: r.get(13)?,
                    local_queue_order: r.get::<_, i64>(14)? as u64,
                })
            },
        )?
        .collect::<rusqlite::Result<_>>()?;
    let mut summary = Summary::default();
    for row in db
        .prepare(&format!(
            "SELECT status,COUNT(*),SUM(speed) FROM {kind} GROUP BY status"
        ))?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)? as usize,
                r.get::<_, f64>(2)?,
            ))
        })?
    {
        let (status, count, speed) = row?;
        summary.total += count;
        if matches!(
            status.as_str(),
            "waiting"
                | "queued"
                | "waiting_for_space"
                | "retry_waiting"
                | "paused"
                | "paused_waiting"
                | "checking_shares"
        ) {
            summary.queued += count;
        }
        if matches!(status.as_str(), "downloading" | "uploading") {
            summary.active += count;
            summary.speed += speed;
        }
        if matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted" | "timed_out" | "blocked"
        ) {
            *summary.clearable.entry("all".into()).or_default() += count;
            summary.clearable.insert(status, count);
        }
    }
    let mut users = std::collections::BTreeMap::new();
    for t in &transfers {
        if users.contains_key(&t.username) {
            continue;
        }
        let mut stats=db.query_row("SELECT downloaded,uploaded,first_at,last_at FROM traffic_users WHERE username=?1 COLLATE NOCASE",[&t.username],|r|Ok(UserStats{downloaded:r.get::<_,i64>(0)? as u64,uploaded:r.get::<_,i64>(1)? as u64,first_at:r.get::<_,i64>(2)? as u64,last_at:r.get::<_,i64>(3)? as u64,..Default::default()})).unwrap_or_default();
        stats.files = db.query_row(
            &format!("SELECT COUNT(*) FROM {kind} WHERE {where_clause} AND username=?3"),
            params![q.status, q.filter, t.username],
            |r| r.get::<_, i64>(0),
        )? as usize;
        users.insert(t.username.clone(), stats);
    }
    let users_since = db.query_row("SELECT since FROM traffic_user_clock WHERE id=1", [], |r| {
        r.get::<_, i64>(0)
    })? as u64;
    Ok(Page {
        users,
        users_since,
        summary,
        next_offset: q.offset + transfers.len(),
        transfers,
        total,
        offset: q.offset,
    })
}
pub fn preview(db: &Connection, kind: &str) -> Result<Vec<TransferView>> {
    Ok(page(db, kind, &Query::default())?.transfers)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_totals_cover_the_whole_direction_independent_of_page_or_filter() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        for kind in ["downloads", "uploads"] {
            let waiting: &[&str] = if kind == "downloads" {
                &[
                    "waiting",
                    "queued",
                    "waiting_for_space",
                    "retry_waiting",
                    "paused",
                    "paused_waiting",
                ]
            } else {
                &["queued", "checking_shares"]
            };
            let active = if kind == "downloads" {
                "downloading"
            } else {
                "uploading"
            };
            let statuses = waiting.iter().copied().chain([
                active,
                "completed",
                "cancelled",
                "failed",
                "timed_out",
                "blocked",
                "interrupted",
                "finalizing",
            ]);
            for (i, status) in statuses.enumerate() {
                store.connection.execute(&format!("INSERT INTO {kind}(username,path,size,status,speed,transferred,added_at,updated_at) VALUES('peer',?1,100,?2,?3,0,1,1)"), params![format!("{i}.flac"), status, if status == active { 123. } else { 0. }]).unwrap();
            }
            let page = page(
                &store.connection,
                kind,
                &Query {
                    status: "completed".into(),
                    limit: 1,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(page.transfers.len(), 1);
            assert_eq!(page.total, 1);
            assert_eq!(page.summary.queued, waiting.len());
            assert_eq!(page.summary.active, 1);
            assert_eq!(page.summary.speed, 123.);
            assert_eq!(page.summary.total, waiting.len() + 8);
        }
    }
    #[test]
    fn missing_live_upload_becomes_failed_without_changing_live_or_completed_history() {
        let db = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let db = &db.connection;
        for (user, status) in [
            ("gone", "uploading"),
            ("live", "uploading"),
            ("done", "completed"),
        ] {
            record_upload(
                db,
                &TransferView {
                    username: user.into(),
                    path: "a.flac".into(),
                    size: 100,
                    transferred: 75,
                    speed: 5511216.,
                    status: status.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        db.execute(
            "UPDATE uploads SET started_at=100,updated_at=105 WHERE username='gone'",
            [],
        )
        .unwrap();
        assert!(reconcile_uploads(db, &[("live".into(), "a.flac".into())]).unwrap());
        let gone: (String,f64,i64,i64,String) = db.query_row("SELECT status,speed,transferred,finished_at,detail FROM uploads WHERE username='gone'",[],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(
            (gone.0.as_str(), gone.1, gone.2, gone.3),
            ("failed", 0., 75, 105)
        );
        assert!(gone.4.contains("no longer active"));
        assert!(!reconcile_uploads(db, &[("live".into(), "a.flac".into())]).unwrap());
        let page = page(db, "uploads", &Query::default()).unwrap();
        assert_eq!(page.summary.active, 1);
        assert_eq!(page.summary.speed, 5511216.);
        assert!(
            page.transfers
                .iter()
                .any(|t| t.username == "done" && t.status == "completed")
        );
    }
    #[test]
    fn unopened_retries_preserve_saved_progress_but_known_zero_resets_it() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let db = &store.connection;
        db.execute("INSERT INTO downloads(username,path,size,status,transferred,speed) VALUES('peer','small.flac',10000,'failed',4093,12)",[]).unwrap();
        let mut transfer = TransferView {
            username: "peer".into(),
            path: "small.flac".into(),
            size: 10000,
            status: "queued".into(),
            queue_position: Some(7),
            ..Default::default()
        };
        assert!(record_download(db, &transfer, None).unwrap());
        let page = page(db, "downloads", &Query::default()).unwrap();
        assert_eq!(page.transfers[0].transferred, 4093);
        assert_eq!(page.transfers[0].queue_position, Some(7));
        assert_eq!(page.transfers[0].speed, 0.0);
        assert!(!record_download(db, &transfer, None).unwrap());
        assert!(record_download(db, &transfer, Some(0)).unwrap());
        transfer.status = "downloading".into();
        transfer.speed = 100.0;
        assert!(record_download(db, &transfer, Some(4093)).unwrap());
        transfer.status = "timed_out".into();
        assert!(record_download(db, &transfer, Some(4093)).unwrap());
        let history = preview(db, "downloads").unwrap();
        assert_eq!((history[0].transferred, history[0].speed), (4093, 0.0));
        assert!(record_download(db, &transfer, Some(u64::MAX)).unwrap());
        assert_eq!(preview(db, "downloads").unwrap()[0].transferred, 10000);
    }

    #[test]
    fn grouped_pages_keep_large_interleaved_users_whole_in_both_directions() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let db = &store.connection;
        let tx = crate::store::write_transaction(db).unwrap();
        for (username, count, suffix, status) in [
            ("alice", 2703, "a", "completed"),
            ("bob", 7, "b", "failed"),
            ("carol", 102, "c", "queued"),
        ] {
            for i in 0..count {
                record_upload(
                    &tx,
                    &TransferView {
                        username: username.into(),
                        path: format!("Music/{i:04}-{suffix}.flac"),
                        size: 100,
                        status: status.into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        }
        tx.execute("INSERT INTO downloads(username,path,size,status,file_name,parent_folder,added_at,updated_at) SELECT username,path,size,status,file_name,parent_folder,added_at,updated_at FROM uploads", []).unwrap();
        tx.commit().unwrap();
        for kind in ["downloads", "uploads"] {
            for user_sort in ["", "most", "least", "oldest", "newest", "recent", "name"] {
                let mut q = Query {
                    group_users: true,
                    order: "name".into(),
                    user_sort: user_sort.into(),
                    ..Default::default()
                };
                let mut seen = std::collections::BTreeSet::new();
                let mut users = std::collections::BTreeSet::new();
                loop {
                    let p = page(db, kind, &q).unwrap();
                    assert_eq!(p.total, 2812);
                    for (user, stats) in &p.users {
                        assert!(users.insert(user.clone()), "a user was split across pages");
                        assert_eq!(
                            p.transfers.iter().filter(|t| &t.username == user).count(),
                            stats.files
                        );
                    }
                    for t in &p.transfers {
                        assert!(seen.insert((t.username.clone(), t.path.clone())));
                    }
                    assert_eq!(p.next_offset, q.offset + p.transfers.len());
                    if p.next_offset == p.total {
                        break;
                    }
                    assert!(p.next_offset > q.offset);
                    q.offset = p.next_offset;
                }
                assert_eq!(seen.len(), 2812);
            }
            let p = page(
                db,
                kind,
                &Query {
                    group_users: true,
                    order: "name".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(p.transfers.len(), 2703);
            assert!(p.transfers.iter().all(|t| t.username == "alice"));
            for (filter, status, expected) in [("-b.flac", "all", 7), ("", "queued", 102)] {
                let p = page(
                    db,
                    kind,
                    &Query {
                        group_users: true,
                        filter: filter.into(),
                        status: status.into(),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!((p.total, p.transfers.len()), (expected, expected));
            }
        }
        assert!(
            page(
                db,
                "uploads",
                &Query {
                    limit: 0,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(
            page(
                db,
                "uploads",
                &Query {
                    limit: 1001,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn lifetime_user_amount_and_date_sorting_survive_history_clearing() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        crate::traffic::record_users(
            &store.connection,
            100,
            0,
            10,
            &std::collections::HashMap::from([("alice".into(), (0, 10))]),
        )
        .unwrap();
        crate::traffic::record_users(
            &store.connection,
            200,
            0,
            50,
            &std::collections::HashMap::from([("bob".into(), (0, 50))]),
        )
        .unwrap();
        for username in ["alice", "bob"] {
            record_upload(
                &store.connection,
                &TransferView {
                    username: username.into(),
                    path: "a.flac".into(),
                    status: "completed".into(),
                    size: 100,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        for (order, first) in [
            ("most", "bob"),
            ("least", "alice"),
            ("oldest", "alice"),
            ("newest", "bob"),
            ("recent", "bob"),
        ] {
            let p = page(
                &store.connection,
                "uploads",
                &Query {
                    user_sort: order.into(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(p.transfers[0].username, first);
            assert_eq!(p.users["bob"].uploaded, 50);
        }
        clear_uploads(&store.connection, &HistoryStatus::All).unwrap();
        assert_eq!(
            crate::traffic::totals(&store.connection).unwrap().uploaded,
            60
        );
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT uploaded FROM traffic_users WHERE username='bob'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            50
        );
    }
    #[test]
    fn sorting_uses_filename_and_unknown_queue_eta_stay_last_both_directions() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        for (path, speed, queue) in [("A\\z.flac", 0., None), ("Z\\a.flac", 10., Some(3))] {
            record_upload(
                &store.connection,
                &TransferView {
                    username: "alice".into(),
                    path: path.into(),
                    status: "uploading".into(),
                    size: 100,
                    speed,
                    queue_position: queue,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert_eq!(
            page(
                &store.connection,
                "uploads",
                &Query {
                    order: "name".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .transfers[0]
                .path,
            "Z\\a.flac"
        );
        for order in ["queue", "eta"] {
            for reverse in [false, true] {
                let p = page(
                    &store.connection,
                    "uploads",
                    &Query {
                        order: order.into(),
                        reverse,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(p.transfers[1].path, "A\\z.flac");
            }
        }
        for order in [
            "activity",
            "speed",
            "progress",
            "remaining",
            "size",
            "folder",
            "user",
            "status",
            "added",
            "updated",
        ] {
            page(
                &store.connection,
                "uploads",
                &Query {
                    order: order.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert!(
            page(
                &store.connection,
                "uploads;DELETE FROM downloads",
                &Query::default()
            )
            .is_err()
        );
    }
    #[test]
    fn upload_history_is_paged_durable_and_cleared_without_touching_active_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = crate::store::Store::open(&path).unwrap();
        for i in 0..2700 {
            record_upload(
                &store.connection,
                &TransferView {
                    username: "alice".into(),
                    path: format!("Music\\{i:04}.flac"),
                    size: 100,
                    status: "completed".into(),
                    transferred: 100,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        record_upload(
            &store.connection,
            &TransferView {
                username: "alice".into(),
                path: "active.flac".into(),
                status: "uploading".into(),
                size: 200,
                speed: 10.,
                ..Default::default()
            },
        )
        .unwrap();
        drop(store);
        let store = crate::store::Store::open(&path).unwrap();
        let p = page(
            &store.connection,
            "uploads",
            &Query {
                offset: 2600,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((p.total, p.transfers.len()), (2701, 100));
        assert_eq!(
            clear_uploads(&store.connection, &HistoryStatus::All).unwrap(),
            2700
        );
        assert_eq!(
            page(&store.connection, "uploads", &Query::default())
                .unwrap()
                .transfers[0]
                .path,
            "active.flac"
        );
    }
}
