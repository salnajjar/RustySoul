//! Payload accounting: seven days of minutes, ninety days of hours, then daily forever.
//! Totals and bucket deltas commit together. No filenames, peers or per-packet records.
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize)]
pub struct Totals {
    pub downloaded: u64,
    pub uploaded: u64,
    pub since: u64,
}
pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS traffic_totals(id INTEGER PRIMARY KEY CHECK(id=1),downloaded INTEGER NOT NULL,uploaded INTEGER NOT NULL,since INTEGER NOT NULL);
      INSERT OR IGNORE INTO traffic_totals VALUES(1,0,0,strftime('%s','now'));
      CREATE TABLE IF NOT EXISTS traffic_buckets(resolution INTEGER NOT NULL,start INTEGER NOT NULL,downloaded INTEGER NOT NULL,uploaded INTEGER NOT NULL,PRIMARY KEY(resolution,start));")?;
    Ok(())
}
pub fn totals(db: &Connection) -> Result<Totals> {
    Ok(db.query_row(
        "SELECT downloaded,uploaded,since FROM traffic_totals WHERE id=1",
        [],
        |r| {
            Ok(Totals {
                downloaded: r.get::<_, i64>(0)? as u64,
                uploaded: r.get::<_, i64>(1)? as u64,
                since: r.get::<_, i64>(2)? as u64,
            })
        },
    )?)
}
pub fn record(db: &Connection, at: u64, downloaded: u64, uploaded: u64) -> Result<()> {
    record_users(db, at, downloaded, uploaded, &Default::default())
}
pub fn record_users(
    db: &Connection,
    at: u64,
    downloaded: u64,
    uploaded: u64,
    users: &std::collections::HashMap<String, (u64, u64)>,
) -> Result<()> {
    if downloaded == 0 && uploaded == 0 {
        return Ok(());
    }
    let d = i64::try_from(downloaded)?;
    let u = i64::try_from(uploaded)?;
    let tx = crate::store::write_transaction(db)?;
    // Refuse overflow rather than silently convert SQLite integers to imprecise floats.
    let old = totals(&tx)?;
    i64::try_from(
        old.downloaded
            .checked_add(downloaded)
            .ok_or_else(|| anyhow::anyhow!("Traffic counter overflow"))?,
    )?;
    i64::try_from(
        old.uploaded
            .checked_add(uploaded)
            .ok_or_else(|| anyhow::anyhow!("Traffic counter overflow"))?,
    )?;
    tx.execute(
        "UPDATE traffic_totals SET downloaded=downloaded+?1,uploaded=uploaded+?2 WHERE id=1",
        params![d, u],
    )?;
    tx.execute("INSERT INTO traffic_buckets VALUES(60,?1,?2,?3) ON CONFLICT(resolution,start) DO UPDATE SET downloaded=downloaded+excluded.downloaded,uploaded=uploaded+excluded.uploaded",params![(at/60*60) as i64,d,u])?;
    for (name, (downloaded, uploaded)) in users {
        tx.execute("INSERT INTO traffic_users VALUES(?1,?2,?3,?4,?4) ON CONFLICT(username) DO UPDATE SET downloaded=downloaded+excluded.downloaded,uploaded=uploaded+excluded.uploaded,last_at=MAX(last_at,excluded.last_at)",params![name,i64::try_from(*downloaded)?,i64::try_from(*uploaded)?,at as i64])?;
    }
    tx.commit()?;
    Ok(())
}
pub fn compact(db: &Connection, now: u64) -> Result<()> {
    let tx = crate::store::write_transaction(db)?;
    for (from, to, cutoff) in [
        (60, 3600, now.saturating_sub(7 * 86400) / 3600 * 3600),
        (3600, 86400, now.saturating_sub(90 * 86400) / 86400 * 86400),
    ] {
        tx.execute("INSERT INTO traffic_buckets SELECT ?1,start/?1*?1,SUM(downloaded),SUM(uploaded) FROM traffic_buckets WHERE resolution=?2 AND start<?3 GROUP BY start/?1 ON CONFLICT(resolution,start) DO UPDATE SET downloaded=downloaded+excluded.downloaded,uploaded=uploaded+excluded.uploaded",params![to,from,cutoff as i64])?;
        tx.execute(
            "DELETE FROM traffic_buckets WHERE resolution=?1 AND start<?2",
            params![from, cutoff as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub from: Option<u64>,
    pub to: Option<u64>,
}
#[derive(Serialize)]
pub struct Point {
    pub start: u64,
    pub downloaded: u64,
    pub uploaded: u64,
}
#[derive(Serialize)]
pub struct History {
    pub totals: Totals,
    pub from: u64,
    pub to: u64,
    pub interval: u64,
    pub points: Vec<Point>,
}
pub fn history(db: &Connection, q: &Query, now: u64) -> Result<History> {
    let totals = totals(db)?;
    let from = q.from.unwrap_or(totals.since);
    let to = q.to.unwrap_or(now.saturating_add(1));
    ensure!(
        from < to && to <= now.saturating_add(86400) && to <= i64::MAX as u64,
        "Invalid graph time range"
    );
    // Older ranges use their coarsest retained resolution; never suggest lost detail.
    let oldest = db
        .query_row(
            "SELECT MAX(resolution) FROM traffic_buckets WHERE start+resolution>?1 AND start<?2",
            params![from as i64, to as i64],
            |r| r.get::<_, Option<i64>>(0),
        )?
        .unwrap_or(60) as u64;
    let interval = ((to - from).div_ceil(720).max(oldest)).div_ceil(oldest) * oldest;
    let mut stmt=db.prepare("SELECT start/?3*?3,SUM(downloaded),SUM(uploaded) FROM traffic_buckets WHERE start+resolution>?1 AND start<?2 GROUP BY start/?3 ORDER BY 1")?;
    let points = stmt
        .query_map(params![from as i64, to as i64, interval as i64], |r| {
            Ok(Point {
                start: r.get::<_, i64>(0)? as u64,
                downloaded: r.get::<_, i64>(1)? as u64,
                uploaded: r.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(History {
        totals,
        from,
        to,
        interval,
        points,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deltas_roll_up_without_loss_restart_or_clear_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let db = Connection::open(&path).unwrap();
        initialize(&db).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=OFF;")
            .unwrap();
        let now = 200 * 86400;
        for day in 0..200 {
            for minute in 0..3 {
                record(&db, day * 86400 + minute * 60, 10, 20).unwrap();
            }
        }
        let before = totals(&db).unwrap();
        compact(&db, now).unwrap();
        compact(&db, now).unwrap();
        assert_eq!((before.downloaded, before.uploaded), (6000, 12000));
        let h = history(
            &db,
            &Query {
                from: Some(0),
                to: Some(now),
            },
            now,
        )
        .unwrap();
        assert!(h.points.len() <= 720);
        assert_eq!(
            h.points.iter().map(|p| p.downloaded).sum::<u64>(),
            before.downloaded
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM traffic_buckets WHERE resolution=60 AND start< ?1",
                [(now - 8 * 86400) as i64],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(db);
        let db = Connection::open(path).unwrap();
        initialize(&db).unwrap();
        assert_eq!(totals(&db).unwrap().uploaded, before.uploaded);
        assert!(
            history(
                &db,
                &Query {
                    from: Some(now + 1),
                    to: Some(now)
                },
                now
            )
            .is_err()
        );
    }
}
