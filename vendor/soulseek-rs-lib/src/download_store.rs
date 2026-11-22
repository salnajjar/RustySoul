use crate::types::{Download, DownloadStatus};

pub struct DownloadStore {
    downloads: Vec<Download>,
    limits: (usize, usize),
    offers: std::collections::HashMap<u32, std::time::Instant>,
    deferred: Vec<(String, String)>,
}
impl Default for DownloadStore {
    fn default() -> Self { Self { downloads: Vec::new(), limits: (usize::MAX, usize::MAX), offers: Default::default(), deferred: Vec::new() } }
}

impl DownloadStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_limits(&mut self, total: usize, per_user: usize) { self.limits = (total, per_user); }
    pub fn usage(&self) -> (usize, std::collections::HashMap<String, usize>) {
        let mut users = std::collections::HashMap::new(); let mut total = 0;
        for d in &self.downloads {
            if matches!(d.status, DownloadStatus::InProgress { .. } | DownloadStatus::Paused { .. }) || (matches!(d.status, DownloadStatus::Queued) && self.offers.contains_key(&d.token)) {
                total += 1; *users.entry(d.username.to_lowercase()).or_default() += 1;
            }
        }
        (total, users)
    }
    fn has_slot(&self, user: &str) -> bool {
        let (total, users) = self.usage();
        total < self.limits.0 && users.get(&user.to_lowercase()).copied().unwrap_or(0) < self.limits.1
    }
    fn defer(&mut self, user: &str, file: &str) {
        if !self.deferred.iter().any(|(u,p)|u==user && p==file) { self.deferred.push((user.into(),file.into())); }
        self.remove_by_file(user,file);
    }
    pub fn take_deferred(&mut self) -> Vec<(String, String)> { std::mem::take(&mut self.deferred) }
    pub fn expire_offers(&mut self, now: std::time::Instant) {
        let expired: Vec<_> = self.offers.iter().filter(|(_,at)|now.saturating_duration_since(**at)>=std::time::Duration::from_secs(30)).map(|(token,_)|*token).collect();
        for token in expired {
            self.offers.remove(&token);
            if let Some(d) = self.get_by_token_mut(token) {
                if matches!(d.status, DownloadStatus::Queued) { d.status=DownloadStatus::TimedOut; let _=d.sender.send(DownloadStatus::TimedOut); }
            }
        }
        self.offers.retain(|token,_|self.downloads.iter().any(|d|d.token==*token && matches!(d.status,DownloadStatus::Queued)));
    }
    pub fn add(&mut self, download: Download) {
        self.downloads.retain(|d| {
            !(d.is_finished()
                && d.username == download.username
                && d.filename == download.filename)
        });
        self.downloads.push(download);
    }

    pub fn remove(&mut self, token: u32) {
        self.downloads.retain(|d| d.token != token);
    }

    #[must_use]
    pub fn get_by_token(&self, token: u32) -> Option<&Download> {
        self.downloads.iter().find(|d| d.token == token)
    }

    pub fn get_by_token_mut(&mut self, token: u32) -> Option<&mut Download> {
        self.downloads.iter_mut().find(|d| d.token == token)
    }

    /// The live entry for a file wins over a finished one, so queueing the
    /// same file twice and cancelling once leaves the second request reachable.
    #[must_use]
    pub fn get_by_file(
        &self,
        username: &str,
        filename: &str,
    ) -> Option<&Download> {
        self.downloads
            .iter()
            .filter(|d| d.username == username && d.filename == filename)
            .min_by_key(|d| d.is_finished())
    }

    pub fn get_by_file_mut(
        &mut self,
        username: &str,
        filename: &str,
    ) -> Option<&mut Download> {
        self.downloads
            .iter_mut()
            .filter(|d| d.username == username && d.filename == filename)
            .min_by_key(|d| d.is_finished())
    }

    /// Register an offer only for a requested, idle download. Updating in place
    /// also handles peers that reuse our token without deleting that entry.
    pub fn accept_offer(&mut self, username: &str, filename: &str, token: u32, size: u64) -> Result<(), &'static str> {
        if self.downloads.iter().any(|d| d.token == token && (d.username != username || d.filename != filename)) {
            return Err("Transfer token is already in use");
        }
        self.expire_offers(std::time::Instant::now());
        let download = self.get_by_file(username, filename).ok_or("File not requested")?;
        if matches!(download.status, DownloadStatus::Cancelled) { return Err("Cancelled"); }
        if download.is_finished() { return Err("File no longer requested"); }
        if !matches!(download.status, DownloadStatus::Queued) || self.offers.contains_key(&download.token) { return Err("Transfer already active, offered or paused"); }
        // Some SoulseekQt peers send zero in offers for files over 2 GiB.
        // Keep the requested length in that case; zero must not publish an
        // empty replacement for a large file. Small genuinely empty files work.
        let size = if size == 0 && download.size > i32::MAX as u64 { download.size } else { size };
        if size > 16 * 1024_u64.pow(4) { return Err("Invalid offered file size (maximum 16 TiB)"); }
        if !self.has_slot(username) {
            self.defer(username,filename);
            return Err("No free download slot; please wait for a new request");
        }
        let download = self.get_by_file_mut(username,filename).unwrap();
        if download.size != size {
            crate::peer::reset_idle_partial(download).map_err(|_| "File size changed, but the old partial file is busy or cannot be safely reset; retry after resolving local storage access")?;
            download.progress.set(0);
            download.size = size;
        }
        download.token = token;
        self.offers.insert(token, std::time::Instant::now());
        Ok(())
    }

    #[must_use]
    pub fn tokens(&self) -> Vec<u32> {
        self.downloads.iter().map(|d| d.token).collect()
    }

    #[must_use]
    pub const fn list(&self) -> &Vec<Download> {
        &self.downloads
    }

    pub fn update_status(
        &mut self,
        token: u32,
        status: DownloadStatus,
    ) -> bool {
        let Some(download) = self.get_by_token_mut(token) else {
            return false;
        };
        if download.status.is_terminal()
            || matches!(
                (&download.status, &status),
                (
                    DownloadStatus::Paused { .. },
                    DownloadStatus::InProgress { .. }
                )
            )
        {
            return false;
        }
        download.status = status;
        true
    }

    /// Atomically reserve a queued download for the peer that owns it.
    ///
    /// Every file-connection direction uses this before opening a partial, so
    /// two sockets cannot both observe `Queued` and write the same target.
    pub fn claim_for_peer(
        &mut self,
        token: u32,
        username: &str,
    ) -> Option<Download> {
        self.expire_offers(std::time::Instant::now());
        let download = self.get_by_token(token)?;
        if download.username != username || !matches!(download.status, DownloadStatus::Queued) { return None; }
        if !self.offers.contains_key(&token) && !self.has_slot(username) {
            let filename=download.filename.clone(); self.defer(username,&filename); return None;
        }
        self.offers.remove(&token);
        let download = self.get_by_token_mut(token)?;
        download.status = DownloadStatus::InProgress {
            bytes_downloaded: 0,
            total_bytes: download.size,
            speed_bytes_per_sec: 0.0,
        };
        Some(download.clone())
    }

    pub fn cancel_by_file(
        &mut self,
        username: &str,
        filename: &str,
    ) -> Option<Download> {
        let download = self.get_by_file_mut(username, filename)?;
        if download.is_finished() {
            return None;
        }
        download.status = DownloadStatus::Cancelled;
        let _ = download.sender.send(DownloadStatus::Cancelled);
        Some(download.clone())
    }

    pub fn update_queue_position(
        &mut self,
        username: &str,
        filename: &str,
        position: u32,
    ) -> bool {
        let Some(download) = self.get_by_file_mut(username, filename) else {
            return false;
        };
        download.queue_position = Some(position);
        true
    }

    pub fn remove_queued_by_file(
        &mut self,
        username: &str,
        filename: &str,
    ) -> bool {
        let Some(index) = self.downloads.iter().position(|download| {
            download.username == username
                && download.filename == filename
                && matches!(download.status, DownloadStatus::Queued)
        }) else {
            return false;
        };

        self.downloads.remove(index);
        true
    }

    /// Remove every download matching `username`/`filename` regardless of
    /// status. Used before retrying a failed download so the stale entry (whose
    /// md5-derived token collides with the retry's) can't shadow the fresh one.
    /// Returns whether anything was removed.
    pub fn remove_by_file(&mut self, username: &str, filename: &str) -> bool {
        let before = self.downloads.len();
        self.downloads
            .retain(|d| !(d.username == username && d.filename == filename));
        self.downloads.len() != before
    }

    pub fn pause_by_file(&mut self, username: &str, filename: &str) -> bool {
        let Some(download) = self.get_by_file_mut(username, filename) else {
            return false;
        };

        let paused_status = match &download.status {
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                ..
            } => DownloadStatus::Paused {
                bytes_downloaded: *bytes_downloaded,
                total_bytes: *total_bytes,
            },
            DownloadStatus::Paused { .. } => return true,
            _ => return false,
        };

        download.status = paused_status.clone();
        let _ = download.sender.send(paused_status);
        true
    }

    pub fn resume_by_file(&mut self, username: &str, filename: &str) -> bool {
        let Some(download) = self.get_by_file_mut(username, filename) else {
            return false;
        };
        if download.progress.is_disconnected() {
            return false;
        }

        let resumed_status = match &download.status {
            DownloadStatus::Paused {
                bytes_downloaded,
                total_bytes,
            } => DownloadStatus::InProgress {
                bytes_downloaded: *bytes_downloaded,
                total_bytes: *total_bytes,
                speed_bytes_per_sec: 0.0,
            },
            DownloadStatus::InProgress { .. } => return true,
            _ => return false,
        };

        download.status = resumed_status.clone();
        let _ = download.sender.send(resumed_status);
        true
    }
}

