//! Live, account-scoped Soulseek privilege checks. Never infer a purchase from a link click.
use serde::Serialize;
use std::time::{Duration, Instant};
#[derive(Clone, Default, Serialize)]
pub struct View {
    pub official_server: bool,
    pub remaining_seconds: Option<u32>,
    pub checked_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub pending: bool,
    pub watching: bool,
    pub error: Option<String>,
}
pub struct Controller {
    pub view: View,
    reply: Option<(u32, u64, Instant)>,
    requested: Option<Instant>,
    last_request: Option<Instant>,
    watch_until: Option<Instant>,
}
impl Controller {
    pub fn new(server: &str) -> Self {
        let server = server.trim_end_matches('.').to_ascii_lowercase();
        Self {
            view: View {
                official_server: server == "slsknet.org" || server.ends_with(".slsknet.org"),
                ..Default::default()
            },
            reply: None,
            requested: None,
            last_request: None,
            watch_until: None,
        }
    }
    pub fn disconnected(&mut self) {
        let official = self.view.official_server;
        *self = Self::new("");
        self.view.official_server = official;
    }
    fn remaining(&self, now: Instant) -> Option<u32> {
        self.reply.map(|(seconds, _, received)| {
            seconds.saturating_sub(
                now.saturating_duration_since(received)
                    .as_secs()
                    .min(u32::MAX as u64) as u32,
            )
        })
    }
    pub fn request(&mut self, now: Instant, watch: bool) -> bool {
        if watch {
            self.watch_until = now.checked_add(Duration::from_secs(300));
        }
        self.view.watching = self.watch_until.is_some_and(|until| now < until);
        if self.requested.is_some()
            || self
                .last_request
                .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(10))
        {
            return false;
        }
        self.requested = Some(now);
        self.last_request = Some(now);
        self.view.pending = true;
        self.view.error = None;
        true
    }
    pub fn failed(&mut self) {
        self.requested = None;
        self.view.pending = false;
        self.view.error =
            Some("Soulseek did not confirm privilege status. Refresh to try again.".into());
    }
    /// Observe all replies, decrement the last server-reported balance and schedule bounded checks.
    pub fn poll(&mut self, observation: Option<(u32, u64)>, now: Instant, wall: u64) -> bool {
        if let Some((seconds, revision)) = observation
            && self.reply.is_none_or(|(_, old, _)| old != revision)
        {
            let previous = self.remaining(now);
            if previous.is_some_and(|old| seconds > old.saturating_add(60)) {
                self.watch_until = None;
            }
            self.reply = Some((seconds, revision, now));
            self.view.checked_at = Some(wall);
            self.view.expires_at = (seconds > 0).then_some(wall.saturating_add(seconds as u64));
            self.requested = None;
            self.view.pending = false;
            self.view.error = None;
        }
        self.view.remaining_seconds = self.remaining(now);
        self.view.watching = self.watch_until.is_some_and(|until| now < until);
        if self
            .requested
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(20))
        {
            self.failed();
        }
        let expired = self.reply.is_some_and(|(seconds, _, _)| seconds > 0)
            && self.view.remaining_seconds == Some(0);
        let interval = if self.view.watching || expired {
            30
        } else if self.view.error.is_some() {
            60
        } else {
            300
        };
        if self
            .last_request
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(interval))
        {
            return self.request(now, false);
        }
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_count_down_and_expiry_is_rechecked_without_losing_unknown_state() {
        let now = Instant::now();
        let mut c = Controller::new("server.slsknet.org");
        assert!(c.view.official_server);
        assert!(c.poll(None, now, 100));
        assert_eq!(c.view.remaining_seconds, None);
        assert!(!c.poll(Some((12, 1)), now, 100));
        assert_eq!(c.view.expires_at, Some(112));
        c.poll(Some((12, 1)), now + Duration::from_secs(11), 111);
        assert_eq!(c.view.remaining_seconds, Some(1));
        assert!(c.poll(Some((12, 1)), now + Duration::from_secs(31), 131));
        assert_eq!(c.view.remaining_seconds, Some(0));
        c.poll(Some((0, 2)), now + Duration::from_secs(32), 132);
        assert_eq!(c.view.expires_at, None);
        assert!(!c.view.pending);
        c.disconnected();
        assert_eq!(c.view.remaining_seconds, None);
        assert_eq!(c.view.checked_at, None);
        assert!(c.view.official_server);
    }
    #[test]
    fn repeated_zero_replies_complete_checks_and_paid_activation_ends_short_polling() {
        let now = Instant::now();
        let mut c = Controller::new("server.slsknet.org");
        assert!(c.request(now, true));
        c.poll(Some((0, 1)), now, 1);
        assert!(!c.request(now + Duration::from_secs(1), true));
        assert!(c.request(now + Duration::from_secs(10), true));
        c.poll(Some((0, 2)), now + Duration::from_secs(11), 12);
        assert!(!c.view.pending);
        assert!(c.view.watching);
        assert!(c.poll(Some((0, 2)), now + Duration::from_secs(41), 42));
        c.poll(Some((86400, 3)), now + Duration::from_secs(42), 43);
        assert_eq!(c.view.remaining_seconds, Some(86400));
        assert!(!c.view.watching);
        assert!(!c.poll(Some((86400, 3)), now + Duration::from_secs(72), 73));
    }
    #[test]
    fn activation_watch_expires_and_normal_checks_resume_without_a_burst() {
        let now = Instant::now();
        let mut c = Controller::new("SERVER.SLSKNET.ORG.");
        assert!(c.view.official_server);
        assert!(c.request(now, true));
        c.poll(Some((0, 1)), now, 100);
        assert!(!c.poll(Some((0, 1)), now + Duration::from_secs(29), 129));
        assert!(c.poll(Some((0, 1)), now + Duration::from_secs(30), 130));
        c.poll(Some((0, 2)), now + Duration::from_secs(31), 131);
        assert!(c.poll(Some((0, 2)), now + Duration::from_secs(330), 430));
        assert!(!c.view.watching);
        c.poll(Some((0, 3)), now + Duration::from_secs(331), 431);
        assert!(!c.poll(Some((0, 3)), now + Duration::from_secs(629), 729));
        assert!(c.poll(Some((0, 3)), now + Duration::from_secs(630), 730));
        assert!(!c.request(now + Duration::from_secs(630), true));
    }
    #[test]
    fn timeouts_keep_last_known_balance_and_retry_without_request_flooding() {
        let now = Instant::now();
        let mut c = Controller::new("server.slsknet.org.attacker.invalid");
        assert!(!c.view.official_server);
        c.poll(Some((100, 1)), now, 1);
        c.poll(Some((100, 1)), now + Duration::from_secs(21), 22);
        assert!(c.view.error.is_some());
        assert_eq!(c.view.remaining_seconds, Some(79));
        assert!(!c.view.pending);
        assert!(!c.poll(Some((100, 1)), now + Duration::from_secs(59), 60));
        assert!(c.poll(Some((100, 1)), now + Duration::from_secs(60), 61));
        c.poll(Some((100, 1)), now + Duration::from_secs(400), 401);
        assert!(!c.view.watching);
    }
}
