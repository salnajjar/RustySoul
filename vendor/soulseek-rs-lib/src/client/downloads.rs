use super::{
    Arc, Client, ClientContext, Download, DownloadMetadata, DownloadStatus,
    Receiver, Result, RwLock, RwLockExt, Sender, error, info, mpsc,
    next_download_token,
};
use crate::peer::discard_idle_partial;

impl Client {
    pub fn set_download_write_guard(&self, guard: Option<Arc<dyn crate::types::DownloadWriteGuard>>) {
        if let Ok(mut context)=self.context.write_safe(){context.download_write_guard=guard;}
    }
    pub fn take_payload_users(&self)->std::collections::HashMap<String,(u64,u64)> {
        let mut rows=std::collections::HashMap::new();
        if let Ok(c)=self.context.read_safe(){for (u,n) in c.download_speed_limit.take_payload_users(){rows.entry(u).or_insert((0,0)).0+=n;}for (u,n) in c.upload_speed_limit.take_payload_users(){rows.entry(u).or_insert((0,0)).1+=n;}}
        rows
    }

    /// Actual file payload read/written during this client lifetime, excluding resume offsets.
    pub fn payload_totals(&self)->(u64,u64) {
        self.context.read_safe().map(|c|(c.download_speed_limit.payload_bytes(),c.upload_speed_limit.payload_bytes())).unwrap_or_default()
    }
    #[must_use]
    pub fn get_all_downloads(&self) -> Vec<Download> {
        self.context
            .write_safe()
            .map(|mut ctx| { ctx.downloads.expire_offers(std::time::Instant::now()); ctx.get_downloads().clone() })
            .unwrap_or_default()
    }

    /// Active and accepted-file reservations, excluding remote queue waits.
    pub fn download_slot_usage(&self) -> (usize, std::collections::HashMap<String, usize>) {
        self.context.read_safe().map(|ctx|ctx.downloads.usage()).unwrap_or_default()
    }
    pub fn set_download_slots(&self, total: usize, per_user: usize) {
        if let Ok(mut ctx)=self.context.write_safe() { ctx.downloads.set_limits(total,per_user); }
    }
    /// Offers declined solely because local slots were full. Hosts requeue these
    /// without counting them as network failures or consuming retry attempts.
    pub fn take_deferred_downloads(&self) -> Vec<(String, String)> {
        self.context.write_safe().map(|mut ctx|ctx.downloads.take_deferred()).unwrap_or_default()
    }

    pub fn set_download_speed_limit(&self, bytes_per_second: u64) {
        if let Ok(ctx) = self.context.read_safe() {
            ctx.download_speed_limit.set_total(bytes_per_second);
        }
    }
    pub fn set_download_speed_limits(&self,total:u64,per_user:u64,overrides:Vec<(String,u64)>) {
        if let Ok(ctx)=self.context.read_safe(){ctx.download_speed_limit.configure(total,per_user,overrides);}
    }
    pub fn set_upload_speed_limits(&self,total:u64,per_user:u64,overrides:Vec<(String,u64)>) {
        if let Ok(ctx)=self.context.read_safe(){ctx.upload_speed_limit.configure(total,per_user,overrides);}
    }

