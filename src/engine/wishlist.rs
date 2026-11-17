use super::*;
use rusqlite::{Connection, params};
pub(super) fn load(db: &Connection) -> Result<Vec<WishView>> {
    Ok(db.prepare("SELECT id,query,enabled,last_run,search_id,last_count FROM wishlist ORDER BY query COLLATE NOCASE,query")?.query_map([],|r|Ok(WishView{id:r.get(0)?,query:r.get(1)?,enabled:r.get(2)?,last_run:r.get::<_,Option<i64>>(3)?.map(|n|n as u64),search_id:r.get(4)?,last_count:r.get::<_,i64>(5)? as usize}))?.collect::<rusqlite::Result<_>>()?)
}
impl Engine {
    pub(super) fn run_wish(&mut self, id: &str) -> Result<()> {
        let c = self
            .client
            .as_ref()
            .context("Connect to Soulseek to run wishlist searches")?;
        let interval = c.wishlist_interval().as_secs();
        ensure!(interval > 0, "The server has disabled wishlist searches");
        ensure!(
            now() >= self.wishlist_last_sent.saturating_add(interval),
            "Wishlist rate limit: wait {} seconds before the next search",
            self.wishlist_last_sent
                .saturating_add(interval)
                .saturating_sub(now())
        );
        ensure!(
            !self
                .view
                .searches
                .iter()
                .any(|s| s.wishlist_id.is_some() && s.collecting),
            "Wait for the current wishlist search to finish"
        );
        let wish = self
            .view
            .wishlist
            .iter()
            .find(|w| w.id == id)
            .context("Wishlist entry not found")?;
        let key = wish
            .search_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let query = wish.query.clone();
        let total = self.store.connection.query_row(
            "SELECT COUNT(*) FROM search_results WHERE search_id=?1",
            [&key],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let mut view = SearchView {
            id: key.clone(),
            query: query.clone(),
            wishlist_id: Some(id.into()),
            collecting: true,
            total,
            results: crate::search::preview(&self.store.connection, &key)?,
            ..Default::default()
        };
        crate::search::create(&self.store.connection, &view)?;
        c.search_keyed(&key, &query, None, None, true)?;
        let expired: Vec<_> = self
            .view
            .searches
            .iter()
            .filter(|s| s.wishlist_id.is_some() && !s.pinned && !s.collecting && s.id != key)
            .map(|s| s.id.clone())
            .collect();
        for old in expired {
            self.view.searches.retain(|s| s.id != old);
            self.searches.remove(&old);
        }
        if let Some(old) = self.view.searches.iter_mut().find(|s| s.id == key) {
            view.pinned = old.pinned;
            view.revision = old.revision + 1;
            *old = view;
        } else {
            self.view.searches.push(view);
        }
        self.searches.insert(key.clone(), (query, Instant::now()));
        self.wishlist_last_sent = now();
        let tx = crate::store::write_transaction(&self.store.connection)?;
        tx.execute(
            "UPDATE wishlist SET last_run=?1,search_id=?2,last_count=?3 WHERE id=?4",
            params![now() as i64, key, total as i64, id],
        )?;
        tx.execute(
            "UPDATE wishlist_clock SET last_sent=?1 WHERE id=1",
            [now() as i64],
        )?;
        tx.commit()?;
        self.view.wishlist = load(&self.store.connection)?;
        self.view.wishlist_next_at = self.wishlist_last_sent.saturating_add(interval);
        Ok(())
    }
    pub(super) fn poll_wishlist(&mut self) {
        let Some(c) = &self.client else {
            self.view.wishlist_next_at = 0;
            return;
        };
        let interval = c.wishlist_interval().as_secs();
        self.view.wishlist_next_at = self.wishlist_last_sent.saturating_add(interval);
        if interval == 0
            || now() < self.view.wishlist_next_at
            || self
                .view
                .searches
                .iter()
                .any(|s| s.wishlist_id.is_some() && s.collecting)
        {
            return;
        }
        if let Some(id) = self
            .view
            .wishlist
            .iter()
            .filter(|w| w.enabled)
            .min_by_key(|w| w.last_run.unwrap_or(0))
            .map(|w| w.id.clone())
        {
            if let Err(e) = self.run_wish(&id) {
                self.view.error = Some(format!("Wishlist: {e}"));
                self.wishlist_last_sent = now();
            }
            self.publish();
        }
    }
    pub(super) fn open_wish(&mut self, id: &str) -> Result<()> {
        let wish = self
            .view
            .wishlist
            .iter()
            .find(|w| w.id == id)
            .context("Wishlist entry not found")?;
        let key = wish
            .search_id
            .clone()
            .context("This wish has not run yet")?;
        if self.view.searches.iter().any(|s| s.id == key && s.pinned) {
            return Ok(());
        }
        ensure!(
            self.view
                .searches
                .iter()
                .filter(|s| s.wishlist_id.is_none() || s.pinned)
                .count()
                < self.config.limits.searches,
            "Close an existing search before opening wishlist results"
        );
        if let Some(s) = self.view.searches.iter_mut().find(|s| s.id == key) {
            s.pinned = true;
        } else {
            let total = self.store.connection.query_row(
                "SELECT COUNT(*) FROM search_results WHERE search_id=?1",
                [&key],
                |r| r.get::<_, i64>(0),
            )? as usize;
            self.view.searches.push(SearchView {
                results: crate::search::preview(&self.store.connection, &key)?,
                id: key,
                query: wish.query.clone(),
                wishlist_id: Some(id.into()),
                pinned: true,
                total,
                ..Default::default()
            });
        }
        Ok(())
    }
}
