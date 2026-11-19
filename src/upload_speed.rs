//! Persist a small, idle-free estimate from actual uploaded payload, not resume offsets.
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
pub const STARTING_SPEED: u32 = 1024 * 1024;
pub struct Estimate {
    rate: Option<f64>,
}
impl Estimate {
    pub fn load(db: &Connection) -> Result<Self> {
        let saved = db
            .query_row(
                "SELECT rate FROM upload_speed_estimate WHERE id=1",
                [],
                |r| r.get::<_, f64>(0),
            )
            .optional()?;
        let rate = saved.filter(|r| r.is_finite() && *r > 0.);
        Ok(Self { rate })
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
    pub fn observe(&mut self, db: &Connection, bytes: u64, seconds: f64) -> Result<()> {
        if bytes == 0 || !seconds.is_finite() || seconds <= 0. {
            return Ok(());
        }
        let sample = (bytes as f64 / seconds).clamp(1., f64::from(u32::MAX));
        let rate = self.rate.map_or(sample, |old| old * 0.75 + sample * 0.25);
        db.execute("INSERT INTO upload_speed_estimate VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET rate=excluded.rate",[rate])?;
        self.rate = Some(rate);
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

    #[test]
    fn idle_samples_do_not_change_the_estimate_and_measurements_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let db = &store.connection;
        let mut estimate = Estimate::load(db).unwrap();
        assert_eq!(estimate.measured(), STARTING_SPEED);
        assert!(!estimate.has_measurement());
        estimate.observe(db, 102400, 2.).unwrap();
        estimate.observe(db, 0, 3600.).unwrap();
        estimate.observe(db, 500, 0.).unwrap();
        estimate.observe(db, 500, f64::NAN).unwrap();
        assert_eq!(estimate.measured(), 51200);
        estimate.observe(db, 204800, 2.).unwrap();
        assert_eq!(estimate.measured(), 64000);
        drop(store);
        let store = Store::open(&path).unwrap();
        let estimate = Estimate::load(&store.connection).unwrap();
        assert_eq!(estimate.measured(), 64000);
        assert!(estimate.has_measurement());
    }

    #[test]
    fn reporting_can_be_overridden_hidden_and_restored_without_losing_measurement() {
        let estimate = Estimate {
            rate: Some(500000.),
        };
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
    fn an_upgrade_does_not_mistake_old_resume_offsets_for_uploaded_payload() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.connection.execute_batch("INSERT INTO uploads(username,path,size,status,transferred,speed,added_at,updated_at,started_at,finished_at) VALUES('peer','a',9999999,'completed',20000,0,100,102,100,102),('peer','b',50000,'failed',50000,0,100,101,100,101),('peer','c',100000,'completed',100000,0,105,105,105,105);").unwrap();
        let estimate = Estimate::load(&store.connection).unwrap();
        assert_eq!(estimate.measured(), STARTING_SPEED);
        assert!(!estimate.has_measurement());
    }
}
