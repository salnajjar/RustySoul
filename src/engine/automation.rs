use super::*;
use rusqlite::{Connection, OptionalExtension, params};
#[derive(Clone, Default)]
pub(super) struct RetryState {
    pub attempts: u8,
    pub next_at: Option<u64>,
}
pub(super) fn finished(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "interrupted" | "timed_out"
    )
}
pub(super) fn load_policy(db: &Connection) -> Result<RetryPolicy> {
    let policy: RetryPolicy = db
        .query_row("SELECT json FROM retry_policy WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    policy.validate().map_err(anyhow::Error::msg)?;
    Ok(policy)
}
pub(super) fn load_retries(db: &Connection) -> Result<BTreeMap<(String, String), RetryState>> {
    Ok(db
        .prepare("SELECT username,path,attempts,next_at FROM download_retries")?
        .query_map([], |r| {
            Ok((
                (r.get(0)?, r.get(1)?),
                RetryState {
                    attempts: r.get(2)?,
                    next_at: r.get::<_, Option<i64>>(3)?.map(|t| t as u64),
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}
pub(super) fn plan(policy: &RetryPolicy, state: &mut RetryState, time: u64) -> bool {
    if !policy.enabled || state.attempts >= policy.max_attempts {
        return false;
    }
    let delay = u64::from(policy.base_delay_seconds)
        .saturating_mul(1 << state.attempts)
        .min(u64::from(policy.max_delay_seconds));
    state.attempts += 1;
    state.next_at = Some(time.saturating_add(delay));
    true
}
pub(super) fn save(db: &Connection, user: &str, path: &str, state: &RetryState) -> Result<()> {
    db.execute("INSERT INTO download_retries VALUES(?1,?2,?3,?4) ON CONFLICT(username,path) DO UPDATE SET attempts=excluded.attempts,next_at=excluded.next_at",params![user,path,state.attempts,state.next_at.map(|n|n as i64)])?;
    Ok(())
}
pub(super) fn advance(
    policy: &RetryPolicy,
    queue: &mut [SavedDownload],
    retries: &mut BTreeMap<(String, String), RetryState>,
    time: u64,
) -> bool {
    if !policy.enabled {
        return false;
    }
    let mut changed = false;
    for row in queue {
        if row.status == "retry_waiting"
            && retries
                .get(&(row.username.clone(), row.path.clone()))
                .is_some_and(|s| s.next_at.is_some_and(|at| at <= time))
        {
            row.status = "waiting".into();
            changed = true;
        }
    }
    changed
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_budget_uses_bounded_exponential_delays_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::Store::open(&dir.path().join("db")).unwrap();
        let policy = RetryPolicy {
            enabled: true,
            uploads_enabled: true,
            max_attempts: 3,
            base_delay_seconds: 5,
            max_delay_seconds: 12,
        };
        let mut state = RetryState::default();
        assert!(plan(&policy, &mut state, 100));
        assert_eq!(state.next_at, Some(105));
        save(&db.connection, "alice", "music\\x.flac", &state).unwrap();
        drop(db);
        let db = crate::store::Store::open(&dir.path().join("db")).unwrap();
        let mut restored = load_retries(&db.connection).unwrap();
        let s = restored
            .get_mut(&("alice".into(), "music\\x.flac".into()))
            .unwrap();
        assert_eq!(s.attempts, 1);
        assert!(plan(&policy, s, 200));
        assert_eq!(s.next_at, Some(210));
        assert!(plan(&policy, s, 300));
        assert_eq!(s.next_at, Some(312));
        assert!(!plan(&policy, s, 400));
    }
}
