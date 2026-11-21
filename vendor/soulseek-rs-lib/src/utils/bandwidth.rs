//! Shared payload budgets: a direction-wide cap AND one budget shared by all files of a user.
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Actual payload and non-overlapping streaming time for one upload recipient.
/// Queueing, connection negotiation and idle gaps between files are excluded.
#[derive(Debug, Clone)]
pub struct UploadRateSample {
    pub username: String,
    pub bytes: u64,
    pub seconds: f64,
    pub last_payload: Option<Instant>,
}

#[derive(Default)]
pub struct Bandwidth {
    payload_bytes: AtomicU64,
    payload_users: Mutex<PayloadCounters>,
    state: Mutex<State>,
    changed: Condvar,
}
#[derive(Default)]
struct PayloadCounters {
    bytes: HashMap<String, u64>,
    uploads: HashMap<String, UploadMeasurement>,
}
#[derive(Default)]
struct UploadMeasurement {
    streams: usize,
    since: Option<Instant>,
    elapsed: Duration,
    bytes: u64,
    group_has_payload: bool,
    last_payload: Option<Instant>,
}
impl UploadMeasurement {
    fn begin(&mut self, now: Instant) {
        if self.streams == 0 {
            self.since = Some(now);
            self.group_has_payload = false;
        }
        self.streams += 1;
    }
    fn end(&mut self, now: Instant) {
        self.streams -= 1;
        if self.streams == 0
            && let Some(since) = self.since.take()
            && self.group_has_payload
        {
            self.elapsed += now.saturating_duration_since(since);
        }
    }
    fn record(&mut self, bytes: u64, now: Instant) {
        self.bytes += bytes;
        self.group_has_payload = true;
        self.last_payload = Some(now);
    }
    fn take(&mut self, username: &str, now: Instant) -> Option<UploadRateSample> {
        // Retain pre-first-byte timing until data arrives; an empty/failed
        // stream cannot create a recipient or dilute an earlier average.
        if self.group_has_payload
            && let Some(since) = self.since
        {
            self.elapsed += now.saturating_duration_since(since);
            self.since = Some(now);
        }
        if self.streams == 0 {
            self.since = None;
        }
        let elapsed = std::mem::take(&mut self.elapsed);
        let bytes = std::mem::take(&mut self.bytes);
        let last_payload = self.last_payload.take();
        (elapsed > Duration::ZERO || bytes > 0).then(|| UploadRateSample {
            username: username.into(),
            bytes,
            seconds: elapsed.as_secs_f64(),
            last_payload,
        })
    }
}
#[derive(Default)]
struct State {
    total: Bucket,
    per_user: u64,
    overrides: HashMap<String, u64>,
    users: HashMap<String, User>,
}
#[derive(Default)]
struct Bucket {
    rate: u64,
    tokens: f64,
    updated: Option<Instant>,
}
struct User {
    bucket: Bucket,
    references: usize,
    idle_since: Option<Instant>,
}
impl Bucket {
    fn capacity(&self) -> f64 {
        (self.rate as f64 / 10.0).clamp(1.0, 65536.0)
    }
    fn set(&mut self, rate: u64) {
        if self.rate != rate {
            self.rate = rate;
            self.tokens = if rate == 0 { 0.0 } else { self.capacity() };
            self.updated = Some(Instant::now());
        }
    }
    fn available(&mut self, now: Instant) -> usize {
        if self.rate == 0 {
            return usize::MAX;
        }
        let elapsed = self
            .updated
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        self.tokens = (self.tokens + elapsed.as_secs_f64() * self.rate as f64).min(self.capacity());
        self.updated = Some(now);
        self.tokens.floor() as usize
    }
    fn spend(&mut self, n: usize) {
        if self.rate != 0 {
            self.tokens -= n as f64;
        }
    }
    fn refund(&mut self, n: usize) {
        if self.rate != 0 {
            self.tokens = (self.tokens + n as f64).min(self.capacity());
        }
    }
}
impl Bandwidth {
    pub fn take_payload_users(&self) -> HashMap<String, u64> {
        std::mem::take(
            &mut self
                .payload_users
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .bytes,
        )
    }
    pub fn take_upload_rate_samples(&self) -> Vec<UploadRateSample> {
        let mut counters = self.payload_users.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let samples = counters
            .uploads
            .iter_mut()
            .filter_map(|(name, measurement)| measurement.take(name, now))
            .collect();
        counters
            .uploads
            .retain(|_, measurement| measurement.streams > 0);
        samples
    }
    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes.load(Ordering::Relaxed)
    }
    pub fn configure(&self, total: u64, per_user: u64, overrides: Vec<(String, u64)>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.total.set(total);
        s.per_user = per_user;
        s.overrides = overrides.into_iter().collect();
        let rates = s.overrides.clone();
        for (name, user) in &mut s.users {
            user.bucket.set(*rates.get(name).unwrap_or(&per_user));
        }
        self.changed.notify_all();
    }
    pub fn set_total(&self, total: u64) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .total
            .set(total);
        self.changed.notify_all();
    }
    pub fn for_user(self: &Arc<Self>, name: &str) -> UserBandwidth {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let rate = *s.overrides.get(name).unwrap_or(&s.per_user);
        // Keep idle credit briefly so successive small files cannot reset a user's budget.
        s.users.retain(|_, u| {
            u.references > 0
                || u.idle_since
                    .is_none_or(|at| at.elapsed() < Duration::from_secs(1))
        });
        let user = s.users.entry(name.into()).or_insert_with(|| {
            let mut bucket = Bucket::default();
            bucket.set(rate);
            User {
                bucket,
                references: 0,
                idle_since: None,
            }
        });
        user.references += 1;
        user.idle_since = None;
        UserBandwidth {
            owner: self.clone(),
            name: name.into(),
            measurement_name: name.to_ascii_lowercase(),
            transferred: Arc::default(),
        }
    }
}
pub struct UserBandwidth {
    owner: Arc<Bandwidth>,
    name: String,
    measurement_name: String,
    pub transferred: Arc<AtomicU64>,
}
pub(crate) struct UploadTiming<'a>(&'a UserBandwidth);
impl Drop for UploadTiming<'_> {
    fn drop(&mut self) {
        let mut counters = self
            .0
            .owner
            .payload_users
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(measurement) = counters.uploads.get_mut(&self.0.measurement_name) {
            measurement.end(Instant::now());
        }
    }
}
pub struct Allowance<'a> {
    user: &'a UserBandwidth,
    pub bytes: usize,
    used: usize,
}
impl UserBandwidth {
    /// Begin only after the file handshake, for a file with actual bytes left.
    pub(crate) fn begin_upload(&self) -> UploadTiming<'_> {
        let mut counters = self
            .owner
            .payload_users
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        counters
            .uploads
            .entry(self.measurement_name.clone())
            .or_default()
            .begin(Instant::now());
        UploadTiming(self)
    }
    pub fn take(&self, want: usize, cancelled: impl Fn() -> bool) -> io::Result<Allowance<'_>> {
        if want == 0 {
            return Ok(Allowance {
                user: self,
                bytes: 0,
                used: 0,
            });
        }
        loop {
            // Callbacks can read protocol state; never invoke one under the budget lock.
            if cancelled() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let mut s = self.owner.state.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let total = s.total.available(now);
            let user = s.users.get_mut(&self.name).expect("live bandwidth handle");
            let available = user.bucket.available(now);
            let bytes = want.min(total).min(available);
            if bytes > 0 {
                user.bucket.spend(bytes);
                s.total.spend(bytes);
                return Ok(Allowance {
                    user: self,
                    bytes,
                    used: 0,
                });
            }
            let _wait = self
                .owner
                .changed
                .wait_timeout(s, Duration::from_millis(25))
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}
impl Allowance<'_> {
    pub fn finish(mut self, used: usize) {
        self.used = used.min(self.bytes);
        self.user
            .owner
            .payload_bytes
            .fetch_add(self.used as u64, Ordering::Relaxed);
        if self.used > 0 {
            let mut users = self
                .user
                .owner
                .payload_users
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *users.bytes.entry(self.user.name.clone()).or_default() += self.used as u64;
            if let Some(measurement) = users.uploads.get_mut(&self.user.measurement_name) {
                measurement.record(self.used as u64, Instant::now());
            }
        }
        self.user
            .transferred
            .fetch_add(self.used as u64, Ordering::Relaxed);
    }
}
impl Drop for Allowance<'_> {
    fn drop(&mut self) {
        let unused = self.bytes - self.used;
        if unused == 0 {
            return;
        }
        let mut s = self
            .user
            .owner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        s.total.refund(unused);
        if let Some(user) = s.users.get_mut(&self.user.name) {
            user.bucket.refund(unused);
        }
        self.user.owner.changed.notify_all();
    }
}
impl Drop for UserBandwidth {
    fn drop(&mut self) {
        let mut s = self.owner.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = s.users.get_mut(&self.name) {
            user.references -= 1;
            if user.references == 0 {
                user.idle_since = Some(Instant::now());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_time_is_the_union_of_parallel_files_and_sampling_never_counts_it_twice() {
        let at = Instant::now();
        let mut user = UploadMeasurement::default();
        user.begin(at);
        user.record(100, at + Duration::from_secs(1));
        user.begin(at + Duration::from_secs(2));
        user.record(200, at + Duration::from_secs(3));
        let first = user.take("alice", at + Duration::from_secs(4)).unwrap();
        assert_eq!((first.bytes, first.seconds), (300, 4.));
        user.end(at + Duration::from_secs(5));
        user.end(at + Duration::from_secs(8));
        let second = user.take("alice", at + Duration::from_secs(20)).unwrap();
        assert_eq!((second.bytes, second.seconds), (0, 4.));
        assert!(second.last_payload.is_none());
        assert_eq!(
            first.seconds + second.seconds,
            8.,
            "not 11 summed file-seconds or 20 idle-inclusive seconds"
        );
        assert!(user.take("alice", at + Duration::from_secs(30)).is_none());
    }
    #[test]
    fn idle_gaps_and_empty_attempts_add_no_upload_samples() {
        let at = Instant::now();
        let mut user = UploadMeasurement::default();
        user.begin(at);
        assert!(user.take("alice", at + Duration::from_secs(1)).is_none());
        user.record(200, at + Duration::from_secs(2));
        user.end(at + Duration::from_secs(2));
        user.begin(at + Duration::from_secs(100));
        user.record(300, at + Duration::from_secs(103));
        user.end(at + Duration::from_secs(103));
        user.begin(at + Duration::from_secs(104));
        user.end(at + Duration::from_secs(134)); // a separate zero-byte failure
        let sample = user.take("alice", at + Duration::from_secs(200)).unwrap();
        assert_eq!((sample.bytes, sample.seconds), (500, 5.));
        let mut empty = UploadMeasurement::default();
        empty.begin(at);
        empty.end(at + Duration::from_secs(30));
        assert!(empty.take("empty", at + Duration::from_secs(31)).is_none());
    }
    #[test]
    fn recipient_measurements_preserve_payload_accounting_and_merge_username_case() {
        let limits = Arc::new(Bandwidth::default());
        let alice = limits.for_user("alice");
        let alias = limits.for_user("ALICE");
        {
            let _one = alice.begin_upload();
            let _two = alias.begin_upload();
            alice.take(100, || false).unwrap().finish(100);
            alias.take(200, || false).unwrap().finish(200);
        }
        assert_eq!(limits.payload_bytes(), 300);
        assert_eq!(limits.take_payload_users().values().sum::<u64>(), 300);
        let samples = limits.take_upload_rate_samples();
        assert_eq!(samples.len(), 1);
        assert_eq!(
            (samples[0].username.as_str(), samples[0].bytes),
            ("alice", 300)
        );
        assert!(samples[0].seconds > 0.);
        assert!(samples[0].last_payload.is_some());
        assert!(limits.take_upload_rate_samples().is_empty());
        assert!(limits.payload_users.lock().unwrap().uploads.is_empty());
    }
    #[test]
    fn one_users_files_share_one_bucket_and_other_users_have_independent_caps() {
        let limits = Arc::new(Bandwidth::default());
        limits.configure(0, 10000, vec![]);
        let a = limits.for_user("alice");
        let a2 = limits.for_user("alice");
        let b = limits.for_user("bob");
        let grant = a.take(1000, || false).unwrap();
        assert_eq!(grant.bytes, 1000);
        grant.finish(1000);
        assert!(limits.state.lock().unwrap().users["alice"].bucket.tokens < 1.0);
        let grant = b.take(1000, || false).unwrap();
        assert_eq!(grant.bytes, 1000);
        grant.finish(1000);
        assert_eq!(limits.state.lock().unwrap().users["alice"].references, 2);
        drop(a);
        drop(a2);
        drop(b);
        assert!(
            limits
                .state
                .lock()
                .unwrap()
                .users
                .values()
                .all(|u| u.references == 0)
        );
    }
    #[test]
    fn total_and_user_budgets_apply_together_and_refund_short_reads() {
        let limits = Arc::new(Bandwidth::default());
        limits.configure(5000, 2000, vec![("bob".into(), 1000)]);
        let a = limits.for_user("alice");
        let b = limits.for_user("bob");
        let grant = a.take(9000, || false).unwrap();
        assert_eq!(grant.bytes, 200);
        grant.finish(50);
        assert_eq!(limits.state.lock().unwrap().total.tokens, 450.0);
        let grant = b.take(9000, || false).unwrap();
        assert_eq!(grant.bytes, 100);
        grant.finish(100);
        assert!(limits.state.lock().unwrap().total.tokens <= 351.0);
    }
    #[test]
    fn real_concurrent_payloads_obey_one_total_budget() {
        let limits = Arc::new(Bandwidth::default());
        limits.configure(12000, 0, vec![]);
        let started = Instant::now();
        std::thread::scope(|s| {
            for name in ["alice", "bob", "alice"] {
                let limits = limits.clone();
                s.spawn(move || {
                    let user = limits.for_user(name);
                    let mut left = 4000;
                    while left > 0 {
                        let grant = user.take(left, || false).unwrap();
                        let n = grant.bytes;
                        grant.finish(n);
                        left -= n;
                    }
                });
            }
        });
        assert!(
            started.elapsed() >= Duration::from_millis(850),
            "shared rate cap was exceeded: {:?}",
            started.elapsed()
        );
    }
    #[test]
    fn changing_limits_and_cancelling_wakes_waiters_without_leaking_users() {
        let limits = Arc::new(Bandwidth::default());
        limits.configure(10, 10, vec![]);
        let user = limits.for_user("alice");
        user.take(1, || false).unwrap().finish(1);
        std::thread::scope(|s| {
            let thread = s.spawn(|| user.take(500, || false).unwrap().bytes);
            std::thread::sleep(Duration::from_millis(10));
            limits.configure(0, 0, vec![]);
            assert_eq!(thread.join().unwrap(), 500);
        });
        assert!(user.take(100, || true).is_err());
        drop(user);
        assert!(
            limits
                .state
                .lock()
                .unwrap()
                .users
                .values()
                .all(|u| u.references == 0)
        );
    }
    #[test]
    fn named_overrides_update_existing_handles() {
        let limits = Arc::new(Bandwidth::default());
        let user = limits.for_user("alice");
        limits.configure(0, 1000, vec![("alice".into(), 2000)]);
        let grant = user.take(9000, || false).unwrap();
        assert_eq!(grant.bytes, 200);
        drop(grant);
        limits.configure(0, 1000, vec![]);
        assert_eq!(user.take(9000, || false).unwrap().bytes, 100);
    }
    #[test]
    fn concurrent_files_from_the_same_user_share_the_per_user_speed_cap() {
        let limits = Arc::new(Bandwidth::default());
        limits.configure(0, 12000, vec![]);
        let started = Instant::now();
        std::thread::scope(|scope| {
            for name in ["alice", "alice", "bob"] {
                let limits = limits.clone();
                scope.spawn(move || {
                    let user = limits.for_user(name);
                    let mut left = 12000;
                    while left > 0 {
                        let grant = user.take(left, || false).unwrap();
                        let n = grant.bytes;
                        grant.finish(n);
                        left -= n;
                    }
                });
            }
        });
        assert!(
            started.elapsed() >= Duration::from_millis(1750),
            "Each file incorrectly received its own user budget: {:?}",
            started.elapsed()
        );
    }
}
