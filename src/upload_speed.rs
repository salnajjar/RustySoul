//! Equal-weight average of the fastest decile of the last 200 upload recipients.
//! One durable pair of payload/time totals per recipient, independent of file history.
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use soulseek_rs::UploadRateSample;
use std::{collections::HashMap, time::Instant};

pub const STARTING_SPEED: u32 = 1024 * 1024;
const RECIPIENTS: usize = 200;
#[derive(Default)]
struct Pending {
    bytes: u64,
    seconds: f64,
    last_payload: Option<Instant>,
}
pub struct Estimate {
    rate: Option<f64>,
    recipients: usize,
    selected: usize,
    sequence: i64,
    pending: HashMap<String, Pending>,
}
fn summary(db: &Connection) -> Result<(Option<f64>, usize, usize)> {
    let mut statement = db.prepare("SELECT payload_bytes,active_seconds FROM upload_speed_users ORDER BY last_sequence DESC,username LIMIT ?1")?;
    let mut rates = statement
        .query_map([RECIPIENTS as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?
        .map(|row| {
            let (bytes, seconds) = row?;
            ensure!(
                bytes > 0 && seconds.is_finite() && seconds > 0.,
                "Invalid upload speed totals"
            );
            Ok(bytes as f64 / seconds)
        })
        .collect::<Result<Vec<_>>>()?;
    rates.sort_by(|a, b| b.total_cmp(a));
    let count = rates.len();
    let selected = count.div_ceil(10);
    let rate = (selected > 0).then(|| rates[..selected].iter().sum::<f64>() / selected as f64);
    Ok((rate, count, selected))
}
impl Estimate {
    pub fn load(db: &Connection) -> Result<Self> {
        let (rate, recipients, selected) = summary(db)?;
        let sequence = db.query_row(
            "SELECT COALESCE(MAX(last_sequence),0) FROM upload_speed_users",
            [],
            |r| r.get(0),
        )?;
        Ok(Self {
            rate,
            recipients,
            selected,
            sequence,
            pending: HashMap::new(),
        })
    }
    pub fn measured(&self) -> u32 {
        self.rate
            .unwrap_or(f64::from(STARTING_SPEED))
            .round()
            .clamp(1., f64::from(u32::MAX)) as u32
    }
    pub fn has_measurement(&self) -> bool {
        self.rate.is_some()
    }
    pub fn recipients(&self) -> usize {
        self.recipients
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn observe(&mut self, db: &Connection, samples: Vec<UploadRateSample>) -> Result<()> {
        for sample in samples {
            if sample.username.is_empty()
                || !sample.seconds.is_finite()
                || sample.seconds <= 0.
                || (sample.bytes > 0 && sample.last_payload.is_none())
            {
                continue;
            }
            let entry = self
                .pending
                .entry(sample.username.to_ascii_lowercase())
                .or_default();
            let bytes = entry
                .bytes
                .checked_add(sample.bytes)
                .ok_or_else(|| anyhow::anyhow!("Upload speed payload counter overflow"))?;
            let seconds = entry.seconds + sample.seconds;
            ensure!(seconds.is_finite(), "Upload speed time counter overflow");
            entry.bytes = bytes;
            entry.seconds = seconds;
            if sample.bytes > 0 {
                entry.last_payload = entry.last_payload.max(sample.last_payload);
            }
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        let tx = crate::store::write_transaction(db)?;
        // Use actual last-payload order, not HashMap iteration or file counts.
        let mut ordered: Vec<_> = self.pending.iter().collect();
        ordered.sort_by(|(a, x), (b, y)| x.last_payload.cmp(&y.last_payload).then(a.cmp(b)));
        let mut sequence = self.sequence;
        for (username, sample) in ordered {
            let old: Option<(i64, f64, i64)> = tx.query_row("SELECT payload_bytes,active_seconds,last_sequence FROM upload_speed_users WHERE username=?1", [username], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
            // No payload means no new recipient and no change of recency.
            if sample.bytes == 0 && old.is_none() {
                continue;
            }
            let (bytes, seconds, last_sequence) = old.unwrap_or((0, 0., 0));
            let bytes = bytes
                .checked_add(i64::try_from(sample.bytes)?)
                .ok_or_else(|| anyhow::anyhow!("Upload speed payload counter overflow"))?;
            let seconds = seconds + sample.seconds;
            ensure!(
                seconds.is_finite() && seconds > 0.,
                "Invalid upload speed elapsed time"
            );
            let last_sequence = if sample.bytes > 0 {
                sequence = sequence
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("Upload speed recipient sequence overflow"))?;
                sequence
            } else {
                last_sequence
            };
            tx.execute("INSERT INTO upload_speed_users VALUES(?1,?2,?3,?4) ON CONFLICT(username) DO UPDATE SET payload_bytes=excluded.payload_bytes,active_seconds=excluded.active_seconds,last_sequence=excluded.last_sequence", params![username, bytes, seconds, last_sequence])?;
        }
        let (rate, recipients, selected) = summary(&tx)?;
        tx.commit()?;
        self.rate = rate;
        self.recipients = recipients;
        self.selected = selected;
        self.sequence = sequence;
        self.pending.clear();
        Ok(())
    }
    pub fn reported(&self, profile: &crate::profile::Profile, limit_kib: u32) -> Option<u32> {
        profile.report_upload_speed.then(|| {
            profile
                .reported_upload_speed_kib
                .or((limit_kib > 0).then_some(limit_kib))
                .map_or_else(|| self.measured(), |n| n.saturating_mul(1024))
        })
    }
    pub fn override_speed(profile: &crate::profile::Profile, limit_kib: u32) -> Option<u32> {
        if !profile.report_upload_speed {
            Some(0)
        } else {
            profile
                .reported_upload_speed_kib
                .or((limit_kib > 0).then_some(limit_kib))
                .map(|n| n.saturating_mul(1024))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{profile::Profile, store::Store};
    use std::time::Duration;
    fn sample(name: &str, bytes: u64, seconds: f64, at: Instant) -> UploadRateSample {
        UploadRateSample {
            username: name.into(),
            bytes,
            seconds,
            last_payload: (bytes > 0).then_some(at),
        }
    }
    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }
    #[test]
    fn fastest_decile_uses_equal_weight_lifetime_user_averages_not_file_or_byte_counts() {
        let store = store();
        let db = &store.connection;
        let mut estimate = Estimate::load(db).unwrap();
        let at = Instant::now();
        let samples = (1..=20)
            .map(|n| sample(&format!("peer{n}"), n * 1000, 1., at))
            .collect();
        estimate.observe(db, samples).unwrap();
        assert_eq!(estimate.measured(), 19500); // (20000 + 19000) / 2
        assert_eq!((estimate.recipients(), estimate.selected()), (20, 2));
        // Many slower files from one person still contribute just one average.
        for _ in 0..99 {
            estimate
                .observe(db, vec![sample("PEER20", 100, 1., at)])
                .unwrap();
        }
        estimate
            .observe(db, vec![sample("peer20", 100, 0., at)])
            .unwrap();
        assert_eq!(estimate.measured(), 18500); // peer19 and peer18
        assert_eq!(estimate.recipients(), 20);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM upload_speed_users", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            20
        );
    }
    #[test]
    fn cohort_is_last_200_distinct_recipients_and_a_returning_user_keeps_all_time_totals() {
        let store = store();
        let db = &store.connection;
        let at = Instant::now();
        let mut estimate = Estimate::load(db).unwrap();
        estimate
            .observe(db, vec![sample("old-fast", 1_000_000, 1., at)])
            .unwrap();
        for n in 1..=200 {
            estimate
                .observe(
                    db,
                    vec![sample(
                        &format!("peer{n}"),
                        1000,
                        1.,
                        at + Duration::from_secs(n),
                    )],
                )
                .unwrap();
        }
        assert_eq!(
            (
                estimate.recipients(),
                estimate.selected(),
                estimate.measured()
            ),
            (200, 20, 1000)
        );
        estimate
            .observe(
                db,
                vec![sample("OLD-FAST", 1000, 1., at + Duration::from_secs(201))],
            )
            .unwrap();
        assert_eq!(estimate.measured(), 25975); // (500500 + 19*1000) / 20
        assert_eq!(
            db.query_row(
                "SELECT payload_bytes FROM upload_speed_users WHERE username='old-fast'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1_001_000
        );
    }
    #[test]
    fn recency_follows_last_payload_not_map_order_or_a_stalled_streams_timer() {
        let store = store();
        let db = &store.connection;
        let at = Instant::now();
        let mut estimate = Estimate::load(db).unwrap();
        let mut samples: Vec<_> = (0..201)
            .map(|n| {
                sample(
                    &format!("peer{n:03}"),
                    if n == 0 { 1_000_000 } else { 1000 },
                    1.,
                    at + Duration::from_secs(n),
                )
            })
            .collect();
        samples.reverse();
        estimate.observe(db, samples).unwrap();
        assert_eq!(estimate.measured(), 1000);
        estimate
            .observe(
                db,
                vec![sample("peer000", 0, 1., at + Duration::from_secs(202))],
            )
            .unwrap();
        assert_eq!(
            estimate.measured(),
            1000,
            "zero payload must not re-enter the cohort"
        );
    }
    #[test]
    fn measurements_survive_restart_and_clearing_transfer_history_and_round_up_small_cohorts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let db = &store.connection;
        let at = Instant::now();
        let mut estimate = Estimate::load(db).unwrap();
        assert_eq!(estimate.measured(), STARTING_SPEED);
        assert!(!estimate.has_measurement());
        estimate
            .observe(
                db,
                vec![
                    sample("idle", 0, 3600., at),
                    sample("invalid", 500, f64::NAN, at),
                ],
            )
            .unwrap();
        assert!(!estimate.has_measurement());
        estimate
            .observe(
                db,
                (1..=11)
                    .map(|n| sample(&format!("peer{n}"), n * 1000, 1., at))
                    .collect(),
            )
            .unwrap();
        assert_eq!((estimate.selected(), estimate.measured()), (2, 10500));
        db.execute("DELETE FROM uploads", []).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let estimate = Estimate::load(&store.connection).unwrap();
        assert_eq!(
            (
                estimate.recipients(),
                estimate.selected(),
                estimate.measured()
            ),
            (11, 2, 10500)
        );
    }
    #[test]
    fn failed_database_writes_retry_exactly_once_without_losing_samples_or_advancing_cached_state()
    {
        let store = store();
        let db = &store.connection;
        let at = Instant::now();
        let mut estimate = Estimate::load(db).unwrap();
        db.pragma_update(None, "query_only", true).unwrap();
        assert!(
            estimate
                .observe(db, vec![sample("alice", 1000, 1., at)])
                .is_err()
        );
        assert!(!estimate.has_measurement());
        db.pragma_update(None, "query_only", false).unwrap();
        estimate
            .observe(
                db,
                vec![sample("alice", 3000, 1., at + Duration::from_secs(1))],
            )
            .unwrap();
        estimate.observe(db, vec![]).unwrap();
        assert_eq!(estimate.measured(), 2000);
        assert_eq!(
            db.query_row("SELECT payload_bytes FROM upload_speed_users", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            4000
        );
    }
    #[test]
    fn reporting_can_be_overridden_hidden_and_restored_without_losing_measurement() {
        let store = store();
        let mut estimate = Estimate::load(&store.connection).unwrap();
        estimate
            .observe(
                &store.connection,
                vec![sample("peer", 500000, 1., Instant::now())],
            )
            .unwrap();
        let mut profile = Profile::default();
        assert_eq!(estimate.reported(&profile, 0), Some(500000));
        assert_eq!(estimate.reported(&profile, 100), Some(102400));
        profile.reported_upload_speed_kib = Some(200);
        assert_eq!(estimate.reported(&profile, 100), Some(204800));
        profile.report_upload_speed = false;
        assert_eq!(estimate.reported(&profile, 100), None);
        assert_eq!(Estimate::override_speed(&profile, 100), Some(0));
        profile.report_upload_speed = true;
        profile.reported_upload_speed_kib = None;
        assert_eq!(estimate.reported(&profile, 0), Some(500000));
        assert_eq!(Estimate::override_speed(&profile, 0), None);
        assert!(
            serde_json::from_str::<Profile>("{}")
                .unwrap()
                .report_upload_speed
        );
        profile.reported_upload_speed_kib = Some(0);
        assert!(profile.validate().is_err());
    }
    #[test]
    fn upgrading_does_not_invent_per_user_measurements_from_a_global_estimate_or_resume_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let old = Store::open_at_schema(&path, 16).unwrap();
        old.connection
            .execute("INSERT INTO upload_speed_estimate VALUES(1,123456)", [])
            .unwrap();
        old.connection.execute_batch("INSERT INTO uploads(username,path,size,status,transferred,speed,added_at,updated_at,started_at,finished_at) VALUES('peer','a',9999999,'completed',20000,0,100,102,100,102);").unwrap();
        drop(old);
        let store = Store::open(&path).unwrap();
        let db = &store.connection;
        let estimate = Estimate::load(db).unwrap();
        assert_eq!(estimate.measured(), STARTING_SPEED);
        assert!(!estimate.has_measurement());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM uploads", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            17
        );
    }
}