    #[must_use]
    pub fn pause_download(&self, username: &str, filename: &str) -> bool {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.downloads.pause_by_file(username, filename),
            Err(e) => {
                error!("[client] pause_download: {}", e);
                false
            }
        }
    }

    #[must_use]
    pub fn resume_download(&self, username: &str, filename: &str) -> bool {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.downloads.resume_by_file(username, filename),
            Err(e) => {
                error!("[client] resume_download: {}", e);
                false
            }
        }
    }

    #[must_use]
    pub fn cancel_download(&self, username: &str, filename: &str) -> bool {
        let cancelled = match self.context.write_safe() {
            Ok(mut ctx) => ctx.downloads.cancel_by_file(username, filename),
            Err(e) => {
                error!("[client] cancel_download: {}", e);
                None
            }
        };
        let Some(download) = cancelled else {
            return false;
        };
        discard_idle_partial(&download);
        true
    }

    #[must_use]
    pub fn remove_queued_download(
        &self,
        username: &str,
        filename: &str,
    ) -> bool {
        match self.context.write_safe() {
            Ok(mut ctx) => {
                ctx.downloads.remove_queued_by_file(username, filename)
            }
            Err(e) => {
                error!("[client] remove_queued_download: {}", e);
                false
            }
        }
    }

    /// Remove every download for `username`/`filename` regardless of status.
    /// Useful before re-issuing [`Client::download`] for a failed download, so
    /// the list does not accumulate dead entries for the same file.
    ///
    /// Returns whether anything was removed.
    #[must_use]
    pub fn remove_download(&self, username: &str, filename: &str) -> bool {
        match self.context.write_safe() {
            Ok(mut ctx) => ctx.downloads.remove_by_file(username, filename),
            Err(e) => {
                error!("[client] remove_download: {}", e);
                false
            }
        }
    }

    pub fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        self.download_with_metadata(
            filename,
            username,
            size,
            download_directory,
            DownloadMetadata::default(),
        )
    }

    pub fn download_with_metadata(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
        metadata: DownloadMetadata,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        info!("[client] Downloading {} from {}", filename, username);

        let token = next_download_token();

        let (download_sender, download_receiver): (
            Sender<DownloadStatus>,
            Receiver<DownloadStatus>,
        ) = mpsc::channel();

        let download = Download {
            username: username.clone(),
            filename,
            token,
            size,
            download_directory,
            status: DownloadStatus::Queued,
            sender: download_sender,
            queue_position: None,
            metadata,
            progress: Default::default(),
        };

        let mut context = self.context.write_safe()?;
        context.add_download(download.clone());

        // If we already have a control connection to this peer, queue the
        // upload immediately. Otherwise open one directly (server GetPeerAddress
        // → outbound PeerInit → PeerConnected → the queued upload is flushed).
        let peer_registered = context
            .peer_registry
            .as_ref()
            .is_some_and(|r| r.contains(&username));
        let queued_now = peer_registered
            && context.peer_registry.as_ref().is_some_and(|r| {
                r.queue_upload(&username, download.filename.clone()).is_ok()
            });

        drop(context);

        let failed = if peer_registered {
            !queued_now
        } else {
            // No existing connection: initiate one. Only a genuinely
            // unconnected client (no server handle) fails outright here.
            self.connect_peer(&username).is_err()
        };

        if failed {
            let reason = if peer_registered {
                "The user declined the download"
            } else {
                "Could not connect to the user"
            };
            let _ = download
                .sender
                .send(DownloadStatus::Failed(Some(reason.to_string())));
            self.context.write_safe()?.update_download_with_status(
                token,
                DownloadStatus::Failed(Some(reason.to_string())),
            );
        }

        Ok((download, download_receiver))
    }

    /// Fail every still-`Queued` download for `username`, both on the caller's
    /// status channel (so a blocked `Receiver` unblocks) and in the store.
    pub(crate) fn fail_queued_downloads(
        client_context: &Arc<RwLock<ClientContext>>,
        username: &str,
    ) {
        let mut context = match client_context.write_safe() {
            Ok(c) => c,
            Err(e) => {
                error!("[client] fail_queued_downloads write: {}", e);
                return;
            }
        };
        let doomed: Vec<(u32, Sender<DownloadStatus>)> = context
            .get_downloads()
            .iter()
            .filter(|d| {
                d.username == username
                    && matches!(d.status, DownloadStatus::Queued)
            })
            .map(|d| (d.token, d.sender.clone()))
            .collect();
        for (token, sender) in doomed {
            let reason = Some("The user went offline".to_string());
            let _ = sender.send(DownloadStatus::Failed(reason.clone()));
            context.update_download_with_status(
                token,
                DownloadStatus::Failed(reason),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::next_download_token;
    use std::collections::HashSet;

    // The download store is keyed by token and removes *every* entry matching
    // one, so two downloads sharing a token means queueing one silently evicts
    // the other — which is what fetching the same filename from two peers does
    // if the token depends on the filename.
    #[test]
    fn download_tokens_are_unique_and_below_the_upload_range() {
        let tokens: Vec<u32> =
            (0..10_000).map(|_| next_download_token()).collect();

        assert_eq!(
            tokens.iter().collect::<HashSet<_>>().len(),
            tokens.len(),
            "every download must get its own token"
        );
        assert!(
            tokens.iter().all(|t| *t < 0x8000_0000),
            "download tokens must not reach into the upload range"
        );
    }
}