/// Returns the tokens of queued downloads matching `username` (and optionally
/// a `filename`) after notifying their senders of `Failed`.
///
/// Caller is responsible for then calling `update_status` and `remove` for
/// each token, typically under a write lock.
#[must_use]
pub fn collect_failed_tokens(
    store: &DownloadStore,
    username: &str,
    filename: Option<&str>,
) -> Vec<u32> {
    store
        .list()
        .iter()
        .filter(|d| {
            d.username == username && filename.is_none_or(|f| d.filename == *f)
        })
        .filter(|d| matches!(d.status, DownloadStatus::Queued))
        .map(|d| {
            let _ = d.sender.send(DownloadStatus::Failed(Some(
                "The upload failed on the other side".to_string(),
            )));
            d.token
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_queues_use_no_slots_but_accepted_offers_reserve_them_atomically() {
        let mut store=DownloadStore::new(); store.set_limits(2,1);
        for (token,user) in [(1,"Alice"),(2,"alice"),(3,"bob"),(4,"carol")] {
            let mut d=make_download(token,DownloadStatus::Queued); d.username=user.into(); store.add(d);
        }
        assert_eq!(store.usage().0,0);
        assert!(store.accept_offer("Alice","file-1.mp3",11,100).is_ok());
        assert_eq!(store.usage().0,1);
        assert!(store.accept_offer("alice","file-2.mp3",12,100).is_err());
        assert_eq!(store.take_deferred(),vec![("alice".into(),"file-2.mp3".into())]);
        assert!(store.accept_offer("bob","file-3.mp3",13,100).is_ok());
        assert_eq!(store.usage().0,2);
        assert!(store.accept_offer("carol","file-4.mp3",14,100).is_err());
        assert!(store.claim_for_peer(11,"Alice").is_some());
        assert!(store.claim_for_peer(11,"Alice").is_none());
        assert_eq!(store.usage().0,2);
        store.update_status(11,DownloadStatus::Completed);
        assert_eq!(store.usage().0,1);
        store.expire_offers(std::time::Instant::now()+std::time::Duration::from_secs(31));
        assert!(matches!(store.get_by_token(13).unwrap().status,DownloadStatus::TimedOut));
        assert_eq!(store.usage().0,0);
    }
    #[test]
    fn unreserved_file_connections_cannot_bypass_download_limits_or_a_zero_slot_pause() {
        let mut store=DownloadStore::new();store.set_limits(1,1);
        store.add(make_download(1,DownloadStatus::Queued));store.add(make_download(2,DownloadStatus::Queued));
        assert!(store.claim_for_peer(1,"peer").is_some());
        assert!(store.claim_for_peer(2,"peer").is_none());
        assert_eq!(store.usage().0,1);
        store.update_status(1,DownloadStatus::Completed);store.set_limits(0,1);
        store.add(make_download(3,DownloadStatus::Queued));
        assert!(store.accept_offer("peer","file-3.mp3",3,100).is_err());
        assert_eq!(store.usage().0,0);
    }
    use crate::types::DownloadMetadata;
    use std::sync::mpsc;

    fn make_download(token: u32, status: DownloadStatus) -> Download {
        Download {
            username: "peer".to_string(),
            filename: format!("file-{token}.mp3"),
            token,
            size: 100,
            download_directory: "test".to_string(),
            status,
            sender: mpsc::channel().0,
            queue_position: None,
            metadata: DownloadMetadata::default(),
            progress: Default::default(),
        }
    }

    #[test]
    fn offers_preserve_reused_tokens_and_reject_unsolicited_or_duplicate_payloads(){
        let mut store=DownloadStore::new();store.add(make_download(7,DownloadStatus::Queued));
        assert!(store.accept_offer("peer","file-7.mp3",7,100).is_ok());assert_eq!(store.list().len(),1);assert!(store.claim_for_peer(7,"peer").is_some());
        assert!(store.accept_offer("peer","file-7.mp3",8,100).is_err());assert!(store.accept_offer("other","file-7.mp3",7,100).is_err());assert!(store.accept_offer("peer","not-requested",8,100).is_err());
        let dir=OfferDir::new();let mut first=make_download(7,DownloadStatus::Queued);first.download_directory=dir.path().to_string_lossy().into_owned();let mut store=DownloadStore::new();store.add(first);store.add(make_download(8,DownloadStatus::Queued));assert!(store.accept_offer("peer","file-7.mp3",8,100).is_err());assert!(store.accept_offer("peer","file-7.mp3",9,101).is_ok());assert_eq!(store.get_by_token(9).unwrap().size,101);assert!(store.get_by_token(7).is_none());assert!(store.claim_for_peer(9,"peer").is_some());
    }
    struct OfferDir(std::path::PathBuf);
    impl OfferDir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
            let path=std::env::temp_dir().join(format!("soulseek-offer-{}-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();Self(path)
        }
        fn path(&self)->&std::path::Path { &self.0 }
    }
    impl Drop for OfferDir { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
    #[test]
    fn changed_offer_size_resets_old_bytes_and_rejects_huge_or_busy_offers() {
        let dir = OfferDir::new();
        let mut d = make_download(1,DownloadStatus::Queued);
        d.download_directory = dir.path().to_string_lossy().into_owned();
        let part = dir.path().join("file-1.mp3.part");
        std::fs::write(&part,b"obsolete tags and audio").unwrap();
        let mut store = DownloadStore::new(); store.add(d);
        // Invalid and unsolicited offers cannot discard existing payload.
        assert!(store.accept_offer("other","file-1.mp3",11,101).is_err());
        assert!(store.accept_offer("peer","file-1.mp3",11,u64::MAX).is_err());
        assert_eq!(std::fs::read(&part).unwrap(),b"obsolete tags and audio");
        let writer = std::fs::OpenOptions::new().read(true).write(true).open(&part).unwrap();
        writer.try_lock().unwrap();
        assert!(store.accept_offer("peer","file-1.mp3",11,101).is_err());
        assert_eq!(store.get_by_token(1).unwrap().size,100);
        assert_eq!(store.usage().0,0);
        drop(writer);
        assert!(store.accept_offer("peer","file-1.mp3",11,101).is_ok());
        assert!(!part.exists());
        let accepted=store.get_by_token(11).unwrap();
        assert_eq!(accepted.size,101); assert_eq!(accepted.progress_bytes(),Some(0));
        assert_eq!(store.usage().0,1);
    }
    #[test]
    fn unchanged_offer_keeps_resumable_payload_and_full_slots_do_not_reset_it() {
        let dir = OfferDir::new();
        let mut d=make_download(1,DownloadStatus::Queued);
        d.download_directory=dir.path().to_string_lossy().into_owned();
        let part=dir.path().join("file-1.mp3.part");std::fs::write(&part,b"keep me").unwrap();
        let mut store=DownloadStore::new();store.add(d.clone());store.set_limits(0,1);
        assert!(store.accept_offer("peer","file-1.mp3",11,101).is_err());
        assert_eq!(std::fs::read(&part).unwrap(),b"keep me");
        store.add(d);store.set_limits(1,1);
        assert!(store.accept_offer("peer","file-1.mp3",11,100).is_ok());
        assert_eq!(std::fs::read(&part).unwrap(),b"keep me");
    }
    #[test]
    fn zero_size_large_file_quirk_preserves_the_known_length_and_partial() {
        let dir=OfferDir::new();let mut d=make_download(1,DownloadStatus::Queued);
        d.size=3*1024_u64.pow(3);d.download_directory=dir.path().to_string_lossy().into_owned();
        let part=dir.path().join("file-1.mp3.part");std::fs::write(&part,b"valid large-file prefix").unwrap();
        let mut store=DownloadStore::new();store.add(d);
        assert!(store.accept_offer("peer","file-1.mp3",11,0).is_ok());
        let claimed=store.claim_for_peer(11,"peer").unwrap();assert_eq!(claimed.size,3*1024_u64.pow(3));
        assert_eq!(std::fs::read(&part).unwrap(),b"valid large-file prefix");
    }
    #[cfg(unix)]
    #[test]
    fn changed_size_never_modifies_external_symlink_or_hardlink_contents() {
        let dir=OfferDir::new();let outside=dir.path().join("outside");
        std::fs::write(&outside,b"external payload").unwrap();
        let mut d=make_download(1,DownloadStatus::Queued);d.download_directory=dir.path().to_string_lossy().into_owned();
        let part=dir.path().join("file-1.mp3.part");std::os::unix::fs::symlink(&outside,&part).unwrap();
        let mut store=DownloadStore::new();store.add(d);
        assert!(store.accept_offer("peer","file-1.mp3",11,101).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(),b"external payload");
        std::fs::remove_file(&part).unwrap();std::fs::hard_link(&outside,&part).unwrap();
        assert!(store.accept_offer("peer","file-1.mp3",11,101).is_ok());
        assert!(!part.exists());assert_eq!(std::fs::read(&outside).unwrap(),b"external payload");
    }
    #[test]
    fn add_get_remove_roundtrip() {
        let mut store = DownloadStore::new();
        store.add(make_download(123, DownloadStatus::Queued));

        assert!(store.get_by_token(123).is_some());
        assert_eq!(store.tokens(), vec![123]);
        assert_eq!(store.list().len(), 1);

        store.remove(123);
        assert!(store.get_by_token(123).is_none());
        assert!(store.list().is_empty());
    }

    #[test]
    fn update_queue_position_sets_field_when_match() {
        let mut store = DownloadStore::new();
        let mut download = make_download(1, DownloadStatus::Queued);
        download.username = "peer".to_string();
        download.filename = "song.mp3".to_string();
        store.add(download);

        assert!(store.update_queue_position("peer", "song.mp3", 42));
        assert_eq!(store.get_by_token(1).unwrap().queue_position, Some(42));

        assert!(!store.update_queue_position("peer", "missing.mp3", 1));
        assert!(!store.update_queue_position("other", "song.mp3", 1));
    }

    #[test]
    fn cancel_by_file_reaches_the_second_request_for_the_same_file() {
        let mut store = DownloadStore::new();
        let mut first = make_download(1, DownloadStatus::Queued);
        first.filename = "song.mp3".to_string();
        let mut second = make_download(2, DownloadStatus::Queued);
        second.filename = "song.mp3".to_string();
        store.add(first);
        store.add(second);

        assert_eq!(store.cancel_by_file("peer", "song.mp3").unwrap().token, 1);
        assert_eq!(store.cancel_by_file("peer", "song.mp3").unwrap().token, 2);
        assert!(store.cancel_by_file("peer", "song.mp3").is_none());
    }

    #[test]
    fn get_by_file_prefers_the_live_entry_over_a_cancelled_one() {
        let mut store = DownloadStore::new();
        let mut cancelled = make_download(1, DownloadStatus::Cancelled);
        cancelled.filename = "song.mp3".to_string();
        let mut queued = make_download(2, DownloadStatus::Queued);
        queued.filename = "song.mp3".to_string();
        store.add(cancelled);
        store.add(queued);

        assert_eq!(store.get_by_file("peer", "song.mp3").unwrap().token, 2);
    }

    #[test]
    fn cancel_by_file_flips_an_unfinished_download_and_tells_its_channel() {
        let mut store = DownloadStore::new();
        let (tx, rx) = mpsc::channel();
        let mut queued = make_download(1, DownloadStatus::Queued);
        queued.sender = tx;
        store.add(queued);
        store.add(make_download(
            2,
            DownloadStatus::InProgress {
                bytes_downloaded: 5,
                total_bytes: 100,
                speed_bytes_per_sec: 1.0,
            },
        ));
        store.add(make_download(
            3,
            DownloadStatus::Paused {
                bytes_downloaded: 5,
                total_bytes: 100,
            },
        ));
        store.add(make_download(4, DownloadStatus::Completed));
        store.add(make_download(5, DownloadStatus::Failed(None)));

        assert!(store.cancel_by_file("peer", "file-1.mp3").is_some());
        assert!(matches!(rx.try_recv(), Ok(DownloadStatus::Cancelled)));
        assert!(store.cancel_by_file("peer", "file-2.mp3").is_some());
        assert!(store.cancel_by_file("peer", "file-3.mp3").is_some());
        assert!(store.cancel_by_file("peer", "file-4.mp3").is_none());
        assert!(store.cancel_by_file("peer", "file-5.mp3").is_none());
        assert!(store.cancel_by_file("peer", "file-9.mp3").is_none());
        for token in 1..=3 {
            let download = store.get_by_token(token).unwrap();
            assert!(matches!(download.status, DownloadStatus::Cancelled));
            assert!(download.is_finished());
        }
    }

    #[test]
    fn a_cancelled_download_stays_cancelled() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Queued));
        assert!(store.cancel_by_file("peer", "file-1.mp3").is_some());
        assert!(!store.update_status(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 0,
                total_bytes: 100,
                speed_bytes_per_sec: 0.0,
            },
        ));
        assert!(
            !store
                .update_status(1, DownloadStatus::Failed(Some("late".into())))
        );
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::Cancelled
        ));
        assert!(store.cancel_by_file("peer", "file-1.mp3").is_none());
        assert!(!store.resume_by_file("peer", "file-1.mp3"));
        assert!(!store.pause_by_file("peer", "file-1.mp3"));
    }

    #[test]
    fn add_evicts_a_finished_entry_for_the_same_file() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Cancelled));
        store.add(make_download(2, DownloadStatus::Queued));
        let mut fresh = make_download(3, DownloadStatus::Queued);
        fresh.filename = "file-1.mp3".to_string();
        store.add(fresh);
        let mut again = make_download(4, DownloadStatus::Queued);
        again.filename = "file-2.mp3".to_string();
        store.add(again);

        assert_eq!(store.tokens(), vec![2, 3, 4], "only the dead row goes");
        assert!(store.cancel_by_file("peer", "file-1.mp3").is_some());
        assert!(matches!(
            store.get_by_token(3).unwrap().status,
            DownloadStatus::Cancelled
        ));
    }

    #[test]
    fn pause_then_resume_in_progress_download() {
        let mut store = DownloadStore::new();
        let (tx, rx) = mpsc::channel();
        let mut download = make_download(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        );
        download.sender = tx;
        store.add(download);

        assert!(store.pause_by_file("peer", "file-1.mp3"));
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100
            }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100
            }
        ));

        assert!(store.resume_by_file("peer", "file-1.mp3"));
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 0.0
            }
        ));
    }

    #[test]
    fn progress_cannot_implicitly_resume_a_paused_download() {
        let mut store = DownloadStore::new();
        store.add(make_download(
            1,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100,
            },
        ));

        assert!(!store.update_status(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 50,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        ));
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100
            }
        ));
    }

    #[test]
    fn a_download_can_only_be_claimed_once_and_by_its_peer() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Queued));

        assert!(store.claim_for_peer(1, "other").is_none());
        let claimed = store.claim_for_peer(1, "peer").unwrap();
        assert!(matches!(
            claimed.status,
            DownloadStatus::InProgress {
                bytes_downloaded: 0,
                total_bytes: 100,
                speed_bytes_per_sec: 0.0
            }
        ));
        assert!(store.claim_for_peer(1, "peer").is_none());
    }

    #[test]
    fn remove_queued_skips_active_downloads() {
        let mut store = DownloadStore::new();
        store.add(make_download(123, DownloadStatus::Queued));
        store.add(make_download(
            456,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        ));
        // Override second download's filename so they don't collide
        store.get_by_token_mut(456).unwrap().filename =
            "active.mp3".to_string();
        store.get_by_token_mut(123).unwrap().filename =
            "queued.mp3".to_string();

        assert!(store.remove_queued_by_file("peer", "queued.mp3"));
        assert!(!store.remove_queued_by_file("peer", "active.mp3"));
        assert!(store.get_by_token(123).is_none());
        assert!(store.get_by_token(456).is_some());
    }

    #[test]
    fn remove_by_file_removes_regardless_of_status() {
        let mut store = DownloadStore::new();
        // A failed download (the retry case) plus a same-name duplicate that a
        // token-migration could have left behind — both must go.
        let mut failed = make_download(1, DownloadStatus::Failed(None));
        failed.filename = "song.mp3".to_string();
        store.add(failed);
        let mut dup = make_download(2, DownloadStatus::Queued);
        dup.filename = "song.mp3".to_string();
        store.add(dup);
        let mut other = make_download(3, DownloadStatus::Queued);
        other.filename = "other.mp3".to_string();
        store.add(other);

        assert!(store.remove_by_file("peer", "song.mp3"));
        assert!(store.get_by_token(1).is_none());
        assert!(store.get_by_token(2).is_none());
        assert!(store.get_by_token(3).is_some(), "other file untouched");
        assert!(!store.remove_by_file("peer", "song.mp3"), "idempotent");
    }

    #[test]
    fn collect_failed_tokens_collects_only_queued_downloads() {
        let mut store = DownloadStore::new();
        let (tx_active, rx_active) = mpsc::channel();
        let (tx_paused, rx_paused) = mpsc::channel();
        let (tx_completed, rx_completed) = mpsc::channel();
        let (tx_queued, _rx_queued) = mpsc::channel();

        let mut active = make_download(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        );
        active.sender = tx_active;

        let mut paused = make_download(
            2,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100,
            },
        );
        paused.sender = tx_paused;

        let mut completed = make_download(3, DownloadStatus::Completed);
        completed.sender = tx_completed;

        let mut queued = make_download(4, DownloadStatus::Queued);
        queued.sender = tx_queued;

        store.add(active);
        store.add(paused);
        store.add(completed);
        store.add(queued);

        let tokens = collect_failed_tokens(&store, "peer", None);

        assert_eq!(tokens, vec![4], "only the queued download may fail");
        assert!(
            rx_active.try_recv().is_err(),
            "an in-progress transfer must not be notified of failure"
        );
        assert!(
            rx_paused.try_recv().is_err(),
            "a paused transfer must not be notified of failure"
        );
        assert!(
            rx_completed.try_recv().is_err(),
            "a completed download must not be notified of failure"
        );
    }

    #[test]
    fn collect_failed_tokens_notifies_and_lists_matching() {
        let mut store = DownloadStore::new();
        let (tx_match, rx_match) = mpsc::channel();
        let (tx_other_user, _rx_other_user) = mpsc::channel();
        let (tx_other_file, _rx_other_file) = mpsc::channel();

        let mut a = make_download(1, DownloadStatus::Queued);
        a.sender = tx_match;
        a.username = "peer".to_string();
        a.filename = "song.mp3".to_string();

        let mut b = make_download(2, DownloadStatus::Queued);
        b.sender = tx_other_user;
        b.username = "other".to_string();
        b.filename = "song.mp3".to_string();

        let mut c = make_download(3, DownloadStatus::Queued);
        c.sender = tx_other_file;
        c.username = "peer".to_string();
        c.filename = "different.mp3".to_string();

        store.add(a);
        store.add(b);
        store.add(c);

        let tokens = collect_failed_tokens(&store, "peer", Some("song.mp3"));

        assert_eq!(tokens, vec![1]);
        assert!(matches!(
            rx_match.try_recv().unwrap(),
            DownloadStatus::Failed(_)
        ));
    }
}
