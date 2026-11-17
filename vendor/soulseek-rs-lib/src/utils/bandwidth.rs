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

#[derive(Default)]
pub struct Bandwidth {
    payload_bytes: AtomicU64,
    payload_users: Mutex<HashMap<String, u64>>,
    state: Mutex<State>,
    changed: Condvar,
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
        std::mem::take(&mut *self.payload_users.lock().unwrap_or_else(|e| e.into_inner()))
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
            transferred: Arc::default(),
        }
    }
}
pub struct UserBandwidth {
    owner: Arc<Bandwidth>,
    name: String,
    pub transferred: Arc<AtomicU64>,
}
pub struct Allowance<'a> {
    user: &'a UserBandwidth,
    pub bytes: usize,
    used: usize,
}
impl UserBandwidth {
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
            *users.entry(self.user.name.clone()).or_default() += self.used as u64;
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
