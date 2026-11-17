//! Bounded, on-demand discovery with SQLite caches. Empty replies are distinct
//! from pending requests; pending work never survives restart.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use soulseek_rs::{Client, DiscoveryEvent};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    Personal {},
    Popular {},
    Topic { item: String },
    Dislikes { item: String },
}
impl Default for Query {
    fn default() -> Self {
        Self::Personal {}
    }
}
impl Query {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if let Self::Topic { item } | Self::Dislikes { item } = self {
            crate::validation::text(item, 80)?;
        }
        Ok(())
    }
    pub fn normalized(self) -> Self {
        match self {
            Self::Topic { item } => Self::Topic {
                item: item.trim().to_lowercase(),
            },
            Self::Dislikes { item } => Self::Dislikes {
                item: item.trim().to_lowercase(),
            },
            other => other,
        }
    }
    fn key(&self) -> String {
        serde_json::to_string(self).expect("serializable query")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub item: String,
    pub rating: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Person {
    pub username: String,
    pub weight: Option<u32>,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub query: Query,
    pub dislike_evidence: Option<crate::discovery_dislikes::Evidence>,
    pub recommendations: Vec<Item>,
    pub against: Vec<Item>,
    pub people: Vec<Person>,
    pub recommendations_at: Option<u64>,
    pub people_at: Option<u64>,
    pub recommendations_pending: bool,
    pub people_pending: bool,
    pub recommendations_error: Option<String>,
    pub people_error: Option<String>,
}
impl View {
    pub fn pending(&self) -> bool {
        self.recommendations_pending || self.people_pending
    }
}

pub fn load(db: &Connection, account: &str, query: Option<&Query>) -> Result<View> {
    let data: Option<String> = if let Some(q) = query {
        db.query_row(
            "SELECT data FROM discovery_cache WHERE account=?1 AND scope=?2",
            params![account, q.key()],
            |r| r.get(0),
        )
        .optional()?
    } else {
        db.query_row("SELECT data FROM discovery_cache WHERE account=?1 ORDER BY updated_at DESC,rowid DESC LIMIT 1", [account], |r| r.get(0)).optional()?
    };
    Ok(match data {
        Some(data) => serde_json::from_str(&data).context("Invalid discovery cache")?,
        None => View {
            query: query.cloned().unwrap_or_default(),
            ..Default::default()
        },
    })
}
fn save(db: &Connection, account: &str, view: &View, now: u64) -> Result<()> {
    let mut saved = view.clone();
    saved.recommendations_pending = false;
    saved.people_pending = false;
    saved.recommendations_error = None;
    saved.people_error = None;
    let tx = crate::store::write_transaction(db)?;
    tx.execute("INSERT INTO discovery_cache(account,scope,data,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(account,scope) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at", params![account, saved.query.key(), serde_json::to_string(&saved)?, now.min(i64::MAX as u64) as i64])?;
    tx.execute("DELETE FROM discovery_cache WHERE rowid NOT IN (SELECT rowid FROM discovery_cache ORDER BY updated_at DESC,rowid DESC LIMIT 12)", [])?;
    tx.commit()?;
    Ok(())
}

#[derive(Default)]
pub struct Controller {
    requested: Option<Instant>,
    dislikes: Option<crate::discovery_dislikes::Cohort>,
}
impl Controller {
    pub fn begin(
        &mut self,
        db: &Connection,
        account: &str,
        client: Option<&Client>,
        view: &mut View,
        query: Query,
        refresh: bool,
    ) -> Result<()> {
        ensure!(
            !view.pending(),
            "Discovery is still updating. Please wait for it to finish."
        );
        let query = query.normalized();
        query.validate().map_err(anyhow::Error::msg)?;
        *view = load(db, account, Some(&query))?;
        if !refresh && (view.recommendations_at.is_some() || view.people_at.is_some()) {
            return Ok(());
        }
        if let Query::Dislikes { item } = &query {
            if client.is_none() {
                crate::discovery_dislikes::Cohort::cached_view(
                    db,
                    account,
                    item,
                    view,
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                )?;
                return Ok(());
            }
            let c = client.unwrap();
            let cohort = crate::discovery_dislikes::Cohort::new(
                db,
                account,
                item,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            )?;
            c.request_item_similar_users(item)?;
            c.request_similar_users()?;
            view.people_pending = true;
            view.recommendations_pending = true;
            view.people_error = None;
            view.recommendations_error = None;
            self.dislikes = Some(cohort);
            self.requested = Some(Instant::now());
            return Ok(());
        }
        let client = client.context(
            "Connect to Soulseek to update discovery. Cached results remain available offline.",
        )?;
        let _ = client.drain_discovery_events();
        let (recommendations, people) = match &query {
            Query::Personal {} => (
                client.request_recommendations(),
                Some(client.request_similar_users()),
            ),
            Query::Popular {} => (client.request_global_recommendations(), None),
            Query::Dislikes { .. } => unreachable!(),
            Query::Topic { item } => (
                client.request_item_recommendations(item),
                Some(client.request_item_similar_users(item)),
            ),
        };
        match recommendations {
            Ok(()) => view.recommendations_pending = true,
            Err(e) => view.recommendations_error = Some(e.to_string()),
        }
        if let Some(result) = people {
            match result {
                Ok(()) => view.people_pending = true,
                Err(e) => view.people_error = Some(e.to_string()),
            }
        }
        self.requested = view.pending().then(Instant::now);
        Ok(())
    }
    pub fn poll(
        &mut self,
        db: &Connection,
        account: &str,
        client: &Client,
        view: &mut View,
        now: u64,
    ) -> Result<()> {
        let mut changed = false;
        for event in client.drain_discovery_events() {
            if let Some(cohort) = &mut self.dislikes {
                cohort.event(&event);
            } else {
                changed |= apply(view, event, now);
            }
        }
        if let Some(cohort) = &mut self.dislikes {
            let (finished, changed) = cohort.poll(db, client, view, now)?;
            if finished {
                self.dislikes = None;
                self.requested = None;
            }
            if changed {
                save(db, account, view, now)?;
            }
            return Ok(());
        }
        if changed {
            save(db, account, view, now)?;
        }
        self.expire(view);
        if !view.pending() {
            self.requested = None;
        }
        Ok(())
    }
    pub fn seed_dislikes(&mut self, users: Vec<String>, excluded: Vec<String>) {
        if let Some(c) = &mut self.dislikes {
            c.exclude(excluded);
            c.enqueue(users);
        }
    }
    fn expire(&mut self, view: &mut View) {
        if self
            .requested
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(20))
        {
            self.finish(view, "The Soulseek server did not answer in time. Try Refresh; any previous results are kept.");
        }
    }
    pub fn finish(&mut self, view: &mut View, message: &str) {
        self.dislikes = None;
        if view.recommendations_pending {
            view.recommendations_error = Some(message.into());
        }
        if view.people_pending {
            view.people_error = Some(message.into());
        }
        view.recommendations_pending = false;
        view.people_pending = false;
        self.requested = None;
    }
}

