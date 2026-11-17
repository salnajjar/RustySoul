//! Monotonic, shared presence state; UI activity never clears manually selected Away.
use std::time::{Duration, Instant};
pub struct Presence {
    last_activity: Instant,
    pub automatic: bool,
}
impl Default for Presence {
    fn default() -> Self {
        Self {
            last_activity: Instant::now(),
            automatic: false,
        }
    }
}
impl Presence {
    pub fn reset(&mut self, now: Instant) -> bool {
        let was = self.automatic;
        self.automatic = false;
        self.last_activity = now;
        was
    }
    pub fn activity(&mut self, now: Instant, idle_seconds: u32) {
        let at = now
            .checked_sub(Duration::from_secs(idle_seconds as u64))
            .unwrap_or(now);
        self.last_activity = self.last_activity.max(at);
    }
    pub fn desired(&self, now: Instant, minutes: u32, away: bool) -> Option<bool> {
        let idle = minutes > 0
            && now.saturating_duration_since(self.last_activity)
                >= Duration::from_secs(minutes as u64 * 60);
        if self.automatic && !idle {
            Some(false)
        } else if !away && idle {
            Some(true)
        } else {
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_away_resets_only_for_real_activity_and_preserves_manual_away() {
        let now = Instant::now();
        let mut p = Presence::default();
        p.reset(now);
        assert_eq!(p.desired(now + Duration::from_secs(59), 1, false), None);
        assert_eq!(
            p.desired(now + Duration::from_secs(60), 1, false),
            Some(true)
        );
        p.automatic = true;
        p.activity(now + Duration::from_secs(61), 0);
        assert_eq!(
            p.desired(now + Duration::from_secs(61), 1, true),
            Some(false)
        );
        p.reset(now);
        assert_eq!(p.desired(now + Duration::from_secs(90), 1, true), None);
    }
    #[test]
    fn delayed_events_cannot_fake_recent_activity_or_overwrite_a_newer_tab() {
        let now = Instant::now();
        let mut p = Presence::default();
        p.reset(now);
        p.activity(now + Duration::from_secs(100), 90);
        assert_eq!(
            p.desired(now + Duration::from_secs(100), 1, false),
            Some(true)
        );
        p.activity(now + Duration::from_secs(100), 0);
        p.activity(now + Duration::from_secs(102), 100);
        assert_eq!(p.desired(now + Duration::from_secs(159), 1, false), None);
        p.automatic = true;
        assert_eq!(
            p.desired(now + Duration::from_secs(300), 0, true),
            Some(false)
        );
    }
}
