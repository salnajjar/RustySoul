//! Confirm shared dislikes using code-57 public interests, not a guess from topic users.
use crate::discovery::{Interests, Item, Person, View};
use anyhow::Result;
use rusqlite::Connection;
use soulseek_rs::{Client, DiscoveryEvent};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    time::{Duration, Instant},
};
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Evidence {
    pub checked: usize,
    pub matched: usize,
    pub cached: usize,
    pub requested: usize,
    pub timed_out: usize,
    pub limited: bool,
    pub missing_sources: usize,
}
pub(crate) struct Cohort {
    item: String,
    own: String,
    started: Instant,
    next_send: Instant,
    queue: VecDeque<String>,
    seen: HashSet<String>,
    excluded: HashSet<String>,
    pending: BTreeMap<String, Instant>,
    profiles: BTreeMap<String, (String, Interests)>,
    evidence: Evidence,
    topic_pending: bool,
    similar_pending: bool,
}
impl Cohort {
    pub fn new(db: &Connection, account: &str, item: &str, now: u64) -> Result<Self> {
        let mut c = Self {
            item: item.into(),
            own: account.to_lowercase(),
            started: Instant::now(),
            next_send: Instant::now(),
            queue: VecDeque::new(),
            seen: HashSet::new(),
            excluded: HashSet::new(),
            pending: BTreeMap::new(),
            profiles: BTreeMap::new(),
            evidence: Evidence::default(),
            topic_pending: true,
            similar_pending: true,
        };
        let mut stmt = db.prepare(
            "SELECT username,data FROM user_interests_cache ORDER BY updated_at DESC LIMIT 256",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let user: String = row.get(0)?;
            let data: String = row.get(1)?;
            let p: Interests = serde_json::from_str(&data)?;
            if user.to_lowercase() != c.own
                && p.updated_at > 0
                && now.saturating_sub(p.updated_at) <= 900
            {
                c.seen.insert(user.to_lowercase());
                c.profiles.insert(user.to_lowercase(), (user, p));
                c.evidence.cached += 1;
            } else {
                c.enqueue([user]);
            }
        }
        Ok(c)
    }
    pub fn enqueue(&mut self, users: impl IntoIterator<Item = String>) {
        for user in users {
            let key = user.to_lowercase();
            if key != self.own
                && !self.excluded.contains(&key)
                && crate::validation::username(&user).is_ok()
                && !self.seen.contains(&key)
            {
                if self.queue.len() >= 512 {
                    self.evidence.limited = true;
                    continue;
                }
                self.seen.insert(key);
                self.queue.push_back(user);
            }
        }
    }
    pub fn exclude(&mut self, users: Vec<String>) {
        self.excluded
            .extend(users.into_iter().map(|u| u.to_lowercase()));
        self.queue
            .retain(|u| !self.excluded.contains(&u.to_lowercase()));
        self.profiles.retain(|u, _| !self.excluded.contains(u));
        self.evidence.cached = self.profiles.len();
    }
    pub fn event(&mut self, event: &DiscoveryEvent) {
        match event {
            DiscoveryEvent::ItemUsers { item, usernames }
                if item.trim().to_lowercase() == self.item =>
            {
                self.topic_pending = false;
                self.enqueue(usernames.clone());
            }
            DiscoveryEvent::SimilarUsers(users) => {
                self.similar_pending = false;
                self.enqueue(users.iter().map(|u| u.username.clone()));
            }
            _ => {}
        }
    }
    pub fn poll(
        &mut self,
        db: &Connection,
        client: &Client,
        view: &mut View,
        now: u64,
    ) -> Result<(bool, bool)> {
        let mut changed = false;
        let pending: Vec<_> = self
            .pending
            .iter()
            .map(|(u, at)| (u.clone(), *at))
            .collect();
        for (user, at) in pending {
            if let Some(value) = client.user_interests(&user) {
                let profile = crate::discovery::save_interests(db, value, now)?;
                self.profiles
                    .insert(user.to_lowercase(), (user.clone(), profile));
                self.pending.remove(&user);
                changed = true;
            } else if at.elapsed() >= Duration::from_secs(10) {
                self.pending.remove(&user);
                self.evidence.timed_out += 1;
                changed = true;
            }
        }
        if self.started.elapsed() >= Duration::from_secs(10) {
            if self.topic_pending {
                self.evidence.missing_sources += 1;
                self.topic_pending = false;
            }
            if self.similar_pending {
                self.evidence.missing_sources += 1;
                self.similar_pending = false;
            }
        }
        if Instant::now() >= self.next_send {
            self.next_send = Instant::now() + Duration::from_millis(500);
            for _ in 0..2 {
                if self.evidence.requested >= 64 {
                    self.evidence.limited |= !self.queue.is_empty();
                    self.queue.clear();
                    break;
                }
                let Some(user) = self.queue.pop_front() else {
                    break;
                };
                self.evidence.requested += 1;
                if client.request_user_interests(&user).is_ok() {
                    self.pending.insert(user, Instant::now());
                } else {
                    self.evidence.timed_out += 1;
                }
                changed = true;
            }
        }
        let finished = (!self.topic_pending
            && !self.similar_pending
            && self.queue.is_empty()
            && self.pending.is_empty())
            || self.started.elapsed() >= Duration::from_secs(40);
        if finished {
            self.evidence.limited |= !self.queue.is_empty();
            self.queue.clear();
            self.evidence.timed_out += self.pending.len();
            self.pending.clear();
        }
        changed |= finished || view.dislike_evidence.is_none();
        if changed {
            self.render(view, now);
        }
        if finished {
            view.recommendations_pending = false;
            view.people_pending = false;
        }
        Ok((finished, changed))
    }
    pub fn render(&mut self, view: &mut View, now: u64) {
        let (people, likes, dislikes) = aggregate(&self.item, self.profiles.values());
        self.evidence.checked = self.profiles.len();
        self.evidence.matched = people.len();
        view.people = people;
        view.recommendations = likes;
        view.against = dislikes;
        view.people_at = Some(now);
        view.recommendations_at = Some(now);
        view.dislike_evidence = Some(self.evidence.clone());
    }
    pub fn cached_view(
        db: &Connection,
        account: &str,
        item: &str,
        view: &mut View,
        now: u64,
    ) -> Result<()> {
        let mut c = Self::new(db, account, item, now)?;
        c.render(view, now);
        Ok(())
    }
}
fn aggregate<'a>(
    item: &str,
    profiles: impl IntoIterator<Item = &'a (String, Interests)>,
) -> (Vec<Person>, Vec<Item>, Vec<Item>) {
    let mut people = Vec::new();
    let mut likes: BTreeMap<String, i32> = BTreeMap::new();
    let mut dislikes: BTreeMap<String, i32> = BTreeMap::new();
    for (username, p) in profiles {
        if !p
            .dislikes
            .iter()
            .any(|s| s.trim().eq_ignore_ascii_case(item))
        {
            continue;
        }
        people.push(Person {
            username: username.clone(),
            weight: Some(1),
        });
        for value in &p.likes {
            if value != item {
                *likes.entry(value.clone()).or_default() += 1;
            }
        }
        for value in &p.dislikes {
            if value != item {
                *dislikes.entry(value.clone()).or_default() -= 1;
            }
        }
    }
    people.sort_by_key(|p| p.username.to_lowercase());
    let mut likes: Vec<_> = likes
        .into_iter()
        .map(|(item, rating)| Item { item, rating })
        .collect();
    likes.sort_by(|a, b| b.rating.cmp(&a.rating).then(a.item.cmp(&b.item)));
    likes.truncate(500);
    let mut dislikes: Vec<_> = dislikes
        .into_iter()
        .map(|(item, rating)| Item { item, rating })
        .collect();
    dislikes.sort_by(|a, b| a.rating.cmp(&b.rating).then(a.item.cmp(&b.item)));
    dislikes.truncate(500);
    (people, likes, dislikes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_confirmed_shared_dislikes_contribute_to_people_and_topic_counts() {
        let profiles = vec![
            (
                "disliker".into(),
                Interests {
                    likes: vec!["jazz".into()],
                    dislikes: vec!["noise".into(), "spam".into()],
                    updated_at: 1,
                },
            ),
            (
                "liker".into(),
                Interests {
                    likes: vec!["noise".into(), "wrong".into()],
                    dislikes: vec![],
                    updated_at: 1,
                },
            ),
        ];
        let (people, likes, against) = aggregate("noise", &profiles);
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].username, "disliker");
        assert_eq!(likes[0].item, "jazz");
        assert_eq!(against[0].item, "spam");
        assert_eq!(against[0].rating, -1);
    }
    #[test]
    fn cohort_work_is_bounded_and_a_missing_profile_never_becomes_a_match() {
        let t = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&t.path().join("db")).unwrap();
        let mut c = Cohort::new(&store.connection, "self", "noise", 1).unwrap();
        c.enqueue((0..1000).map(|n| format!("user-{n}")));
        let client = Client::new("self", "password");
        let mut view = View {
            people_pending: true,
            recommendations_pending: true,
            ..Default::default()
        };
        for _ in 0..33 {
            c.next_send = Instant::now();
            let _ = c.poll(&store.connection, &client, &mut view, 1).unwrap();
        }
        assert_eq!(c.evidence.requested, 64);
        assert!(c.evidence.limited);
        c.started = Instant::now() - Duration::from_secs(41);
        let (done, _) = c.poll(&store.connection, &client, &mut view, 42).unwrap();
        assert!(done);
        assert!(view.people.is_empty());
        assert!(!view.people_pending);
        assert_eq!(view.dislike_evidence.unwrap().timed_out, 64);
    }
    #[test]
    fn cached_profiles_are_scoped_fresh_exclude_self_and_respect_blocks() {
        let t = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&t.path().join("db")).unwrap();
        for user in ["self", "friend", "blocked"] {
            crate::discovery::save_interests(
                &store.connection,
                soulseek_rs::UserInterests {
                    username: user.into(),
                    likes: vec![],
                    hates: vec!["noise".into()],
                },
                100,
            )
            .unwrap();
        }
        let mut c = Cohort::new(&store.connection, "self", "noise", 101).unwrap();
        c.exclude(vec!["BLOCKED".into()]);
        let mut view = View::default();
        c.render(&mut view, 101);
        assert_eq!(view.people.len(), 1);
        assert_eq!(view.people[0].username, "friend");
        let stale = Cohort::new(&store.connection, "self", "noise", 1002).unwrap();
        assert!(stale.profiles.is_empty());
    }
}