fn items(values: Vec<soulseek_rs::Recommendation>, descending: bool) -> Vec<Item> {
    let mut unique: BTreeMap<String, Item> = BTreeMap::new();
    for value in values.into_iter().take(500) {
        let item = value.item.trim().to_owned();
        if crate::validation::text(&item, 80).is_ok() {
            unique
                .entry(item.to_lowercase())
                .and_modify(|old| {
                    if (descending && value.rating > old.rating)
                        || (!descending && value.rating < old.rating)
                    {
                        old.rating = value.rating;
                    }
                })
                .or_insert(Item {
                    item,
                    rating: value.rating,
                });
        }
    }
    let mut values: Vec<_> = unique.into_values().collect();
    values.sort_by(|a, b| {
        (if descending {
            b.rating.cmp(&a.rating)
        } else {
            a.rating.cmp(&b.rating)
        })
        .then(a.item.to_lowercase().cmp(&b.item.to_lowercase()))
    });
    values
}
fn people(values: Vec<Person>) -> Vec<Person> {
    let mut unique: BTreeMap<String, Person> = BTreeMap::new();
    for p in values.into_iter().take(500) {
        if crate::validation::username(&p.username).is_ok() {
            unique
                .entry(p.username.to_lowercase())
                .and_modify(|old| {
                    old.weight = old.weight.max(p.weight);
                })
                .or_insert(p);
        }
    }
    let mut values: Vec<_> = unique.into_values().collect();
    values.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then(a.username.to_lowercase().cmp(&b.username.to_lowercase()))
    });
    values
}
fn apply(view: &mut View, event: DiscoveryEvent, now: u64) -> bool {
    match event {
        DiscoveryEvent::Recommendations {
            global,
            recommended,
            unrecommended,
        } if view.recommendations_pending
            && ((global && view.query == (Query::Popular {}))
                || (!global && view.query == (Query::Personal {}))) =>
        {
            view.recommendations = items(recommended, true);
            view.against = items(unrecommended, false);
            view.recommendations_pending = false;
            view.recommendations_error = None;
            view.recommendations_at = Some(now);
        }
        DiscoveryEvent::ItemRecommendations {
            item,
            recommendations,
        } if view.recommendations_pending
            && view.query
                == (Query::Topic {
                    item: item.trim().to_lowercase(),
                }) =>
        {
            let (positive, negative) = recommendations.into_iter().partition(|r| r.rating >= 0);
            view.recommendations = items(positive, true);
            view.against = items(negative, false);
            view.recommendations_pending = false;
            view.recommendations_error = None;
            view.recommendations_at = Some(now);
        }
        DiscoveryEvent::SimilarUsers(users)
            if view.people_pending && view.query == (Query::Personal {}) =>
        {
            view.people = people(
                users
                    .into_iter()
                    .map(|p| Person {
                        username: p.username,
                        weight: Some(p.weight),
                    })
                    .collect(),
            );
            view.people_pending = false;
            view.people_error = None;
            view.people_at = Some(now);
        }
        DiscoveryEvent::ItemUsers { item, usernames }
            if view.people_pending
                && view.query
                    == (Query::Topic {
                        item: item.trim().to_lowercase(),
                    }) =>
        {
            view.people = people(
                usernames
                    .into_iter()
                    .map(|username| Person {
                        username,
                        weight: None,
                    })
                    .collect(),
            );
            view.people_pending = false;
            view.people_error = None;
            view.people_at = Some(now);
        }
        _ => return false,
    }
    true
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Interests {
    pub likes: Vec<String>,
    pub dislikes: Vec<String>,
    pub updated_at: u64,
}
pub fn load_interests(db: &Connection, username: &str) -> Result<Interests> {
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM user_interests_cache WHERE username=?1 COLLATE NOCASE",
            [username],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|data| serde_json::from_str(&data).map_err(Into::into))
        .unwrap_or_else(|| Ok(Interests::default()))
}
pub fn save_interests(
    db: &Connection,
    value: soulseek_rs::UserInterests,
    now: u64,
) -> Result<Interests> {
    let clean = |values: Vec<String>| {
        let mut values: Vec<_> = values
            .into_iter()
            .take(500)
            .map(|v| v.trim().to_lowercase())
            .filter(|v| crate::validation::text(v, 80).is_ok())
            .collect();
        values.sort();
        values.dedup();
        values
    };
    let interests = Interests {
        likes: clean(value.likes),
        dislikes: clean(value.hates),
        updated_at: now,
    };
    let tx = crate::store::write_transaction(db)?;
    tx.execute("INSERT INTO user_interests_cache(username,data,updated_at) VALUES(?1,?2,?3) ON CONFLICT(username) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at", params![value.username, serde_json::to_string(&interests)?, now.min(i64::MAX as u64) as i64])?;
    tx.execute("DELETE FROM user_interests_cache WHERE rowid NOT IN (SELECT rowid FROM user_interests_cache ORDER BY updated_at DESC,rowid DESC LIMIT 256)", [])?;
    tx.commit()?;
    Ok(interests)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recommendation(item: &str, rating: i32) -> soulseek_rs::Recommendation {
        soulseek_rs::Recommendation {
            item: item.into(),
            rating,
        }
    }

    #[test]
    fn replies_match_scope_and_empty_answers_replace_previous_results() {
        let mut view = View {
            query: Query::Topic {
                item: "jazz".into(),
            },
            recommendations_pending: true,
            people_pending: true,
            ..Default::default()
        };
        assert!(!apply(
            &mut view,
            DiscoveryEvent::Recommendations {
                global: false,
                recommended: vec![recommendation("wrong", 9)],
                unrecommended: vec![]
            },
            1
        ));
        assert!(!apply(
            &mut view,
            DiscoveryEvent::ItemUsers {
                item: "rock".into(),
                usernames: vec!["wrong".into()]
            },
            1
        ));
        assert!(view.pending());
        assert!(apply(
            &mut view,
            DiscoveryEvent::ItemRecommendations {
                item: "Jazz".into(),
                recommendations: vec![
                    recommendation("Blues", 2),
                    recommendation("blues", 8),
                    recommendation("pop", -3),
                    recommendation("invalid\nitem", 100)
                ]
            },
            10
        ));
        assert_eq!(view.recommendations.len(), 1);
        assert_eq!(view.recommendations[0].rating, 8);
        assert_eq!(view.against[0].rating, -3);
        assert!(view.people_pending);
        assert!(apply(
            &mut view,
            DiscoveryEvent::ItemUsers {
                item: "jazz".into(),
                usernames: vec![]
            },
            11
        ));
        assert!(!view.pending());
        assert_eq!(view.people_at, Some(11));
        view.recommendations_pending = true;
        assert!(apply(
            &mut view,
            DiscoveryEvent::ItemRecommendations {
                item: "jazz".into(),
                recommendations: vec![]
            },
            12
        ));
        assert!(view.recommendations.is_empty() && view.against.is_empty());
        assert!(!apply(
            &mut view,
            DiscoveryEvent::ItemRecommendations {
                item: "jazz".into(),
                recommendations: vec![recommendation("unsolicited", 1)]
            },
            13
        ));
    }
    #[test]
    fn people_are_validated_deduplicated_and_sorted_by_similarity() {
        let values = people(vec![
            Person {
                username: "alice".into(),
                weight: Some(2),
            },
            Person {
                username: "bob".into(),
                weight: Some(8),
            },
            Person {
                username: "ALICE".into(),
                weight: Some(9),
            },
            Person {
                username: "bad\nuser".into(),
                weight: Some(u32::MAX),
            },
        ]);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].username, "alice");
        assert_eq!(values[0].weight, Some(9));
    }
    #[test]
    fn caches_are_bounded_account_scoped_and_restart_without_pending_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let store = crate::store::Store::open(&path).unwrap();
            let db = &store.connection;
            for n in 0..20 {
                save(
                    db,
                    "alice",
                    &View {
                        query: Query::Topic {
                            item: format!("topic{n}"),
                        },
                        recommendations: vec![Item {
                            item: "kept".into(),
                            rating: 3,
                        }],
                        recommendations_at: Some(n),
                        recommendations_pending: true,
                        people_pending: true,
                        ..Default::default()
                    },
                    n,
                )
                .unwrap();
            }
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM discovery_cache", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                12
            );
            assert!(
                load(
                    db,
                    "alice",
                    Some(&Query::Topic {
                        item: "topic0".into()
                    })
                )
                .unwrap()
                .recommendations
                .is_empty()
            );
            assert!(load(db, "bob", None).unwrap().recommendations.is_empty());
        }
        let store = crate::store::Store::open(&path).unwrap();
        let view = load(&store.connection, "alice", None).unwrap();
        assert_eq!(
            view.query,
            Query::Topic {
                item: "topic19".into()
            }
        );
        assert_eq!(view.recommendations[0].item, "kept");
        assert!(!view.pending());
        let mut controller = Controller::default();
        let mut restored = View::default();
        controller
            .begin(
                &store.connection,
                "alice",
                None,
                &mut restored,
                view.query.clone(),
                false,
            )
            .unwrap();
        assert_eq!(restored.recommendations[0].item, "kept");
        assert!(
            controller
                .begin(
                    &store.connection,
                    "alice",
                    None,
                    &mut restored,
                    view.query,
                    true
                )
                .is_err()
        );
        assert_eq!(restored.recommendations[0].item, "kept");
    }
    #[test]
    fn disconnect_finishes_pending_sections_and_preserves_cached_data() {
        let mut controller = Controller {
            requested: Some(Instant::now()),
            ..Default::default()
        };
        let mut view = View {
            recommendations: vec![Item {
                item: "jazz".into(),
                rating: 9,
            }],
            recommendations_at: Some(4),
            recommendations_pending: true,
            people_pending: true,
            ..Default::default()
        };
        controller.finish(&mut view, "Disconnected");
        assert!(!view.pending());
        assert!(controller.requested.is_none());
        assert_eq!(view.recommendations[0].item, "jazz");
        assert_eq!(view.people_error.as_deref(), Some("Disconnected"));
        view.recommendations_pending = true;
        view.people_pending = true;
        controller.requested = Some(Instant::now() - Duration::from_secs(21));
        controller.expire(&mut view);
        assert!(!view.pending());
        assert!(
            view.recommendations_error
                .as_deref()
                .unwrap()
                .contains("did not answer in time")
        );
        assert_eq!(view.recommendations_at, Some(4));
    }
    #[test]
    fn public_interests_are_sanitized_and_cache_retention_is_bounded() {
        let store = crate::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let db = &store.connection;
        for n in 0..260 {
            save_interests(
                db,
                soulseek_rs::UserInterests {
                    username: format!("user{n}"),
                    likes: vec![" JAZZ ".into(), "jazz".into(), "bad\0text".into()],
                    hates: vec![],
                },
                n,
            )
            .unwrap();
        }
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM user_interests_cache", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            256
        );
        assert_eq!(load_interests(db, "USER259").unwrap().likes, vec!["jazz"]);
        assert!(load_interests(db, "user0").unwrap().likes.is_empty());
    }
    #[test]
    fn discovery_and_interest_commands_reject_invalid_or_ambiguous_inputs() {
        for input in [
            r#"{"type":"discover","query":{"mode":"topic","item":"\n"}}"#,
            r#"{"type":"discover","query":{"mode":"popular","item":"unexpected"}}"#,
            r#"{"type":"set_interest","item":"jazz","disposition":"surprise"}"#,
        ] {
            let command = serde_json::from_str::<crate::model::Command>(input);
            assert!(command.is_err() || command.unwrap().validate().is_err());
        }
        assert_eq!(
            Query::Topic {
                item: " JAZZ ".into()
            }
            .normalized(),
            Query::Topic {
                item: "jazz".into()
            }
        );
    }
}
