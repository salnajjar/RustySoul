//! Serializes control operations away from Tokio's HTTP workers. The protocol library owns sockets.
use crate::store::{SavedDownload, Store};
use crate::{
    config::{Config, Secrets},
    model::*,
};
use anyhow::{Context, Result, ensure};
use soulseek_rs::{Client, ClientSettings, ClientVersion, DownloadStatus, PeerAddress, RoomEvent};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;
mod automation;
mod users;
mod wishlist;

pub struct EngineHandle {
    pub commands: mpsc::SyncSender<Command>,
    pub snapshot: watch::Receiver<Snapshot>,
    pub stop: mpsc::Sender<()>,
    pub thread: std::thread::JoinHandle<()>,
}

struct Engine {
    idle: crate::idle::Presence,
    privileges: crate::privileges::Controller,
    storage: std::sync::Arc<crate::storage::Guard>,
    storage_volumes: BTreeMap<(String, String), u64>,
    storage_retry: BTreeMap<(String, String), Instant>,
    system: crate::system::Monitor,
    room_controller: crate::rooms::Controller,
    discovery: crate::discovery::Controller,
    queue_rules: crate::queue_order::Rules,
    work_dirs: BTreeMap<(String, String), cap_std::fs::Dir>,
    #[cfg(target_os = "linux")]
    staging_cleanup: crate::downloads::StagingCleanup,
    message_ids: (u64, u64),
    profile: crate::profile::Profile,
    upload_speed: crate::upload_speed::Estimate,
    away_replied: BTreeMap<String, Instant>,
    traffic_pending: std::collections::HashMap<String, (u64, u64)>,
    traffic_sample_at: Instant,
    traffic_compacted_at: Instant,
    notice_deadline: Option<Instant>,
    library_notice: bool,
    library_error: Option<String>,
    network: NetworkSettings,
    network_worker: crate::network::Network,
    room_activity: BTreeMap<String, u64>,
    library: crate::library::Library,
    index: std::sync::Arc<soulseek_rs::shares::Shares>,
    locations: FileLocations,
    locations_generation: u64,
    vault: crate::vault::Vault,
    config: Config,
    secrets: Secrets,
    client: Option<Client>,
    view: Snapshot,
    updates: watch::Sender<Snapshot>,
    searches: BTreeMap<String, (String, Instant)>,
    browsing: BTreeMap<String, Instant>,
    lookups: BTreeMap<String, Instant>,
    watched: BTreeMap<String, Instant>,
    profiles: BTreeMap<String, Instant>,
    focused_users: BTreeMap<String, Instant>,
    wishlist_last_sent: u64,
    retries: BTreeMap<(String, String), automation::RetryState>,
    queue: Vec<SavedDownload>,
    store: Store,
    last_save: Instant,
    unread_through: u64,
    _instance_lock: std::fs::File,
    retry: Option<Instant>,
    retry_seconds: u64,
}

pub fn start(mut config: Config, mut secrets: Secrets) -> Result<EngineHandle> {
    let started_at = Instant::now();
    prepare_data_dir(&config.data_dir)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.data_dir.join("daemon.lock"))?;
    lock.try_lock()
        .context("Another daemon is already using this data directory")?;
    let store = Store::open(&config.data_dir.join("rustysoul.sqlite3"))?;
    crate::transfers::recover_downloads(&store.connection)?;
    let mut network = store.network()?.unwrap_or(NetworkSettings {
        port: config.soulseek.listen_port,
        connect_on_startup: config.soulseek.auto_connect,
        upload_slots: config.limits.upload_slots,
        max_peers: config.limits.peers,
        download_limit_kib: config
            .limits
            .download_bytes_per_second
            .div_ceil(1024)
            .min(1_000_000) as u32,
        ..Default::default()
    });
    if let Some(port) = config.published_transfer_port
        && (network.port != port || network.upnp)
    {
        network.port = port;
        network.upnp = false;
        store.save_network(&network)?;
    }
    network.validate().map_err(anyhow::Error::msg)?;
    let network_worker = crate::network::Network::start(network.clone(), store.identity()?)?;
    let downloads = config
        .download_dir
        .clone()
        .unwrap_or_else(|| config.data_dir.join("downloads"));
    std::fs::create_dir_all(&downloads)?;
    let locations = store.locations()?.unwrap_or(FileLocations {
        downloads: std::fs::canonicalize(downloads)?
            .to_string_lossy()
            .into_owned(),
        shares: config
            .soulseek
            .shares
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    });
    config.soulseek.shares = locations.shares.iter().map(Into::into).collect();
    let index = crate::library::cached_index(&store.connection, &config.soulseek.shares)?;
    let library_settings = store.library_settings()?;
    let library = crate::library::Library::start(
        config.data_dir.join("rustysoul.sqlite3"),
        config.soulseek.shares.clone(),
        library_settings.index_on_startup || config.data_dir.join("restore-reindex").is_file(),
    )?;
    if config.data_dir.join("restore-reindex").is_file() {
        std::fs::remove_file(config.data_dir.join("restore-reindex"))?;
    }
    let saved = store.encrypted_credentials()?;
    let vault =
        crate::vault::Vault::open(&config.data_dir.join("credentials.key"), saved.is_some())?;
    if let Some(saved) = saved {
        (secrets.username, secrets.password) = vault.decrypt(&saved)?;
    }
    let mut queue = store.downloads()?;
    let restored_history = config.data_dir.join("restore-media-safe").is_file();
    for row in queue
        .iter_mut()
        .filter(|r| !restored_history && r.status == "completed" && r.local_path.is_empty())
    {
        if let Ok(Some(path)) = crate::downloads::organize(row) {
            row.local_path = path.to_string_lossy().into_owned();
        }
    }
    let mut store = store;
    store.save_downloads(&queue)?;
    for row in queue
        .iter()
        .filter(|r| !restored_history && r.status == "completed" && !r.local_path.is_empty())
    {
        let _ = crate::downloads::remove_staging(row);
    }

    ensure!(
        queue
            .iter()
            .filter(|r| !automation::finished(&r.status))
            .count()
            <= config.limits.transfers,
        "Saved queue exceeds configured transfer limit"
    );
    for row in &queue {
        Command::Download {
            username: row.username.clone(),
            path: row.path.clone(),
            size: row.size,
        }
        .validate()
        .map_err(anyhow::Error::msg)?;
    }
    let profile = crate::profile::load(&store.connection)?;
    let upload_speed = crate::upload_speed::Estimate::load(&store.connection)?;
    let mut view = Snapshot {
        room_management: crate::rooms::load(&store.connection, &secrets.username)?,
        discovery: crate::discovery::load(&store.connection, &secrets.username, None)?,
        sharing_policy: crate::sharing::load(&store.connection)?,
        profile: profile.view(0),
        away: profile.away_on_startup,
        http_bind: config.bind.to_string(),
        port_binding: PortBinding {
            fixed_port: config.published_transfer_port,
        },
        network: network.clone(),
        transfer_capacity: config.limits.transfers,
        location_roots: config
            .location_roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        locations: locations.clone(),
        version: env!("CARGO_PKG_VERSION"),
        connection: "disconnected".into(),
        username: secrets.username.clone(),
        credentials_configured: !secrets.username.is_empty(),
        downloads: crate::transfers::preview(&store.connection, "downloads")?,
        downloads_total: queue.len(),
        buddies: store.buddies()?,
        library_settings,
        wishlist: wishlist::load(&store.connection)?,
        user_rules: users::rules(&store.connection)?,
        retry_policy: automation::load_policy(&store.connection)?,
        shares: ShareCounts {
            folders: index.folder_count(),
            files: index.file_count(),
        },
        ..Default::default()
    };
    store.restore(&mut view)?;
    // The initial watch snapshot is available before the engine's first poll.
    // Publish durable measurements immediately, including after a restart.
    view.profile.measured_upload_speed = upload_speed.measured();
    view.profile.upload_speed_measured = upload_speed.has_measurement();
    view.profile.upload_speed_recipients = upload_speed.recipients();
    view.profile.upload_speed_selected_recipients = upload_speed.selected();
    view.subscribed_rooms = store
        .connection
        .prepare("SELECT room FROM room_subscriptions ORDER BY rowid")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    view.subscribed_rooms
        .extend(crate::rooms::private_subscriptions(
            &store.connection,
            &secrets.username,
        )?);
    view.subscribed_rooms.sort();
    view.subscribed_rooms.dedup();
    view.room_read = crate::messages::read_cursors(&store.connection, false)?;
    view.private_read = crate::messages::read_cursors(&store.connection, true)?;
    view.room_unread = crate::messages::unread(&store.connection, &secrets.username, false, None)?;
    view.private_unread =
        crate::messages::unread(&store.connection, &secrets.username, true, None)?;
    let unread_through = view.messages.last().map_or(0, |m| m.id);

    crate::transfers::recover_uploads(&store.connection)?;
    view.uploads = crate::transfers::preview(&store.connection, "uploads")?;
    view.traffic = crate::traffic::totals(&store.connection)?;
    crate::traffic::compact(&store.connection, now())?;
    let room_activity = view
        .rooms
        .iter()
        .filter_map(|r| r.last_activity.map(|t| (r.name.clone(), t)))
        .collect();
    let wishlist_last_sent =
        store
            .connection
            .query_row("SELECT last_sent FROM wishlist_clock WHERE id=1", [], |r| {
                r.get::<_, i64>(0)
            })? as u64;
    let retries = automation::load_retries(&store.connection)?;
    let queue_rules = crate::queue_order::load(&store.connection)?;
    let message_ids = store.reserve_message_ids()?;
    // A reserved ID block also identifies the history cache generation. It is
    // different after every restart, even if a client missed a deletion event.
    view.message_reset = message_ids.0;
    let (updates, snapshot) = watch::channel(view.clone());
    let storage = std::sync::Arc::new(crate::storage::Guard::new(network.minimum_free_space_mib));
    let system = crate::system::Monitor::start_with_storage(
        config.data_dir.clone(),
        locations.downloads.clone().into(),
        started_at,
        Some(storage.clone()),
    )?;
    let (commands, receiver) = mpsc::sync_channel(32);
    let (stop, stop_receiver) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("soulseek-control".into())
        .spawn(move || {
            let auto_connect = network.connect_on_startup;
            let mut engine = Engine {
                idle: Default::default(),
                privileges: crate::privileges::Controller::new(&config.soulseek.server),
                storage,
                storage_volumes: BTreeMap::new(),
                storage_retry: BTreeMap::new(),
                system,
                room_controller: Default::default(),
                discovery: Default::default(),
                queue_rules,
                work_dirs: BTreeMap::new(),
                #[cfg(target_os = "linux")]
                staging_cleanup: Default::default(),
                message_ids,
                profile,
                upload_speed,
                away_replied: BTreeMap::new(),
                traffic_pending: std::collections::HashMap::new(),
                traffic_sample_at: Instant::now(),
                traffic_compacted_at: Instant::now(),
                notice_deadline: None,
                library_notice: false,
                library_error: None,
                network,
                network_worker,
                room_activity,
                library,
                index,
                locations,
                locations_generation: 0,
                vault,
                config,
                secrets,
                client: None,
                view,
                updates,
                queue,
                store,
                last_save: Instant::now(),
                unread_through,
                _instance_lock: lock,
                searches: BTreeMap::new(),
                browsing: BTreeMap::new(),
                lookups: BTreeMap::new(),
                watched: BTreeMap::new(),
                profiles: BTreeMap::new(),
                focused_users: BTreeMap::new(),
                wishlist_last_sent,
                retries,
                retry: None,
                retry_seconds: 5,
            };
            if auto_connect && !engine.secrets.username.is_empty() {
                engine.retry = Some(Instant::now());
                engine.execute(Command::Connect);
            }
            loop {
                if stop_receiver.try_recv().is_ok() {
                    break;
                }
                match receiver.recv_timeout(Duration::from_millis(500)) {
                    Ok(command) => engine.execute(command),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if engine.retry.is_some_and(|at| at <= Instant::now()) {
                    engine.execute(Command::Connect);
                }
                engine.poll_library();
                engine.poll_network();
                #[cfg(target_os = "linux")]
                match engine.staging_cleanup.poll(
                    Path::new(&engine.locations.downloads),
                    &engine.queue,
                    engine.work_dirs.values(),
                ) {
                    Ok(Some(removed)) if removed > 0 => crate::activity::record(
                        "info",
                        "maintenance",
                        &format!("Removed {removed} empty download staging folders"),
                    ),
                    Err(error) => crate::activity::record(
                        "warn",
                        "maintenance",
                        &format!("Could not finish download staging cleanup: {error:#}"),
                    ),
                    _ => (),
                }
                if engine
                    .notice_deadline
                    .is_some_and(|at| Instant::now() >= at)
                {
                    engine.view.notice = None;
                    engine.notice_deadline = None;
                    engine.publish();
                }
                engine.refresh();
                engine.poll_wishlist();
            }
            if let Some(client) = engine.client.as_mut() {
                client.disconnect();
            }
            engine.sample_traffic(true);
            engine.client = None;
            engine.library.shutdown();
            engine.network_worker.shutdown();
            for search in &mut engine.view.searches {
                search.collecting = false;
            }
            if let Err(error) = engine
                .store
                .save_snapshot(&engine.view)
                .and_then(|_| engine.store.checkpoint())
            {
                tracing::error!(%error, "Could not flush database during shutdown");
            }
        })?;
    Ok(EngineHandle {
        commands,
        snapshot,
        stop,
        thread,
    })
}

fn prepare_data_dir(path: &Path) -> Result<()> {
    if path.exists() {
        ensure!(
            !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
            "Data directory must not be a symlink"
        );
    }
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = std::fs::metadata(path)?;
        // A dedicated directory prevents other local users planting symlinks in download targets.
        ensure!(
            meta.mode() & 0o077 == 0 || std::fs::read_dir(path)?.next().is_none(),
            "Existing data directory must be private (chmod 700)"
        );
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn saved_view(row: &SavedDownload) -> TransferView {
    TransferView {
        username: row.username.clone(),
        path: row.path.clone(),
        size: row.size,
        transferred: if row.status == "completed" {
            row.size
        } else {
            0
        },
        speed: 0.,
        status: if matches!(row.status.as_str(), "queued" | "downloading") {
            "interrupted".into()
        } else {
            row.status.clone()
        },
        queue_position: None,
        ..Default::default()
    }
}

impl Engine {
    fn commit_profile(&mut self, profile: crate::profile::Profile) -> Result<()> {
        crate::profile::save(&self.store.connection, &profile)?;
        if let Some(c) = &self.client {
            c.set_public_profile(profile.description.clone(), profile.picture_bytes());
            for item in self
                .profile
                .likes
                .iter()
                .filter(|i| !profile.likes.contains(i))
            {
                let _ = c.remove_interest(item);
            }
            for item in self
                .profile
                .dislikes
                .iter()
                .filter(|i| !profile.dislikes.contains(i))
            {
                let _ = c.remove_dislike(item);
            }
            for item in profile
                .likes
                .iter()
                .filter(|i| !self.profile.likes.contains(i))
            {
                let _ = c.add_interest(item);
            }
            for item in profile
                .dislikes
                .iter()
                .filter(|i| !self.profile.dislikes.contains(i))
            {
                let _ = c.add_dislike(item);
            }
        }
        self.view.profile = profile.view(self.view.profile.revision + 1);
        self.profile = profile;
        if let Some(c) = &self.client {
            self.configure_speed_reporting(c);
        }
        self.view.notice = Some("Profile saved. It is available to peers while connected.".into());
        Ok(())
    }

    fn sample_traffic(&mut self, force: bool) {
        if !force && self.traffic_sample_at.elapsed() < Duration::from_secs(5) {
            return;
        }
        if let Some(c) = &self.client {
            for (u, (d, b)) in c.take_payload_users() {
                let p = self.traffic_pending.entry(u).or_default();
                p.0 += d;
                p.1 += b;
            }
        }
        let downloaded = self.traffic_pending.values().map(|p| p.0).sum();
        let uploaded = self.traffic_pending.values().map(|p| p.1).sum();
        match crate::traffic::record_users(
            &self.store.connection,
            now(),
            downloaded,
            uploaded,
            &self.traffic_pending,
        ) {
            Ok(()) => {
                let samples = self
                    .client
                    .as_ref()
                    .map(Client::take_upload_rate_samples)
                    .unwrap_or_default();
                if let Err(e) = self.upload_speed.observe(&self.store.connection, samples) {
                    self.view.error = Some(e.to_string());
                }
                if let Some(c) = &self.client {
                    self.configure_speed_reporting(c);
                }
                if downloaded > 0 || uploaded > 0 {
                    self.view.transfer_revision += 1;
                }
                self.traffic_pending.clear();
                self.traffic_sample_at = Instant::now();
                self.view.traffic = crate::traffic::totals(&self.store.connection)
                    .unwrap_or_else(|_| self.view.traffic.clone());
            }
            Err(e) => self.view.error = Some(e.to_string()),
        }
        if self.traffic_compacted_at.elapsed() >= Duration::from_secs(3600) {
            match crate::traffic::compact(&self.store.connection, now()) {
                Ok(()) => self.traffic_compacted_at = Instant::now(),
                Err(e) => self.view.error = Some(e.to_string()),
            }
        }
    }
    fn publish(&self) {
        crate::activity::transition("error", "system", self.view.error.as_deref());
        crate::activity::transition("info", "system", self.view.notice.as_deref());
        let mut snapshot = self.view.clone();
        snapshot.privileges = self.privileges.view.clone();
        snapshot.profile.measured_upload_speed = self.upload_speed.measured();
        snapshot.profile.upload_speed_measured = self.upload_speed.has_measurement();
        snapshot.profile.upload_speed_recipients = self.upload_speed.recipients();
        snapshot.profile.upload_speed_selected_recipients = self.upload_speed.selected();
        snapshot.away_automatic = self.idle.automatic;
        snapshot.system = self.system.latest();
        snapshot.effective_peer_limit = self.client.as_ref().map_or(0, |c| c.peer_capacity());
        snapshot.messages.retain(|m| {
            !snapshot
                .user_rules
                .iter()
                .any(|r| r.ignored && r.username.eq_ignore_ascii_case(&m.username))
        });
        snapshot.download_capacity_used = self
            .queue
            .iter()
            .filter(|r| !automation::finished(&r.status))
            .count();
        snapshot.downloads_total = self.queue.len();
        snapshot.uploads_total = self
            .store
            .connection
            .query_row("SELECT COUNT(*) FROM uploads", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize;
        snapshot.downloads.truncate(100);
        snapshot.uploads.truncate(100);
        self.updates.send_replace(snapshot);
    }
    fn execute(&mut self, command: Command) {
        if matches!(command, Command::UiActivity { .. }) {
            let _ = self.apply(command);
            self.publish();
            return;
        }
        let operation = match &command {
            Command::UpdateStorageReserve { .. } => "Update storage protection",
            Command::UpdateCredentials { .. } => "Verify and save Soulseek account",
            Command::CheckPrivileges { .. } => "Check Soulseek privileges",
            Command::ClearCredentials => "Remove Soulseek account",
            Command::UpdateProfile { .. }
            | Command::UpdateProfilePrepared { .. }
            | Command::SetInterest { .. } => "Update profile",
            Command::UpdateLocations { .. } => "Update file locations",
            Command::RescanShares => "Re-index library",
            Command::Connect => "Connect to Soulseek",
            Command::Disconnect => "Disconnect from Soulseek",
            Command::Search { .. } | Command::SearchScoped { .. } => "Start search",
            Command::RestartSearch { .. } => "Repeat search",
            Command::Discover { .. } => "Update discovery",
            Command::RetryDownloads { .. } => "Retry failed downloads",
            Command::CancelUploads { .. } | Command::CancelUpload { .. } => "Cancel uploads",
            Command::CancelDownloads { .. } => "Cancel downloads",
            Command::UpdateTransferQueue { .. } => "Update transfer priority and queue",
            Command::Download { .. }
            | Command::DownloadBatch { .. }
            | Command::DownloadBatchConfirmed { .. }
            | Command::DownloadFolder { .. } => "Queue downloads",
            Command::Transfer { .. } => "Update download",
            Command::ClearTransfers { .. } => "Clear transfer history",
            Command::Browse { .. } => "Browse shares",
            Command::CloseBrowse { .. } => "Close shared library",
            Command::JoinRoom { .. } => "Join room",
            Command::LeaveRoom { .. } => "Leave room",
            Command::CreatePrivateRoom { .. } => "Create private room",
            Command::PrivateRoomAction { .. } => "Update private-room access",
            Command::SetRoomTicker { .. } => "Update room-board message",
            Command::SetRoomInvitations { .. } => "Update room invitation preference",
            Command::SendMessage { .. } => "Send message",
            Command::ClearMessageHistory { .. } => "Delete message history",
            Command::WatchUsers { .. } | Command::ReadRoom { .. } | Command::ReadPrivate { .. } => {
                ""
            }
            _ => "Update settings or user controls",
        };
        crate::activity::record("info", "activity", operation);
        if matches!(
            command,
            Command::WatchUsers { .. } | Command::ReadRoom { .. } | Command::ReadPrivate { .. }
        ) {
            if let Err(e) = self.apply(command) {
                self.view.error = Some(format!("{e:#}"));
            }
            self.publish();
            return;
        }
        let library_notice = matches!(
            command,
            Command::RescanShares | Command::UpdateLocations { .. }
        );
        self.notice_deadline = None;
        self.library_notice = false;
        self.view.error = None;
        self.view.notice = None;
        let joining_room = match &command {
            Command::JoinRoom { room } | Command::CreatePrivateRoom { room } => Some(room.clone()),
            _ => None,
        };
        if let Err(error) = self.apply(command) {
            self.view.notice = None;
            // Protocol errors may contain peer text; passwords/tokens are never interpolated here.
            self.view.error = Some(
                format!("{operation} failed: {error:#}")
                    .chars()
                    .take(2048)
                    .collect(),
            );
            if let Some(room) = joining_room
                && !self.room_controller.pending(&room)
                && (self.view.room_management.rooms.contains_key(&room)
                    || self.view.room_management.rooms.len() < 256)
            {
                let summary = self.view.room_management.rooms.entry(room).or_default();
                summary.revision = summary.revision.wrapping_add(1);
                summary.error.clone_from(&self.view.error);
            }
            if self.view.connection == "verifying" || self.view.connection == "connecting" {
                self.view.connection = "disconnected".into();
            }
        }
        if self.view.notice.is_some() {
            self.library_notice = library_notice;
            if !library_notice {
                self.notice_deadline = Some(Instant::now() + Duration::from_secs(6));
            }
        }
        if let Err(error) = self.flush_messages() {
            self.view.error = Some(format!("Save messages: {error:#}"));
        }
        self.publish();
    }
    fn poll_library(&mut self) {
        if self.library.updates.has_changed().unwrap_or(false) {
            let update = self.library.updates.borrow_and_update().clone();
            if let Some(update) = update.filter(|u| u.generation == self.locations_generation) {
                self.view.library = update.status.clone();
                if let Some(error) = update.error {
                    self.library_error = Some(error.clone());
                    self.view.error = Some(error);
                }
                if !update.status.scanning {
                    if self.library_notice {
                        self.view.notice = None;
                        self.library_notice = false;
                    }
                    if update.index.is_some() && self.library_error.is_some() {
                        if self.view.error == self.library_error {
                            self.view.error = None;
                        }
                        self.library_error = None;
                    }
                }
                if let Some(index) = update.index {
                    self.view.shares = ShareCounts {
                        folders: index.folder_count(),
                        files: index.file_count(),
                    };
                    self.index = index;
                    if let Some(c) = &self.client
                        && let Err(e) =
                            c.replace_share_index(self.locations.shares.clone(), self.index.clone())
                    {
                        self.view.error = Some(format!("{e:#}"));
                    }
                }
                self.publish();
            }
        }
    }
    fn poll_network(&mut self) {
        if self.network_worker.status.has_changed().unwrap_or(false) {
            let mut status = self.network_worker.status.borrow_and_update().clone();
            status.listener_ready = self.client.is_some()
                && std::net::TcpStream::connect_timeout(
                    &std::net::SocketAddr::from(([127, 0, 0, 1], self.network.port)),
                    Duration::from_millis(200),
                )
                .is_ok();
            self.view.connectivity = status;
            self.publish();
        }
    }
    fn authenticate(&self, username: &str, password: &str) -> Result<Client> {
        let mut settings = ClientSettings::new(username, password);
        settings.server_address = PeerAddress::new(
            self.config.soulseek.server.clone(),
            self.config.soulseek.port,
        );
        settings.enable_listen = true;
        settings.listen_port = self.network.port;
        settings.accept_children = false;
        settings.version = ClientVersion {
            major: 177,
            minor: 1,
        };
        // SQLite's cached index is installed after login; never run the library's full scanner.
        settings.shared_directories = vec![];
        let mut client = Client::with_settings(settings);
        self.configure_transfers(&client);
        client
            .connect()
            .context("Could not connect to the Soulseek server")?;
        ensure!(
            client
                .login()
                .context("Soulseek login failed or timed out")?,
            "Soulseek rejected the login. Check your username and password"
        );
        client.set_public_profile(
            self.profile.description.clone(),
            self.profile.picture_bytes(),
        );
        for item in &self.profile.likes {
            let _ = client.add_interest(item);
        }
        for item in &self.profile.dislikes {
            let _ = client.add_dislike(item);
        }
        client.set_away(self.view.away)?;
        self.configure_speed_reporting(&client);
        client.replace_share_index(self.locations.shares.clone(), self.index.clone())?;
        let _ = client.request_room_list();
        client.set_room_invitations_enabled(crate::rooms::invitations(
            &self.store.connection,
            username,
        )?)?;
        // Private rooms rejoin only after the server confirms current access.
        for room in self
            .store
            .connection
            .prepare("SELECT room FROM room_subscriptions")?
            .query_map([], |r| r.get::<_, String>(0))?
        {
            let _ = client.join_room(&room?);
        }
        Ok(client)
    }
    fn upload_queue_rules(&self) -> Vec<(String, String, i8, u64)> {
        self.queue_rules
            .iter()
            .filter(|((kind, _, _), _)| kind == "uploads")
            .map(|((_, u, p), r)| (u.clone(), p.clone(), r.priority, r.position))
            .collect()
    }
    fn configure_speed_reporting(&self, client: &Client) {
        client.configure_upload_speed_reporting(
            crate::upload_speed::Estimate::override_speed(
                &self.profile,
                self.network.upload_limit_kib,
            ),
            self.upload_speed.measured(),
        );
    }
    fn configure_transfers(&self, client: &Client) {
        self.configure_speed_reporting(client);
        client.set_download_write_guard(Some(self.storage.clone()));
        client.set_download_slots(
            self.network.download_slots,
            self.network.effective_download_slots_per_user(),
        );
        client.set_upload_policy(self.view.sharing_policy.protocol(&self.view.buddies));
        client.set_max_peers(self.network.max_peers);
        client.set_upload_queue_order(self.upload_queue_rules());
        client.set_upload_limits(
            self.network.upload_slots,
            self.network.upload_slots_per_user,
        );
        let kib = |n: u32| u64::from(n) * 1024;
        client.set_banned_users(
            self.view
                .user_rules
                .iter()
                .filter(|r| r.banned)
                .map(|r| r.username.clone())
                .collect(),
        );
        client.set_download_speed_limits(
            kib(self.network.download_limit_kib),
            kib(self.network.download_per_user_limit_kib),
            self.network
                .user_limits
                .iter()
                .map(|u| (u.username.clone(), kib(u.download_limit_kib)))
                .collect(),
        );
        client.set_upload_speed_limits(
            kib(self.network.upload_limit_kib),
            kib(self.network.upload_per_user_limit_kib),
            self.network
                .user_limits
                .iter()
                .map(|u| (u.username.clone(), kib(u.upload_limit_kib)))
                .collect(),
        );
    }
    fn refresh_unread(&mut self, target: &str, private: bool) -> Result<()> {
        let counts = crate::messages::unread(
            &self.store.connection,
            &self.secrets.username,
            private,
            Some(target),
        )?;
        let map = if private {
            &mut self.view.private_unread
        } else {
            &mut self.view.room_unread
        };
        let key = if private {
            target.to_ascii_lowercase()
        } else {
            target.into()
        };
        map.remove(&key);
        map.extend(counts);
        Ok(())
    }
    fn mark_read(&mut self, target: &str, through_id: u64, private: bool) -> Result<()> {
        let collation = if private { "COLLATE NOCASE" } else { "" };
        let latest = self.store.connection.query_row(&format!("SELECT COALESCE(MAX(id),0) FROM messages WHERE private=?1 AND target=?2 {collation}"),rusqlite::params![private,target],|r|r.get::<_,i64>(0))? as u64;
        if latest == 0 {
            return Ok(());
        }
        ensure!(
            through_id <= latest,
            "Cannot acknowledge messages that have not arrived"
        );
        let (table, column) = if private {
            ("private_reads", "username")
        } else {
            ("room_reads", "room")
        };
        self.store.connection.execute(&format!("INSERT INTO {table} VALUES(?1,?2) ON CONFLICT({column}) DO UPDATE SET last_id=MAX(last_id,excluded.last_id)"),rusqlite::params![target,through_id as i64])?;
        let map = if private {
            &mut self.view.private_read
        } else {
            &mut self.view.room_read
        };
        let key = if private {
            target.to_ascii_lowercase()
        } else {
            target.into()
        };
        map.entry(key)
            .and_modify(|id| *id = (*id).max(through_id))
            .or_insert(through_id);
        self.refresh_unread(target, private)
    }
    fn flush_messages(&mut self) -> Result<()> {
        if self.store.save_messages(&self.view.messages)? {
            self.view.message_revision += 1;
            let targets: std::collections::BTreeSet<_> = self
                .view
                .messages
                .iter()
                .filter(|m| m.id > self.unread_through)
                .map(|m| (m.private, m.target.clone()))
                .collect();
            for (private, target) in targets {
                self.refresh_unread(&target, private)?;
            }
            self.unread_through = self
                .view
                .messages
                .iter()
                .map(|m| m.id)
                .max()
                .unwrap_or(self.unread_through);
        }
        let overflow = self.view.messages.len().saturating_sub(500);
        self.view.messages.drain(..overflow);
        Ok(())
    }
    fn persist(&mut self) -> Result<()> {
        self.store
            .save_downloads(&self.queue)
            .context("Cannot persist transfer records")?;
        self.view.transfer_revision += 1;
        self.view.downloads = crate::transfers::preview(&self.store.connection, "downloads")?;
        Ok(())
    }
    fn connect(&mut self) -> Result<()> {
        let retrying = self.retry.is_some();
        self.retry = None;
        ensure!(self.client.is_none(), "Already connected");
        ensure!(
            !self.secrets.username.is_empty(),
            "Save your Soulseek account in Settings first"
        );
        self.view.connection = "connecting".into();
        self.publish();
        match self.authenticate(&self.secrets.username, &self.secrets.password) {
            Ok(client) => {
                self.clear_presence();

                self.client = Some(client);
                self.room_controller
                    .connected(&mut self.view.room_management);
                self.network_worker.update(self.network.clone(), true);
                self.view.connection = "connected".into();
                self.retry_seconds = 5;
                Ok(())
            }
            Err(error) => {
                self.view.connection = "disconnected".into();
                if retrying {
                    self.retry = Some(Instant::now() + Duration::from_secs(self.retry_seconds));
                    self.retry_seconds = (self.retry_seconds * 2).min(300);
                }
                Err(error)
            }
        }
    }
    fn apply(&mut self, command: Command) -> Result<()> {
        command.validate().map_err(anyhow::Error::msg)?;
        if let Command::Transfer {
            username,
            path,
            action,
        } = &command
            && matches!(action, TransferAction::Pause | TransferAction::Cancel)
            && let Some(index) = self.queue.iter().position(|r| {
                &r.username == username && &r.path == path && r.status == "waiting_for_space"
            })
        {
            self.queue[index].status = if matches!(action, TransferAction::Pause) {
                "paused_waiting"
            } else {
                "cancelled"
            }
            .into();
            self.storage_retry.remove(&(username.clone(), path.clone()));
            self.persist()?;
            return Ok(());
        }
        match command {
            Command::UiActivity { idle_seconds } => {
                self.idle.activity(Instant::now(), idle_seconds);
                self.poll_idle();
                return Ok(());
            }
            Command::CheckPrivileges {
                watch_for_activation,
            } => {
                let client = self
                    .client
                    .as_ref()
                    .context("Connect to Soulseek to check privileges")?;
                if self
                    .privileges
                    .request(Instant::now(), watch_for_activation)
                    && let Err(error) = client.check_privileges()
                {
                    self.privileges.failed();
                    return Err(error.into());
                }
                return Ok(());
            }
            Command::UpdateStorageReserve {
                minimum_free_space_mib,
            } => {
                let mut settings = self.network.clone();
                settings.minimum_free_space_mib = minimum_free_space_mib;
                settings.validate().map_err(anyhow::Error::msg)?;
                self.store.save_network(&settings)?;
                self.network = settings;
                self.storage.set_minimum(minimum_free_space_mib);
                self.storage_retry.clear();
                self.system.request_refresh();
                self.view.network = self.network.clone();
                self.view.notice = Some("Download storage protection saved".into());
                return Ok(());
            }

            Command::DownloadBatchConfirmed { files, reply } => {
                let count = files.len();
                let result = self.queue_files(files);
                let _ = reply.send(
                    result
                        .as_ref()
                        .map(|()| count)
                        .map_err(|e| format!("{e:#}")),
                );
                result?;
                self.view.notice = Some(format!("Added {count} files to the download queue"));
                return Ok(());
            }
            Command::RestartSearch { id, query } => {
                let index = self
                    .view
                    .searches
                    .iter()
                    .position(|s| s.id == id)
                    .context("Search no longer exists")?;
                let old = self.view.searches[index].clone();
                let c = self.client.as_ref().context("Connect to Soulseek first")?;
                let _ = c.forget_search(&id);
                match &old.scope {
                    SearchScope::Global => c.search_keyed(&id, &query, None, None, false),
                    SearchScope::User { username } => {
                        c.search_keyed(&id, &query, Some(username), None, false)
                    }
                    SearchScope::Room { room } => {
                        c.search_keyed(&id, &query, None, Some(room), false)
                    }
                }?;
                let view = SearchView {
                    id: id.clone(),
                    query: query.clone(),
                    scope: old.scope,
                    collecting: true,
                    revision: old.revision + 1,
                    wishlist_id: old.wishlist_id,
                    pinned: old.pinned,
                    ..Default::default()
                };
                let tx = crate::store::write_transaction(&self.store.connection)?;
                tx.execute("DELETE FROM search_results WHERE search_id=?1", [&id])?;
                tx.execute("DELETE FROM search_user_stats WHERE search_id=?1", [&id])?;
                crate::search::create(&tx, &view)?;
                tx.commit()?;
                self.view.searches[index] = view;
                self.searches.insert(id, (query, Instant::now()));
                return Ok(());
            }
            Command::RetryDownloads { username } => {
                let c = self
                    .client
                    .as_ref()
                    .context("Connect to Soulseek before retrying downloads")?;
                let retryable = |r: &&SavedDownload| {
                    matches!(r.status.as_str(), "failed" | "timed_out" | "interrupted")
                        && username.as_ref().is_none_or(|u| u == &r.username)
                };
                let count = self.queue.iter().filter(retryable).count();
                let active = self
                    .queue
                    .iter()
                    .filter(|r| !automation::finished(&r.status))
                    .count();
                ensure!(
                    count + active <= self.config.limits.transfers,
                    "Not enough download queue space; retry individual files or increase the transfer queue limit"
                );
                let keys = self
                    .queue
                    .iter()
                    .filter(retryable)
                    .map(|r| (r.username.clone(), r.path.clone()))
                    .collect::<Vec<_>>();
                for (user, path) in &keys {
                    let _ = c.remove_download(user, path);
                    self.retries.remove(&(user.clone(), path.clone()));
                    self.store.connection.execute(
                        "DELETE FROM download_retries WHERE username=?1 AND path=?2",
                        rusqlite::params![user, path],
                    )?;
                }
                for r in &mut self.queue {
                    if keys.iter().any(|(u, p)| u == &r.username && p == &r.path) {
                        r.status = "waiting".into();
                    }
                }
                self.persist()?;
                self.pump_downloads()?;
                self.view.notice = Some(format!(
                    "Queued {count} failed or interrupted downloads for retry"
                ));
                return Ok(());
            }
            Command::UpdateTransferQueue {
                direction,
                username,
                path,
                priority,
                movement,
            } => {
                let kind = match direction {
                    HistoryDirection::Downloads => "downloads",
                    HistoryDirection::Uploads => "uploads",
                };
                self.queue_rules = crate::queue_order::update(
                    &self.store.connection,
                    kind,
                    &username,
                    &path,
                    priority,
                    movement,
                )?;
                if let Some(client) = &self.client {
                    client.set_upload_queue_order(self.upload_queue_rules());
                }
                self.view.transfer_revision += 1;
                self.view.notice = Some(
                    "Transfer queue preferences saved. Active transfers continue uninterrupted."
                        .into(),
                );
                self.pump_downloads()?;
                return Ok(());
            }
            Command::CancelDownloads { username, paths } => {
                let selected: std::collections::HashSet<_> =
                    paths.iter().map(String::as_str).collect();
                let targets: Vec<_> = self
                    .queue
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        r.username.eq_ignore_ascii_case(&username)
                            && (paths.is_empty() || selected.contains(r.path.as_str()))
                            && matches!(
                                r.status.as_str(),
                                "waiting"
                                    | "waiting_for_space"
                                    | "queued"
                                    | "downloading"
                                    | "paused"
                                    | "paused_waiting"
                                    | "retry_waiting"
                                    | "interrupted"
                            )
                    })
                    .map(|(i, r)| (i, r.clone()))
                    .collect();
                let tx = crate::store::write_transaction(&self.store.connection)?;
                for (_, row) in &targets {
                    tx.execute("UPDATE downloads SET status='cancelled',speed=0,updated_at=strftime('%s','now') WHERE username=?1 AND path=?2",rusqlite::params![row.username,row.path])?;
                    tx.execute(
                        "DELETE FROM download_retries WHERE username=?1 AND path=?2",
                        rusqlite::params![row.username, row.path],
                    )?;
                }
                tx.commit()?;
                for (i, row) in &targets {
                    self.queue[*i].status = "cancelled".into();
                    self.retries
                        .remove(&(row.username.clone(), row.path.clone()));
                    self.storage_retry
                        .remove(&(row.username.clone(), row.path.clone()));
                    if let Some(c) = &self.client {
                        let _ = c.cancel_download(&row.username, &row.path);
                    }
                }
                self.persist()?;
                self.view.transfer_revision += 1;
                self.view.downloads =
                    crate::transfers::preview(&self.store.connection, "downloads")?;
                self.view.notice = Some(format!(
                    "Cancelled {} downloads for {username}",
                    targets.len()
                ));
                self.pump_downloads()?;
                return Ok(());
            }
            Command::CancelUploads { username, paths } => {
                let c = self.client.as_ref().context("Connect to Soulseek first")?;
                let cancelled = c.cancel_uploads(&username, &paths);
                let count = cancelled.len();
                for u in cancelled {
                    crate::transfers::record_upload(
                        &self.store.connection,
                        &TransferView {
                            username: u.username,
                            path: u.filename,
                            size: u.size,
                            transferred: u.bytes_sent,
                            status: "cancelled".into(),
                            detail: "Cancelled by administrator".into(),
                            ..Default::default()
                        },
                    )?;
                }
                self.view.transfer_revision += 1;
                self.view.notice = Some(format!("Cancelled {count} uploads for {username}"));
                return Ok(());
            }
            Command::ReadRoom { room, through_id } => {
                return self.mark_read(&room, through_id, false);
            }
            Command::ReadPrivate {
                username,
                through_id,
            } => return self.mark_read(&username, through_id, true),
            Command::SetRoomInvitations { enabled } => {
                self.room_controller.set_invitations(
                    &self.store.connection,
                    &self.secrets.username,
                    self.client.as_ref(),
                    &mut self.view.room_management,
                    enabled,
                )?;
                return Ok(());
            }
            Command::SetRoomTicker { room, text } => {
                ensure!(
                    self.view.subscribed_rooms.contains(&room)
                        || self.view.joined_rooms.contains(&room),
                    "Join this room before setting its board message"
                );
                self.room_controller.set_ticker(
                    crate::rooms::Session {
                        db: &self.store.connection,
                        account: &self.secrets.username,
                        client: self.client.as_ref(),
                        joined: self.view.joined_rooms.contains(&room),
                    },
                    &mut self.view.room_management,
                    &room,
                    text,
                )?;
                return Ok(());
            }
            Command::PrivateRoomAction {
                room,
                action,
                username,
            } => {
                self.room_controller.action(
                    crate::rooms::Session {
                        db: &self.store.connection,
                        account: &self.secrets.username,
                        client: self.client.as_ref(),
                        joined: self.view.joined_rooms.contains(&room),
                    },
                    &mut self.view.room_management,
                    &room,
                    action,
                    username,
                )?;
                return Ok(());
            }
            Command::CreatePrivateRoom { room } => {
                if self.view.joined_rooms.contains(&room) {
                    ensure!(
                        self.view
                            .room_management
                            .rooms
                            .get(&room)
                            .is_some_and(|s| s.private),
                        "This is an existing public room. Choose another private-room name"
                    );
                    return Ok(());
                }
                ensure!(
                    self.view.subscribed_rooms.len() < 32
                        || self.view.subscribed_rooms.contains(&room),
                    "Leave a room before joining another (maximum 32)"
                );
                ensure!(
                    !self.view.rooms.iter().any(|r| r.name == room
                        && !self
                            .view
                            .room_management
                            .rooms
                            .get(&room)
                            .is_some_and(|s| s.private)),
                    "This is an existing public room. Choose another private-room name"
                );
                self.room_controller.create(
                    &self.store.connection,
                    &self.secrets.username,
                    self.client.as_ref().context("Connect to Soulseek first")?,
                    &mut self.view.room_management,
                    &room,
                )?;
                if !self.view.subscribed_rooms.contains(&room) {
                    self.view.subscribed_rooms.push(room);
                }
                return Ok(());
            }
            Command::JoinRoom { room } => {
                ensure!(
                    self.view.subscribed_rooms.len() < 32
                        || self.view.subscribed_rooms.contains(&room),
                    "Leave a room before joining another (maximum 32)"
                );
                let private = self
                    .view
                    .room_management
                    .rooms
                    .get(&room)
                    .is_some_and(|s| s.private);
                if private {
                    ensure!(
                        !self.room_controller.pending(&room),
                        "Wait for the current room request to finish"
                    );
                    ensure!(
                        self.view.room_management.rooms[&room].confirmed
                            && self.view.room_management.rooms[&room].member,
                        "Wait for the server to confirm membership before joining this private room"
                    );
                }
                if self.view.joined_rooms.contains(&room) {
                    return Ok(());
                }
                let client = self.client.as_ref().context("Connect to Soulseek first")?;
                if private {
                    client.join_private_room(&room)?;
                } else {
                    client.join_room(&room)?;
                }
                if private {
                    self.room_controller
                        .joining_private(&mut self.view.room_management, &room);
                }
                crate::rooms::subscribe(
                    &self.store.connection,
                    &self.secrets.username,
                    &room,
                    private,
                )?;
                if !self.view.subscribed_rooms.contains(&room) {
                    self.view.subscribed_rooms.push(room);
                }
                return Ok(());
            }
            Command::LeaveRoom { room } => {
                crate::rooms::unsubscribe(&self.store.connection, &self.secrets.username, &room)?;
                self.view.subscribed_rooms.retain(|r| r != &room);
                if let Some(c) = &self.client {
                    c.leave_room(&room)?;
                }
                self.view.joined_rooms.retain(|r| r != &room);
                return Ok(());
            }

            Command::UpdateSharingPolicy { settings } => {
                crate::sharing::save(&self.store.connection, &settings)?;
                self.view.sharing_policy = settings;
                if let Some(c) = &self.client {
                    c.set_upload_policy(self.view.sharing_policy.protocol(&self.view.buddies));
                }
                self.view.notice=Some("Sharing protection saved. Waiting requests will be checked again; active uploads can finish.".into());
                return Ok(());
            }

            Command::SetInterest { item, disposition } => {
                let item = item.trim().to_lowercase();
                let mut settings = self.profile.clone();
                settings.likes.retain(|i| *i != item);
                settings.dislikes.retain(|i| *i != item);
                match disposition {
                    InterestDisposition::Like => settings.likes.push(item),
                    InterestDisposition::Dislike => settings.dislikes.push(item),
                    InterestDisposition::Remove => {}
                }
                return self.apply(Command::UpdateProfile { settings });
            }
            Command::UpdateProfile { settings } => {
                let profile = settings.normalize_against(&self.profile)?;
                return self.commit_profile(profile);
            }
            Command::UpdateProfilePrepared {
                settings,
                expected_revision,
            } => {
                ensure!(
                    expected_revision == self.view.profile.revision,
                    "Your profile changed while the picture was processing. Reload and try again."
                );
                return self.commit_profile(settings);
            }

            Command::SetUserRule {
                username,
                ignored,
                banned,
            } => {
                ensure!(
                    !username.eq_ignore_ascii_case(&self.secrets.username),
                    "You cannot ignore or ban your own account"
                );
                ensure!(
                    self.view.user_rules.len() < 512
                        || self
                            .view
                            .user_rules
                            .iter()
                            .any(|r| r.username.eq_ignore_ascii_case(&username)),
                    "User control limit reached (512)"
                );
                self.flush_messages()?;
                if !ignored && !banned {
                    self.store
                        .connection
                        .execute("DELETE FROM user_rules WHERE username=?1", [&username])?;
                } else {
                    self.store.connection.execute("INSERT INTO user_rules VALUES(?1,?2,?3) ON CONFLICT(username) DO UPDATE SET ignored=excluded.ignored,banned=excluded.banned",rusqlite::params![username,ignored,banned])?;
                }
                if banned {
                    self.store.connection.execute("UPDATE uploads SET status='cancelled',speed=0,updated_at=strftime('%s','now') WHERE username=?1 COLLATE NOCASE AND status IN ('queued','uploading','checking_shares')",[&username])?;
                    self.view.transfer_revision += 1;
                }
                self.view.user_rules = users::rules(&self.store.connection)?;
                self.view.message_revision += 1;
                self.view.room_unread = crate::messages::unread(
                    &self.store.connection,
                    &self.secrets.username,
                    false,
                    None,
                )?;
                self.view.private_unread = crate::messages::unread(
                    &self.store.connection,
                    &self.secrets.username,
                    true,
                    None,
                )?;
                if let Some(c) = &self.client {
                    c.set_banned_users(
                        self.view
                            .user_rules
                            .iter()
                            .filter(|r| r.banned)
                            .map(|r| r.username.clone())
                            .collect(),
                    );
                }
                return Ok(());
            }
            Command::CancelUpload { username, path } => {
                ensure!(
                    self.client
                        .as_ref()
                        .is_some_and(|c| c.cancel_upload(&username, &path)),
                    "Upload is no longer active"
                );
                self.store.connection.execute("UPDATE uploads SET status='cancelled',speed=0,updated_at=strftime('%s','now') WHERE username=?1 AND path=?2 AND status IN ('queued','uploading','checking_shares')",rusqlite::params![username,path])?;
                self.view.transfer_revision += 1;
                return Ok(());
            }
            Command::AddWish { query } => {
                ensure!(
                    self.view.wishlist.len() < 128,
                    "Wishlist limit reached (128)"
                );
                self.store
                    .connection
                    .execute(
                        "INSERT INTO wishlist(id,query,enabled) VALUES(?1,?2,1)",
                        rusqlite::params![uuid::Uuid::new_v4().to_string(), query.trim()],
                    )
                    .context("This query is already in your wishlist")?;
                self.view.wishlist = wishlist::load(&self.store.connection)?;
                return Ok(());
            }
            Command::ClearWishResults { id } => {
                if let Some(key) = self
                    .view
                    .wishlist
                    .iter()
                    .find(|w| w.id == id)
                    .and_then(|w| w.search_id.clone())
                {
                    if let Some(c) = &self.client {
                        let _ = c.forget_search(&key);
                    }
                    self.searches.remove(&key);
                    self.view.searches.retain(|s| s.id != key);
                    self.store
                        .connection
                        .execute("DELETE FROM searches WHERE id=?1", [&key])?;
                }
                self.store
                    .connection
                    .execute("UPDATE wishlist SET last_count=0 WHERE id=?1", [id])?;
                self.view.wishlist = wishlist::load(&self.store.connection)?;
                return Ok(());
            }
            Command::UpdateWish { id, enabled } => {
                if !enabled
                    && let Some(key) = self
                        .view
                        .wishlist
                        .iter()
                        .find(|w| w.id == id)
                        .and_then(|w| w.search_id.clone())
                {
                    if let Some(c) = &self.client {
                        let _ = c.forget_search(&key);
                    }
                    self.searches.remove(&key);
                    if let Some(s) = self.view.searches.iter_mut().find(|s| s.id == key) {
                        s.collecting = false;
                    }
                }
                self.store.connection.execute(
                    "UPDATE wishlist SET enabled=?1 WHERE id=?2",
                    rusqlite::params![enabled, id],
                )?;
                self.view.wishlist = wishlist::load(&self.store.connection)?;
                return Ok(());
            }
            Command::RemoveWish { id } => {
                if let Some(wish) = self.view.wishlist.iter().find(|w| w.id == id)
                    && let Some(search) = &wish.search_id
                {
                    if let Some(c) = &self.client {
                        let _ = c.forget_search(search);
                    }
                    self.searches.remove(search);
                    self.view.searches.retain(|s| s.id != *search);
                    self.store
                        .connection
                        .execute("DELETE FROM searches WHERE id=?1", [search])?;
                }
                self.store
                    .connection
                    .execute("DELETE FROM wishlist WHERE id=?1", [id])?;
                self.view.wishlist = wishlist::load(&self.store.connection)?;
                return Ok(());
            }
            Command::RunWish { id } => return self.run_wish(&id),
            Command::OpenWish { id } => return self.open_wish(&id),
            Command::WatchUsers { usernames } => {
                for user in usernames {
                    self.focused_users.insert(user, Instant::now());
                }
                while self.focused_users.len() > 256 {
                    if let Some(old) = self
                        .focused_users
                        .iter()
                        .min_by_key(|(_, t)| *t)
                        .map(|(u, _)| u.clone())
                    {
                        self.focused_users.remove(&old);
                    }
                }
                return Ok(());
            }
            Command::Search { query } => return self.begin_search(query, SearchScope::Global),
            Command::SearchScoped { query, scope } => return self.begin_search(query, scope),
            Command::UpdateLibrarySettings { settings } => {
                self.store.save_library_settings(&settings)?;
                self.view.library_settings = settings;
                self.view.notice = Some("Library startup preference saved".into());
                return Ok(());
            }
            Command::SetBuddy { username, buddy } => {
                ensure!(
                    !buddy
                        || self.view.buddies.len() < 128
                        || self.view.buddies.contains(&username),
                    "Buddy limit reached (128)"
                );
                self.store.set_buddy(&username, buddy)?;
                self.view.buddies = self.store.buddies()?;
                if let Some(c) = &self.client {
                    c.set_upload_policy(self.view.sharing_policy.protocol(&self.view.buddies));
                }
                self.view.notice = Some(
                    if buddy {
                        "Added to buddies"
                    } else {
                        "Removed from buddies"
                    }
                    .into(),
                );
                return Ok(());
            }
            Command::Discover { query, refresh } => {
                self.discovery.begin(
                    &self.store.connection,
                    &self.secrets.username,
                    self.client.as_ref(),
                    &mut self.view.discovery,
                    query,
                    refresh,
                )?;
                let users = self
                    .view
                    .buddies
                    .iter()
                    .chain(self.view.users.keys())
                    .chain(self.view.room_members.values().flatten())
                    .take(1000)
                    .cloned()
                    .collect();
                let excluded = self
                    .view
                    .user_rules
                    .iter()
                    .filter(|r| r.banned || r.ignored)
                    .map(|r| r.username.clone())
                    .collect();
                self.discovery.seed_dislikes(users, excluded);
                return Ok(());
            }
            Command::LookupUser { username } => {
                ensure!(
                    self.client.is_some() || username == self.secrets.username,
                    "Connect to Soulseek to look up another user"
                );
                if self.lookups.len() >= 32
                    && !self.lookups.contains_key(&username)
                    && let Some(old) = self
                        .lookups
                        .iter()
                        .min_by_key(|(_, t)| *t)
                        .map(|(u, _)| u.clone())
                {
                    self.lookups.remove(&old);
                    self.profiles.remove(&old);
                }
                self.lookups.insert(username.clone(), Instant::now());
                self.profiles.remove(&username);
                self.view
                    .users
                    .entry(username.clone())
                    .or_default()
                    .profile_error = None;
                // A new lookup is explicit refresh, including a formerly unknown name.
                if username == self.secrets.username {
                    self.refresh_users();
                    return Ok(());
                }
                if let Some(c) = &self.client {
                    c.unwatch_user(&username)?;
                    c.watch_user(&username)?;
                    self.watched.insert(username.clone(), Instant::now());
                    if let Err(e) = c.request_user_interests(&username) {
                        crate::activity::record(
                            "warn",
                            "daemon",
                            &format!("Request user interests: {e}"),
                        );
                    }
                }
                let interests =
                    crate::discovery::load_interests(&self.store.connection, &username)?;
                self.view.users.insert(
                    username,
                    UserView {
                        status: "unknown".into(),
                        pending: true,
                        interests,
                        interests_pending: true,
                        ..Default::default()
                    },
                );
                return Ok(());
            }
            Command::CloseBrowse { username } => {
                self.store
                    .connection
                    .execute("DELETE FROM browse_folders WHERE username=?1", [&username])?;
                self.browsing.remove(&username);
                self.view.browse.remove(&username);
                self.store
                    .connection
                    .execute("DELETE FROM browse_files WHERE username=?1", [username])?;
                self.store.save_snapshot(&self.view)?;
                return Ok(());
            }
            Command::Browse { ref username } if *username == self.secrets.username => {
                let listing = crate::browse::save_with_limit(
                    &self.store.connection,
                    username,
                    self.index.files().iter().map(|f| FileView {
                        username: username.clone(),
                        path: f.virtual_path.clone(),
                        size: f.size,
                        free_slots: 0,
                        speed: 0,
                        attributes: f.attributes.iter().copied().collect(),
                    }),
                    self.config.limits.browse_files,
                )?;
                self.view.browse.insert(username.clone(), listing);
                self.store.save_snapshot(&self.view)?;
                return Ok(());
            }
            Command::ClearTransfers { direction, status } => {
                match direction {
                    HistoryDirection::Downloads => {
                        let active: std::collections::HashSet<_> = self
                            .client
                            .as_ref()
                            .map(|c| {
                                c.get_all_downloads()
                                    .into_iter()
                                    .filter(|d| !d.is_finished())
                                    .map(|d| (d.username, d.filename))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let previous = self.queue.clone();
                        let mut removed = vec![];
                        self.queue.retain(|r| {
                            let remove = !active.contains(&(r.username.clone(), r.path.clone()))
                                && status.matches(&saved_view(r).status);
                            if remove {
                                removed.push((r.username.clone(), r.path.clone()));
                            }
                            !remove
                        });
                        if let Err(e) = self.persist() {
                            self.queue = previous;
                            return Err(e);
                        }
                        for (user, path) in &removed {
                            self.retries.remove(&(user.clone(), path.clone()));
                            self.store.connection.execute(
                                "DELETE FROM download_retries WHERE username=?1 AND path=?2",
                                rusqlite::params![user, path],
                            )?;
                        }
                        if let Some(c) = &self.client {
                            for (user, path) in &removed {
                                let _ = c.remove_download(user, path);
                            }
                        }
                        self.view.downloads =
                            crate::transfers::preview(&self.store.connection, "downloads")
                                .unwrap_or_default();
                        self.view.transfer_revision += 1;
                        self.view.notice = Some(format!(
                            "Cleared {} download history records",
                            removed.len()
                        ));
                    }
                    HistoryDirection::Uploads => {
                        let key = match status {
                            HistoryStatus::All => "all",
                            HistoryStatus::Completed => "completed",
                            HistoryStatus::Failed => "failed",
                            HistoryStatus::Cancelled => "cancelled",
                            _ => "none",
                        };
                        if let Some(c) = &self.client {
                            c.clear_upload_history(key);
                        }
                        let n = crate::transfers::clear_uploads(&self.store.connection, &status)?;
                        self.view.uploads =
                            crate::transfers::preview(&self.store.connection, "uploads")?;
                        self.view.transfer_revision += 1;
                        self.view.notice = Some(format!("Cleared {n} upload history records"));
                    }
                }
                return Ok(());
            }
            Command::ClearMessageHistory {
                target,
                private,
                range,
            } => {
                self.flush_messages()?;
                let at = now();
                let (count, ids) = self
                    .store
                    .clear_message_history_filtered(&target, private, &range, at)?;
                self.message_ids = ids;
                self.view.messages.retain(|m| {
                    !(m.private == private
                        && (if private {
                            m.target.eq_ignore_ascii_case(&target)
                        } else {
                            m.target == target
                        })
                        && range.matches(m.timestamp, at))
                });
                self.refresh_unread(&target, private)?;
                self.view.message_reset = ids.0;
                self.view.message_revision += 1;
                self.view.notice = Some(format!("Deleted {count} saved messages from {target}"));
                return Ok(());
            }
            Command::DismissMessages => {
                self.view.room_management.invitations_error = None;
                for summary in self.view.room_management.rooms.values_mut() {
                    summary.error = None;
                }
                return Ok(());
            }
            Command::CheckNetwork => {
                self.network_worker.check();
                return Ok(());
            }
            Command::UpdateNetwork { settings } => {
                ensure!(
                    self.config
                        .published_transfer_port
                        .is_none_or(|port| port == settings.port),
                    "This container uses a fixed published port. Change RUSTYSOUL_PUBLISHED_TRANSFER_PORT and its matching TCP port mapping in the deployment and recreate it to change the port"
                );
                ensure!(
                    self.config.published_transfer_port.is_none() || !settings.upnp,
                    "Automatic UPnP requires host networking; use manual forwarding for a bridge-network container"
                );
                ensure!(
                    settings.port == self.network.port || self.client.is_none(),
                    "Disconnect from Soulseek before changing the transfer port"
                );
                if settings.port != self.network.port {
                    let _probe = std::net::TcpListener::bind(("0.0.0.0", settings.port))
                        .context("That transfer port is already used; choose another")?;
                }
                self.store.save_network(&settings)?;
                self.network = settings;
                self.storage
                    .set_minimum(self.network.minimum_free_space_mib);
                self.storage_retry.clear();
                self.system.request_refresh();
                if !self.network.auto_reconnect {
                    self.retry = None;
                }
                if let Some(c) = &self.client {
                    self.configure_transfers(c);
                }
                self.network_worker
                    .update(self.network.clone(), self.client.is_some());
                self.view.network = self.network.clone();
                self.view.notice = Some("Connection settings saved".into());
                return Ok(());
            }
            Command::DownloadBatch { files } => return self.queue_files(files),
            Command::DownloadFolder {
                username,
                folder,
                recursive,
            } => {
                ensure!(
                    self.view
                        .browse
                        .get(&username)
                        .is_some_and(|b| !b.pending && !b.truncated),
                    "Refresh a complete share listing before queueing a folder"
                );
                let mut stmt=self.store.connection.prepare("SELECT path,size FROM browse_files WHERE username=?1 AND (folder=?2 OR (?3 AND substr(folder,1,length(?2)+1)=?2||char(92))) ORDER BY path LIMIT ?4")?;
                let files = stmt
                    .query_map(
                        rusqlite::params![
                            username,
                            folder,
                            recursive,
                            (self.config.limits.transfers + 1) as i64
                        ],
                        |r| {
                            Ok(DownloadFile {
                                username: username.clone(),
                                path: r.get(0)?,
                                size: r.get::<_, i64>(1)? as u64,
                            })
                        },
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                drop(stmt);
                ensure!(
                    !files.is_empty(),
                    "This folder contains no downloadable files"
                );
                return self.queue_files(files);
            }
            Command::UpdateRetryPolicy { settings } => {
                self.store.connection.execute("INSERT INTO retry_policy VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(&settings)?])?;
                if !settings.enabled {
                    for row in &mut self.queue {
                        if row.status == "retry_waiting" {
                            row.status = "failed".into();
                        }
                    }
                    self.persist()?;
                }
                self.view.retry_policy = settings;
                self.view.notice = Some("Download retry settings saved".into());
                return Ok(());
            }
            Command::RescanShares => {
                self.library.request(
                    self.locations_generation,
                    self.config.soulseek.shares.clone(),
                )?;
                self.view.library.scanning = true;
                self.view.library.phase = "queued".into();
                self.view.library.trigger = "manual".into();
                self.view.notice = Some("Incremental library re-index queued".into());
                return Ok(());
            }
            Command::UpdateLocations { downloads, shares } => {
                let locations = crate::library::validate_locations(
                    &FileLocations { downloads, shares },
                    &self.config.data_dir,
                )?;
                crate::library::check_location_roots(&locations, &self.config.location_roots)?;
                self.store.save_locations(&locations)?;
                if locations.shares != self.locations.shares {
                    self.index = std::sync::Arc::new(soulseek_rs::shares::Shares::empty());
                    if let Some(c) = &self.client {
                        c.replace_share_index(locations.shares.clone(), self.index.clone())?;
                    }
                }
                self.locations = locations;
                self.system
                    .set_downloads(self.locations.downloads.clone().into());
                self.locations_generation += 1;
                self.config.soulseek.shares =
                    self.locations.shares.iter().map(Into::into).collect();
                self.view.locations = self.locations.clone();
                self.view.library.scanning = true;
                self.library.request(
                    self.locations_generation,
                    self.config.soulseek.shares.clone(),
                )?;
                self.view.notice=Some("File locations saved. Library update queued; existing downloads keep their original destinations.".into());
                return Ok(());
            }
            Command::UpdateCredentials { username, password } => {
                ensure!(
                    self.client.is_none(),
                    "Disconnect before changing credentials"
                );
                let password = match password {
                    Some(p) => p,
                    None => {
                        ensure!(
                            username == self.secrets.username && !self.secrets.password.is_empty(),
                            "Enter a password for this username"
                        );
                        self.secrets.password.clone()
                    }
                };
                self.view.connection = "verifying".into();
                self.view.notice = Some("Verifying your Soulseek login…".into());
                self.publish();
                let mut client = self.authenticate(&username, &password)?;
                if let Err(e) = self
                    .store
                    .save_credentials(&self.vault.encrypt(&username, &password)?)
                {
                    client.disconnect();
                    return Err(e);
                }
                self.clear_presence();

                self.client = Some(client);
                self.network_worker.update(self.network.clone(), true);
                self.view.connection = "connected".into();
                self.secrets.username = username.clone();
                self.secrets.password = password;
                self.view.room_unread = crate::messages::unread(
                    &self.store.connection,
                    &self.secrets.username,
                    false,
                    None,
                )?;
                self.view.private_unread = crate::messages::unread(
                    &self.store.connection,
                    &self.secrets.username,
                    true,
                    None,
                )?;
                self.view.username = username;
                self.view.discovery =
                    crate::discovery::load(&self.store.connection, &self.secrets.username, None)?;
                self.view.room_management =
                    crate::rooms::load(&self.store.connection, &self.secrets.username)?;
                self.room_controller
                    .connected(&mut self.view.room_management);
                self.view.subscribed_rooms = self
                    .store
                    .connection
                    .prepare("SELECT room FROM room_subscriptions ORDER BY rowid")?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                self.view
                    .subscribed_rooms
                    .extend(crate::rooms::private_subscriptions(
                        &self.store.connection,
                        &self.secrets.username,
                    )?);
                self.view.credentials_configured = true;
                self.view.credentials_revision += 1;
                self.retry = None;
                self.view.notice =
                    Some("Soulseek login verified. Account saved and connected.".into());
                return Ok(());
            }
            Command::ClearCredentials => {
                ensure!(
                    self.client.is_none(),
                    "Disconnect before removing credentials"
                );
                // Persist an encrypted empty record so environment bootstrap values do not return on restart.
                self.store.save_credentials(&self.vault.encrypt("", "")?)?;
                self.secrets.username.clear();
                self.secrets.password.clear();
                self.view.username.clear();
                self.view.discovery = Default::default();
                self.view.room_management = Default::default();
                self.view.users.clear();
                self.view.credentials_configured = false;
                self.view.credentials_revision += 1;
                self.retry = None;
                return Ok(());
            }
            Command::Connect => return self.connect(),
            Command::Disconnect => {
                for row in &mut self.queue {
                    if matches!(row.status.as_str(), "queued" | "downloading") {
                        row.status = "interrupted".into();
                    }
                }
                self.persist()?;
                self.clear_presence();
                self.retry = None;
                if let Some(c) = self.client.as_mut() {
                    c.disconnect();
                }
                self.sample_traffic(true);
                self.client = None;
                crate::transfers::recover_uploads(&self.store.connection)?;
                self.view.transfer_revision += 1;
                self.network_worker.update(self.network.clone(), false);
                self.searches.clear();
                for search in &mut self.view.searches {
                    search.collecting = false;
                }
                self.browsing.clear();
                for listing in self.view.browse.values_mut() {
                    listing.pending = false;
                }
                self.view.joined_rooms.clear();
                self.view.uploads = crate::transfers::preview(&self.store.connection, "uploads")?;
                self.view.connection = "disconnected".into();
                self.view.downloads =
                    crate::transfers::preview(&self.store.connection, "downloads")
                        .unwrap_or_default();
                return Ok(());
            }
            Command::ForgetSearch { id } => {
                if let Some((_, _)) = self.searches.remove(&id)
                    && let Some(c) = &self.client
                {
                    let _ = c.forget_search(&id);
                }
                self.view.searches.retain(|s| s.id != id);
                self.store.save_snapshot(&self.view)?;
                return Ok(());
            }
            Command::Transfer {
                username,
                path,
                action: TransferAction::Remove,
            } => {
                if let Some(c) = &self.client {
                    let active = c
                        .get_all_downloads()
                        .iter()
                        .any(|d| d.username == username && d.filename == path && !d.is_finished());
                    ensure!(!active, "Cancel the active transfer before removing it");
                    let _ = c.remove_download(&username, &path);
                }
                self.queue
                    .retain(|d| d.username != username || d.path != path);
                self.persist()?;
                self.view
                    .downloads
                    .retain(|d| d.username != username || d.path != path);
                return Ok(());
            }
            _ => {}
        }
        let c = self.client.as_ref().context("Connect to Soulseek first")?;
        match command {
            Command::Download {
                username,
                path,
                size,
            } => {
                ensure!(
                    self.queue
                        .iter()
                        .filter(|r| !automation::finished(&r.status))
                        .count()
                        < self.config.limits.transfers,
                    "Transfer limit reached; remove completed downloads"
                );
                ensure!(
                    !self
                        .queue
                        .iter()
                        .any(|d| d.username == username && d.path == path),
                    "File already queued; use resume or remove it first"
                );
                self.queue.push(SavedDownload {
                    local_path: String::new(),
                    directory: self.locations.downloads.clone(),
                    username: username.clone(),
                    path: path.clone(),
                    size,
                    status: "waiting".into(),
                });
                // Commit intent before asking the peer; an I/O failure never starts an untracked download.
                if let Err(e) = self.persist() {
                    self.queue.pop();
                    return Err(e);
                }
                self.pump_downloads()?;
            }
            Command::Transfer {
                username,
                path,
                action,
            } => {
                let row = self
                    .queue
                    .iter()
                    .find(|r| r.username == username && r.path == path)
                    .context("Transfer not found")?
                    .clone();
                match action {
                    TransferAction::Pause => {
                        if matches!(row.status.as_str(), "waiting" | "waiting_for_space") {
                            self.queue
                                .iter_mut()
                                .find(|r| r.username == username && r.path == path)
                                .unwrap()
                                .status = "paused_waiting".into();
                            self.persist()?;
                            return Ok(());
                        }
                        ensure!(
                            c.pause_download(&username, &path),
                            "Transfer cannot be paused"
                        );
                    }
                    TransferAction::Cancel => {
                        if matches!(
                            row.status.as_str(),
                            "waiting" | "paused_waiting" | "retry_waiting" | "waiting_for_space"
                        ) {
                            self.queue
                                .iter_mut()
                                .find(|r| r.username == username && r.path == path)
                                .unwrap()
                                .status = "cancelled".into();
                            self.persist()?;
                            return Ok(());
                        }
                        ensure!(
                            c.cancel_download(&username, &path),
                            "Transfer cannot be cancelled"
                        );
                    }
                    TransferAction::Resume => {
                        self.retries.remove(&(username.clone(), path.clone()));
                        self.store.connection.execute(
                            "DELETE FROM download_retries WHERE username=?1 AND path=?2",
                            rusqlite::params![username, path],
                        )?;
                        ensure!(
                            !automation::finished(&row.status)
                                || self
                                    .queue
                                    .iter()
                                    .filter(|r| !automation::finished(&r.status))
                                    .count()
                                    < self.config.limits.transfers,
                            "Download queue is full"
                        );
                        ensure!(row.status != "completed", "Transfer is already complete");
                        if !c.resume_download(&username, &path) {
                            let _ = c.remove_download(&username, &path);
                            self.queue
                                .iter_mut()
                                .find(|r| r.username == username && r.path == path)
                                .unwrap()
                                .status = "waiting".into();
                            self.persist()?;
                            self.pump_downloads()?;
                        }
                    }
                    TransferAction::Remove => unreachable!(),
                }
            }
            Command::RefreshRooms => c.request_room_list()?,
            Command::SendMessage {
                target,
                text,
                private,
            } => {
                if private {
                    c.send_private_message(&target, &text)?;
                } else {
                    ensure!(
                        self.view.joined_rooms.contains(&target),
                        "Join the room first"
                    );
                    c.say_in_room(&target, &text)?;
                }
                if private {
                    self.view.messages.push(MessageView {
                        id: next_message_id(&self.store, &mut self.message_ids).unwrap_or(0),
                        target,
                        username: self.secrets.username.clone(),
                        text,
                        private,
                        timestamp: now(),
                    });
                }
            }
            Command::Browse { username } => {
                c.browse_user(&username)?;
                self.browsing.insert(username.clone(), Instant::now());
                let listing = self.view.browse.entry(username).or_default();
                listing.pending = true;
                listing.error = None;
            }
            Command::SetAway { away } => {
                c.set_away(away)?;
                self.view.away = away;
                self.idle.reset(Instant::now());
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn queue_files(&mut self, files: Vec<DownloadFile>) -> Result<()> {
        ensure!(self.client.is_some(), "Connect to Soulseek first");
        for f in &files {
            Command::Download {
                username: f.username.clone(),
                path: f.path.clone(),
                size: f.size,
            }
            .validate()
            .map_err(anyhow::Error::msg)?;
        }
        let mut seen = std::collections::HashSet::new();
        let files: Vec<_> = files
            .into_iter()
            .filter(|f| {
                seen.insert((f.username.clone(), f.path.clone()))
                    && !self
                        .queue
                        .iter()
                        .any(|r| r.username == f.username && r.path == f.path)
            })
            .collect();
        ensure!(
            self.queue
                .iter()
                .filter(|r| !automation::finished(&r.status))
                .count()
                + files.len()
                <= self.config.limits.transfers,
            "Not enough download queue space; choose a smaller folder or increase limits.transfers"
        );
        let previous = self.queue.len();
        for f in &files {
            self.queue.push(SavedDownload {
                local_path: String::new(),
                username: f.username.clone(),
                path: f.path.clone(),
                size: f.size,
                status: "waiting".into(),
                directory: self.locations.downloads.clone(),
            });
        }
        if let Err(e) = self.persist() {
            self.queue.truncate(previous);
            return Err(e);
        }
        self.pump_downloads()?;
        self.view.notice = Some(format!(
            "Queued {} files; existing queue entries were skipped",
            files.len()
        ));
        Ok(())
    }
    fn begin_search(&mut self, query: String, scope: SearchScope) -> Result<()> {
        let c = self.client.as_ref().context("Connect to Soulseek first")?;
        let id = uuid::Uuid::new_v4().to_string();
        let view = SearchView {
            id: id.clone(),
            query: query.clone(),
            scope: scope.clone(),
            collecting: true,
            ..Default::default()
        };
        crate::search::create(&self.store.connection, &view)?;
        let result = match &scope {
            SearchScope::Global => c.search_keyed(&id, &query, None, None, false),
            SearchScope::User { username } => {
                c.search_keyed(&id, &query, Some(username), None, false)
            }
            SearchScope::Room { room } => c.search_keyed(&id, &query, None, Some(room), false),
        };
        if let Err(e) = result {
            self.store
                .connection
                .execute("DELETE FROM searches WHERE id=?1", [&id])?;
            return Err(e.into());
        }
        self.searches.insert(id, (query, Instant::now()));
        self.view.searches.push(view);
        crate::search::trim_previews(&mut self.view.searches);
        Ok(())
    }
    fn pump_downloads(&mut self) -> Result<()> {
        if automation::advance(
            &self.view.retry_policy,
            &mut self.queue,
            &mut self.retries,
            now(),
        ) {
            self.persist()?;
        }
        let Some(c) = &self.client else {
            return Ok(());
        };
        let active = c.get_all_downloads();
        let (total, users) = c.download_slot_usage();
        let mut pending: std::collections::HashSet<_> = active
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::Queued))
            .map(|d| d.username.to_lowercase())
            .collect();
        let unfinished: std::collections::HashSet<_> = self
            .queue
            .iter()
            .filter(|r| !automation::finished(&r.status))
            .map(|r| (r.username.as_str(), r.path.as_str()))
            .collect();
        self.storage_volumes
            .retain(|key, _| unfinished.contains(&(key.0.as_str(), key.1.as_str())));
        self.storage_retry
            .retain(|key, _| unfinished.contains(&(key.0.as_str(), key.1.as_str())));
        self.storage
            .retain(&self.storage_volumes.values().copied().collect());
        let mut planned = vec![];
        let mut changes = vec![];
        let mut order: Vec<_> = (0..self.queue.len())
            .filter(|i| {
                matches!(
                    self.queue[*i].status.as_str(),
                    "waiting" | "waiting_for_space"
                )
            })
            .collect();
        order.sort_by_key(|i| {
            let row = &self.queue[*i];
            let r = crate::queue_order::rule(
                &self.queue_rules,
                "downloads",
                &row.username,
                &row.path,
                u64::MAX,
            );
            (std::cmp::Reverse(r.priority), r.position, *i)
        });
        for i in order {
            if total >= self.network.download_slots {
                break;
            }
            let row = self.queue[i].clone();
            if !matches!(row.status.as_str(), "waiting" | "waiting_for_space")
                || users
                    .get(&row.username.to_lowercase())
                    .copied()
                    .unwrap_or(0)
                    >= self.network.effective_download_slots_per_user()
                || pending.contains(&row.username.to_lowercase())
            {
                continue;
            }
            let key = (row.username.clone(), row.path.clone());
            if self
                .storage_retry
                .get(&key)
                .is_some_and(|at| *at > Instant::now())
            {
                continue;
            }
            let prepared = self.prepare_download(&row);
            let status = match &prepared {
                Ok(Some(_)) => "queued",
                Ok(None) => "waiting_for_space",
                Err(_) => "failed",
            };
            if row.status != status {
                changes.push((i, row.status.clone()));
                self.queue[i].status = status.into();
            }
            match prepared {
                Ok(Some(dir)) => {
                    pending.insert(row.username.to_lowercase());
                    planned.push((i, row, dir));
                }
                Ok(None) => {}
                Err(e) => self.view.error = Some(format!("{e:#}")),
            }
        }
        if changes.is_empty() && planned.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.persist() {
            for (i, old) in changes {
                self.queue[i].status = old;
            }
            return Err(error);
        }
        let mut failed = false;
        for (i, row, dir) in planned {
            #[cfg(unix)]
            let work_path = {
                use std::os::fd::AsRawFd;
                format!("/proc/self/fd/{}", dir.as_raw_fd())
            };
            #[cfg(not(unix))]
            let work_path = std::path::Path::new(&row.directory)
                .join(crate::downloads::staging_relative(&row).parent().unwrap())
                .to_string_lossy()
                .into_owned();
            let result = self.client.as_ref().context("Not connected")?.download(
                row.path.clone(),
                row.username.clone(),
                row.size,
                work_path,
            );
            match result {
                Ok(_) => {
                    self.work_dirs.insert((row.username, row.path), dir);
                }
                Err(e) => {
                    self.queue[i].status = "failed".into();
                    self.view.error = Some(e.to_string());
                    failed = true;
                }
            }
        }
        if failed {
            self.persist()?;
        }
        Ok(())
    }
    fn prepare_download(&mut self, row: &SavedDownload) -> Result<Option<cap_std::fs::Dir>> {
        let key = (row.username.clone(), row.path.clone());
        if self.storage.enabled() {
            let known = self.storage_volumes.get(&key).copied().or_else(|| {
                (row.directory == self.locations.downloads)
                    .then(|| self.storage.primary())
                    .flatten()
            });
            if let Some(id) = known
                && self.storage.can_start(id, row.size).is_err()
            {
                return Ok(None);
            }
            if row.directory == self.locations.downloads && self.storage.primary().is_none() {
                return Ok(None);
            }
        }
        let directory = crate::downloads::work_directory(row);
        #[cfg(unix)]
        if directory.as_ref().is_err_and(|error| {
            error.chain().any(|cause| {
                cause.downcast_ref::<std::io::Error>().is_some_and(|e| {
                    e.raw_os_error().is_some_and(|n| {
                        matches!(
                            rustix::io::Errno::from_raw_os_error(n),
                            rustix::io::Errno::NOSPC | rustix::io::Errno::DQUOT
                        )
                    })
                })
            })
        }) {
            self.storage_retry
                .insert(key, Instant::now() + Duration::from_secs(30));
            return Ok(None);
        }
        let dir = directory.with_context(|| {
            format!(
                "Cannot prepare download folder {} for {}",
                row.directory, row.username
            )
        })?;
        if self.storage.enabled() {
            let id = match self.storage.register(&dir, false) {
                Ok(id) => id,
                Err(_) => return Ok(None),
            };
            self.storage_volumes.insert(key, id);
            if self.storage.can_start(id, row.size).is_err() {
                self.system.request_refresh();
                return Ok(None);
            }
        }
        Ok(Some(dir))
    }

    fn poll_idle(&mut self) {
        if let Some(client) = &self.client
            && let Some(away) = self.idle.desired(
                Instant::now(),
                self.profile.auto_away_minutes,
                self.view.away,
            )
            && client.set_away(away).is_ok()
        {
            self.view.away = away;
            self.idle.automatic = away;
        }
    }
    fn refresh(&mut self) {
        self.poll_idle();
        if let Some(client) = &self.client
            && self
                .privileges
                .poll(client.own_privilege_observation(), Instant::now(), now())
            && client.check_privileges().is_err()
        {
            self.privileges.failed();
        }
        self.room_controller.tick(&mut self.view.room_management);
        self.sample_traffic(false);
        if let Some(c) = &self.client
            && let Err(e) = self.discovery.poll(
                &self.store.connection,
                &self.secrets.username,
                c,
                &mut self.view.discovery,
                now(),
            )
        {
            self.view.error = Some(format!("Save discovery results: {e:#}"));
        }
        if self.client.is_none() {
            self.refresh_users();
            self.publish();
            return;
        }
        let Some(c) = &self.client else {
            return;
        };
        if let Some(loss) = c.session_loss() {
            let displaced = matches!(loss, soulseek_rs::SessionLoss::Displaced);
            if let Some(c) = self.client.as_mut() {
                c.disconnect();
            }
            self.sample_traffic(true);
            self.client = None;
            let _ = crate::transfers::recover_uploads(&self.store.connection);
            self.view.transfer_revision += 1;
            self.clear_presence();
            self.network_worker.update(self.network.clone(), false);
            self.view.connection = "disconnected".into();
            self.view.error = Some(loss.to_string());
            for row in &mut self.queue {
                if matches!(row.status.as_str(), "queued" | "downloading") {
                    row.status = if !displaced && self.network.auto_reconnect {
                        "waiting"
                    } else {
                        "interrupted"
                    }
                    .into();
                }
            }
            if let Err(e) = self.persist() {
                self.view.error = Some(format!("{e:#}"));
            }
            self.view.downloads =
                crate::transfers::preview(&self.store.connection, "downloads").unwrap_or_default();
            self.searches.clear();
            for search in &mut self.view.searches {
                search.collecting = false;
            }
            self.view.joined_rooms.clear();
            if !displaced && self.network.auto_reconnect {
                self.retry = Some(Instant::now() + Duration::from_secs(self.retry_seconds));
                self.retry_seconds = (self.retry_seconds * 2).min(300);
            }
            self.publish();
            return;
        }
        let (folders, files) = c.shared_counts();
        self.view.shares = ShareCounts { folders, files };
        let recent = self
            .view
            .searches
            .len()
            .saturating_sub(crate::search::SNAPSHOT_PREVIEWS);
        for (index, search) in self.view.searches.iter_mut().enumerate() {
            if let Some((_, started)) = self.searches.get(&search.id) {
                if !search.collecting {
                    continue;
                }
                let (responses, truncated) = c.take_search_results(&search.id);
                if !responses.is_empty() {
                    let incoming = responses.into_iter().flat_map(|r| {
                        r.files.into_iter().map(move |f| FileView {
                            username: f.username,
                            path: f.name,
                            size: f.size,
                            free_slots: r.slots,
                            speed: r.speed,
                            attributes: f.attribs.into_iter().collect(),
                        })
                    });
                    match crate::search::append(
                        &self.store.connection,
                        &search.id,
                        incoming,
                        self.config.limits.search_results,
                    ) {
                        Ok((total, limited)) => {
                            search.total = total;
                            search.truncated |= limited || truncated;
                            search.revision += 1;
                            search.results = if index >= recent {
                                crate::search::preview(&self.store.connection, &search.id)
                                    .unwrap_or_default()
                            } else {
                                Vec::new()
                            };
                        }
                        Err(e) => self.view.error = Some(e.to_string()),
                    }
                }
                if let Some(id) = &search.wishlist_id
                    && let Some(wish) = self.view.wishlist.iter_mut().find(|w| w.id == *id)
                    && wish.last_count != search.total
                {
                    wish.last_count = search.total;
                    let _ = self.store.connection.execute(
                        "UPDATE wishlist SET last_count=?1 WHERE id=?2",
                        rusqlite::params![search.total as i64, id],
                    );
                }
                search.collecting =
                    started.elapsed() < Duration::from_secs(120) && !search.truncated;
                if !search.collecting {
                    let _ = c.forget_search(&search.id);
                    self.searches.remove(&search.id);
                }
            }
        }
        self.view.downloads =
            crate::transfers::preview(&self.store.connection, "downloads").unwrap_or_default();
        let mut changed = false;
        for (user, path) in c.take_deferred_downloads() {
            if let Some(row) = self
                .queue
                .iter_mut()
                .find(|r| r.username == user && r.path == path && r.status == "queued")
            {
                row.status = "waiting".into();
                changed = true;
                self.work_dirs.remove(&(user, path));
            }
        }
        for d in c.get_all_downloads() {
            let mut status = download_status(&d.status).to_string();
            let storage_held = matches!(d.status, DownloadStatus::WaitingForSpace { .. });
            if storage_held {
                let delay = if matches!(&d.status,DownloadStatus::WaitingForSpace{reason,..} if reason.contains("quota"))
                {
                    30
                } else {
                    5
                };
                self.storage_retry.insert(
                    (d.username.clone(), d.filename.clone()),
                    Instant::now() + Duration::from_secs(delay),
                );
                let _ = c.remove_download(&d.username, &d.filename);
            }

            if let Some(row) = self
                .queue
                .iter_mut()
                .find(|r| r.username == d.username && r.path == d.filename)
            {
                if status == "completed" && row.local_path.is_empty() {
                    match crate::downloads::organize(row) {
                        Ok(Some(path)) => {
                            row.local_path = path.to_string_lossy().into_owned();
                            changed = true;
                        }
                        Ok(None) => {
                            status = "finalizing".into();
                            self.view.error = Some(
                                "The downloaded file is unavailable at its staging location."
                                    .into(),
                            );
                        }
                        Err(e) => {
                            status = "finalizing".into();
                            self.view.error =
                                Some(format!("Could not organize a completed download: {e}"));
                        }
                    }
                }
                if row.status != status {
                    if matches!(status.as_str(), "failed" | "timed_out") {
                        let key = (d.username.clone(), d.filename.clone());
                        let mut retry = self.retries.get(&key).cloned().unwrap_or_default();
                        if automation::plan(&self.view.retry_policy, &mut retry, now()) {
                            match automation::save(
                                &self.store.connection,
                                &d.username,
                                &d.filename,
                                &retry,
                            ) {
                                Ok(()) => {
                                    self.retries.insert(key, retry);
                                    status = "retry_waiting".into();
                                    let _ = c.remove_download(&d.username, &d.filename);
                                }
                                Err(e) => self.view.error = Some(e.to_string()),
                            }
                        }
                    }
                    row.status = status.clone();
                    changed = true;
                }
                if row.size != d.size {
                    row.size = d.size;
                    changed = true;
                }
            }
            let transfer = TransferView {
                username: d.username.clone(),
                path: d.filename.clone(),
                size: d.size,
                transferred: d.bytes_downloaded(),
                speed: d.speed_bytes_per_sec(),
                status,
                queue_position: d.queue_position,
                ..Default::default()
            };
            match crate::transfers::record_download(
                &self.store.connection,
                &transfer,
                d.progress_bytes(),
            ) {
                Ok(changed) => {
                    if changed {
                        self.view.transfer_revision += 1;
                    }
                }
                Err(e) => self.view.error = Some(e.to_string()),
            }
        }
        if let Ok(batch) = c.upload_snapshot() {
            let uploads: Vec<_> = batch
                .uploads()
                .iter()
                .cloned()
                .map(|u| TransferView {
                    username: u.username,
                    path: u.filename,
                    size: u.size,
                    transferred: u.bytes_sent,
                    speed: u.speed_bytes_per_sec,
                    detail: match &u.status {
                        soulseek_rs::UploadStatus::Queued(0) =>
                            "Upload slot reserved; waiting for the receiving peer to answer the transfer offer.".into(),
                        soulseek_rs::UploadStatus::Failed(e)
                        | soulseek_rs::UploadStatus::TimedOut(e) => e.clone(),
                        _ => String::new(),
                    },
                    status: match u.status {
                        soulseek_rs::UploadStatus::Queued(_) => "queued",
                        soulseek_rs::UploadStatus::CheckingShares(_) => "checking_shares",
                        soulseek_rs::UploadStatus::Failed(ref e)
                            if e.starts_with("Sharing protection:") =>
                        {
                            "blocked"
                        }
                        soulseek_rs::UploadStatus::InProgress => "uploading",
                        soulseek_rs::UploadStatus::Completed => "completed",
                        soulseek_rs::UploadStatus::Cancelled => "cancelled",
                        soulseek_rs::UploadStatus::TimedOut(_) => "timed_out",
                        _ => "failed",
                    }
                    .into(),
                    queue_position: match u.status {
                        soulseek_rs::UploadStatus::Queued(n)
                        | soulseek_rs::UploadStatus::CheckingShares(n) => Some(n),
                        _ => None,
                    },
                    ..Default::default()
                })
                .collect();
            let mut recorded = true;
            for t in &uploads {
                match crate::transfers::record_upload(&self.store.connection, t) {
                    Ok(changed) => {
                        if changed {
                            self.view.transfer_revision += 1;
                        }
                    }
                    Err(e) => {
                        recorded = false;
                        self.view.error = Some(format!("{e:#}"));
                    }
                }
            }
            if recorded {
                match crate::transfers::reconcile_uploads(
                    &self.store.connection,
                    batch.current_keys(),
                ) {
                    Ok(changed) => {
                        if changed {
                            self.view.transfer_revision += 1;
                        }
                        c.acknowledge_upload_snapshot(batch);
                    }
                    Err(e) => {
                        self.view.error = Some(format!("Could not reconcile upload history: {e:#}"))
                    }
                }
            }
        }
        self.view.uploads =
            crate::transfers::preview(&self.store.connection, "uploads").unwrap_or_default();
        self.view.rooms = c
            .room_list()
            .into_iter()
            .take(1000)
            .map(|r| RoomView {
                last_activity: self.room_activity.get(&r.name).copied(),
                name: r.name,
                users: r.user_count,
            })
            .collect();
        for (room, summary) in &self.view.room_management.rooms {
            if summary.private && summary.member && !self.view.rooms.iter().any(|r| r.name == *room)
            {
                self.view.rooms.push(RoomView {
                    name: room.clone(),
                    users: if self.view.joined_rooms.contains(room) {
                        c.room_members(room).len().min(u32::MAX as usize) as u32
                    } else {
                        summary.users.unwrap_or(0)
                    },
                    last_activity: self.room_activity.get(room).copied(),
                });
            }
        }
        for event in c.take_room_events() {
            let private_join = matches!(&event, RoomEvent::PrivateJoined { .. });
            let rejected_public = matches!(&event,RoomEvent::Joined{room,..} if self.room_controller.expects_private(room));
            match self.room_controller.event(
                &self.store.connection,
                &self.secrets.username,
                &mut self.view.room_management,
                &event,
                now(),
            ) {
                Ok(rooms) => {
                    for room in rooms {
                        if !self.view.joined_rooms.contains(&room)
                            && !self.room_controller.pending(&room)
                            && c.join_private_room(&room).is_ok()
                        {
                            self.room_controller
                                .joining_private(&mut self.view.room_management, &room);
                        }
                    }
                }
                Err(e) => self.view.error = Some(format!("Update room state: {e:#}")),
            }
            match event {
                RoomEvent::Joined { room, .. } | RoomEvent::PrivateJoined { room, .. } => {
                    if rejected_public {
                        let _ = c.leave_room(&room);
                        self.view.error=Some("The private-room request returned a public room. RustySoul left it; choose another name.".into());
                        self.view.subscribed_rooms.retain(|r| r != &room);
                        continue;
                    }
                    if !self.view.joined_rooms.contains(&room) {
                        self.view.joined_rooms.push(room.clone());
                    }
                    if private_join && !self.view.subscribed_rooms.contains(&room) {
                        self.view.subscribed_rooms.push(room);
                    }
                }
                RoomEvent::Tickers { room, .. } => {
                    if let Some(ticker) = self
                        .view
                        .room_management
                        .rooms
                        .get(&room)
                        .map(|s| s.own_ticker.clone())
                        .filter(|_| self.view.room_management.rooms[&room].own_ticker_set)
                    {
                        let _ = c.set_room_ticker(&room, &ticker);
                    }
                }
                RoomEvent::OwnStandingChanged {
                    room,
                    members: true,
                    granted: false,
                } => {
                    self.view.joined_rooms.retain(|r| r != &room);
                    self.view.subscribed_rooms.retain(|r| r != &room);
                    let _ = c.leave_room(&room);
                }
                RoomEvent::CantCreate { room } => {
                    self.view.subscribed_rooms.retain(|r| r != &room);
                    self.view.error = Some(format!(
                        "Could not join {room}; it may be private or unavailable."
                    ));
                }
                RoomEvent::Left { room } => self.view.joined_rooms.retain(|r| r != &room),
                RoomEvent::Message {
                    room,
                    username,
                    message,
                } => {
                    if self
                        .view
                        .user_rules
                        .iter()
                        .any(|r| r.username.eq_ignore_ascii_case(&username) && r.ignored)
                    {
                        continue;
                    }
                    let time = now();
                    self.room_activity.insert(room.clone(), time);
                    if let Some(r) = self.view.rooms.iter_mut().find(|r| r.name == room) {
                        r.last_activity = Some(time);
                    }
                    self.view.messages.push(MessageView {
                        id: next_message_id(&self.store, &mut self.message_ids).unwrap_or(0),
                        target: room,
                        username,
                        text: message,
                        private: false,
                        timestamp: now(),
                    });
                }
                _ => {}
            }
        }
        self.view.subscribed_rooms.retain(|room| {
            !self
                .view
                .room_management
                .rooms
                .get(room)
                .is_some_and(|s| s.private && s.confirmed && !s.member)
        });
        self.view.joined_rooms.retain(|room| {
            let lost = self
                .view
                .room_management
                .rooms
                .get(room)
                .is_some_and(|s| s.private && s.confirmed && !s.member);
            if lost {
                let _ = c.leave_room(room);
            }
            !lost
        });
        for message in c.take_private_messages() {
            if self
                .view
                .user_rules
                .iter()
                .any(|r| r.username.eq_ignore_ascii_case(message.username()) && r.ignored)
            {
                continue;
            }
            self.away_replied
                .retain(|_, at| at.elapsed() < Duration::from_secs(900));
            if self.view.away
                && !self.profile.away_message.is_empty()
                && message.username() != self.secrets.username
                && !self.away_replied.contains_key(message.username())
                && self.away_replied.len() < 256
                && c.send_private_message(message.username(), &self.profile.away_message)
                    .is_ok()
            {
                self.away_replied
                    .insert(message.username().into(), Instant::now());
            }
            self.view.messages.push(MessageView {
                id: next_message_id(&self.store, &mut self.message_ids).unwrap_or(0),
                target: message.username().into(),
                username: message.username().into(),
                text: message.message().into(),
                private: true,
                timestamp: now(),
            });
        }
        let pending: Vec<_> = self.browsing.keys().cloned().collect();
        for username in pending {
            if let Some(dirs) = c.take_browse_result(&username) {
                let files = dirs.into_iter().flat_map(|dir| {
                    let username = username.clone();
                    let name = dir.name;
                    dir.files.into_iter().map(move |f| FileView {
                        username: username.clone(),
                        path: format!("{}\\{}", name, f.name),
                        size: f.size,
                        free_slots: 0,
                        speed: 0,
                        attributes: f.attributes.into_iter().collect(),
                    })
                });
                // Each listing is replaced transactionally; a failed refresh keeps the old cache.
                match crate::browse::save_with_limit(
                    &self.store.connection,
                    &username,
                    files,
                    self.config.limits.browse_files,
                ) {
                    Ok(listing) => {
                        self.view.browse.insert(username.clone(), listing);
                    }
                    Err(e) => {
                        if let Some(b) = self.view.browse.get_mut(&username) {
                            b.pending = false;
                            b.error = Some(e.to_string());
                        }
                    }
                }
                self.browsing.remove(&username);
            } else if self.browsing[&username].elapsed() > Duration::from_secs(45) {
                self.browsing.remove(&username);
                if let Some(b) = self.view.browse.get_mut(&username) {
                    b.pending = false;
                    b.error=Some("Peer browse timed out; the user may be offline or unreachable. Retry browsing when they are online.".into());
                }
            }
        }
        self.refresh_users();
        if changed {
            self.view.transfer_revision += 1;
        }
        if changed {
            match self.persist() {
                Ok(()) => {
                    let keys: Vec<_> = self.work_dirs.keys().cloned().collect();
                    for key in keys {
                        if let Some(row) = self
                            .queue
                            .iter()
                            .find(|r| r.username == key.0 && r.path == key.1)
                            && crate::engine::automation::finished(&row.status)
                        {
                            if row.status == "completed"
                                && !row.local_path.is_empty()
                                && let Err(e) = crate::downloads::remove_staging(row)
                            {
                                self.view.error = Some(format!(
                                    "Completed file is safe, but staging cleanup failed: {e}"
                                ));
                                continue;
                            }
                            if let Some(c) = &self.client {
                                let _ = c.remove_download(&key.0, &key.1);
                            }
                            self.work_dirs.remove(&key);
                        }
                    }
                }
                Err(e) => self.view.error = Some(e.to_string()),
            }
        }
        if let Err(e) = self.pump_downloads() {
            self.view.error = Some(format!("{e:#}"));
        }
        if let Err(error) = self.flush_messages() {
            self.view.error = Some(format!("Save messages: {error:#}"));
        }
        if self.last_save.elapsed() >= Duration::from_secs(2) {
            if let Err(e) = self
                .store
                .save_snapshot(&self.view)
                .and_then(|_| self.store.save_downloads(&self.queue))
            {
                self.view.error = Some(format!("{e:#}"));
            }
            self.last_save = Instant::now();
        }
        self.view.downloads =
            crate::transfers::preview(&self.store.connection, "downloads").unwrap_or_default();
        self.publish();
    }
}

fn next_message_id(store: &Store, ids: &mut (u64, u64)) -> Result<u64> {
    if ids.0 >= ids.1 {
        *ids = store.reserve_message_ids()?;
    }
    let id = ids.0;
    ids.0 += 1;
    Ok(id)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn download_status(status: &DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Queued => "queued",
        DownloadStatus::InProgress { .. } => "downloading",
        DownloadStatus::Paused { .. } => "paused",
        DownloadStatus::Completed => "completed",
        DownloadStatus::Cancelled => "cancelled",
        DownloadStatus::Failed(_) => "failed",
        DownloadStatus::WaitingForSpace { .. } => "waiting_for_space",
        DownloadStatus::TimedOut => "timed_out",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn more_than_eight_peer_libraries_survive_restart_and_do_not_block_own_browse() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = Config {
            data_dir: dir.path().into(),
            ..Default::default()
        };
        let secrets = || Secrets::new("s".repeat(32), "myself".into(), "eightpwd".into()).unwrap();
        let mut store = Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        let mut view = Snapshot::default();
        for i in 0..40 {
            view.browse
                .insert(format!("peer-{i}"), BrowseView::default());
        }
        store.save_snapshot(&view).unwrap();
        drop(store);
        let h = start(config.clone(), secrets()).unwrap();
        assert_eq!(h.snapshot.borrow().browse.len(), 40);
        h.commands
            .send(Command::Browse {
                username: "myself".into(),
            })
            .unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while !h.snapshot.borrow().browse.contains_key("myself") {
            assert!(Instant::now() < end, "{:?}", h.snapshot.borrow().error);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(h.snapshot.borrow().browse.len(), 41);
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let h = start(config, secrets()).unwrap();
        assert_eq!(h.snapshot.borrow().browse.len(), 41);
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
    }
    #[test]
    fn archived_chat_history_and_offline_deletion_survive_daemon_restarts() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = dir.path().join("rustysoul.sqlite3");
        let mut store = Store::open(&path).unwrap();
        let mut view = Snapshot::default();
        for id in 1..=1201 {
            view.messages.push(MessageView {
                id,
                target: if id == 1 { "archived-peer" } else { "room" }.into(),
                username: "self".into(),
                text: format!("message {id}"),
                private: id == 1,
                timestamp: 1,
            });
        }
        store.save_snapshot(&view).unwrap();
        drop(store);
        let mut config = Config {
            data_dir: dir.path().into(),
            ..Default::default()
        };
        config.soulseek.auto_connect = false;
        let secrets = || Secrets::new("s".repeat(32), "self".into(), "password".into()).unwrap();
        let h = start(config.clone(), secrets()).unwrap();
        assert_eq!(h.snapshot.borrow().messages.len(), 500);
        assert!(!h.snapshot.borrow().messages.iter().any(|m| m.private));
        let previous_reset = h.snapshot.borrow().message_reset;
        h.commands
            .send(Command::ClearMessageHistory {
                range: Default::default(),
                target: "archived-peer".into(),
                private: true,
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(4);
        while h.snapshot.borrow().message_reset == previous_reset {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(h.snapshot.borrow().error.is_none());
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let h = start(config, secrets()).unwrap();
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1200
        );
        assert!(
            crate::messages::page(
                &store.connection,
                &crate::messages::Query {
                    target: "archived-peer".into(),
                    private: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .messages
            .is_empty()
        );
    }
    #[test]
    fn private_alerts_and_ignore_controls_are_durable_and_independent_of_room_cursors() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = dir.path().join("rustysoul.sqlite3");
        let mut store = Store::open(&path).unwrap();
        store
            .save_messages(&[
                MessageView {
                    id: 1,
                    target: "room".into(),
                    username: "PEER".into(),
                    text: "room message".into(),
                    private: false,
                    timestamp: 1,
                },
                MessageView {
                    id: 2,
                    target: "Peer".into(),
                    username: "peer".into(),
                    text: "direct message".into(),
                    private: true,
                    timestamp: 2,
                },
                MessageView {
                    id: 3,
                    target: "other".into(),
                    username: "other".into(),
                    text: "another direct".into(),
                    private: true,
                    timestamp: 3,
                },
            ])
            .unwrap();
        drop(store);
        let mut config = Config {
            data_dir: dir.path().into(),
            ..Default::default()
        };
        config.soulseek.auto_connect = false;
        let secret = || Secrets::new("s".repeat(32), "self".into(), "password".into()).unwrap();
        let h = start(config.clone(), secret()).unwrap();
        let wait = |h: &EngineHandle, predicate: fn(&Snapshot) -> bool| {
            let until = Instant::now() + Duration::from_secs(5);
            while !predicate(&h.snapshot.borrow()) {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&h, |v| {
            v.private_unread.get("peer") == Some(&1)
                && v.private_unread.get("other") == Some(&1)
                && v.room_unread.get("room") == Some(&1)
        });
        h.commands
            .send(Command::SetUserRule {
                username: "Peer".into(),
                ignored: true,
                banned: true,
            })
            .unwrap();
        wait(&h, |v| {
            !v.private_unread.contains_key("peer")
                && !v.room_unread.contains_key("room")
                && v.messages
                    .iter()
                    .all(|m| !m.username.eq_ignore_ascii_case("peer"))
        });
        h.commands
            .send(Command::SetUserRule {
                username: "PEER".into(),
                ignored: false,
                banned: false,
            })
            .unwrap();
        wait(&h, |v| {
            v.private_unread.get("peer") == Some(&1) && v.room_unread.get("room") == Some(&1)
        });
        h.commands
            .send(Command::ReadPrivate {
                username: "PEER".into(),
                through_id: 2,
            })
            .unwrap();
        wait(&h, |v| {
            !v.private_unread.contains_key("peer")
                && v.private_unread.get("other") == Some(&1)
                && v.room_unread.get("room") == Some(&1)
        });
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let h = start(config, secret()).unwrap();
        wait(&h, |v| {
            v.private_read.get("peer") == Some(&2)
                && !v.private_unread.contains_key("peer")
                && v.private_unread.get("other") == Some(&1)
        });
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
    #[test]
    fn room_read_cursors_are_independent_durable_and_message_ids_do_not_restart() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = dir.path().join("rustysoul.sqlite3");
        let mut store = Store::open(&path).unwrap();
        let mut v = Snapshot::default();
        for (id, room) in [(1, "Room A"), (2, "Room B"), (3, "Room A")] {
            v.messages.push(MessageView {
                id,
                target: room.into(),
                username: "other".into(),
                text: "activity".into(),
                private: false,
                timestamp: 1,
            });
        }
        store.save_snapshot(&v).unwrap();
        drop(store);
        let config = Config {
            data_dir: dir.path().into(),
            ..Default::default()
        };
        let secrets = || Secrets::new("s".repeat(32), "self".into(), "password".into()).unwrap();
        let mut config = config;
        config.soulseek.auto_connect = false;
        let h = start(config.clone(), secrets()).unwrap();
        let wait = |h: &EngineHandle, a: usize, b: usize| {
            let until = Instant::now() + Duration::from_secs(4);
            while h
                .snapshot
                .borrow()
                .room_unread
                .get("Room A")
                .copied()
                .unwrap_or(0)
                != a
                || h.snapshot
                    .borrow()
                    .room_unread
                    .get("Room B")
                    .copied()
                    .unwrap_or(0)
                    != b
            {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&h, 2, 1);
        h.commands
            .send(Command::ReadRoom {
                room: "Room A".into(),
                through_id: 1,
            })
            .unwrap();
        wait(&h, 1, 1);
        h.commands
            .send(Command::ReadRoom {
                room: "Room A".into(),
                through_id: 3,
            })
            .unwrap();
        wait(&h, 0, 1);
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let h = start(config, secrets()).unwrap();
        wait(&h, 0, 1);
        assert_eq!(h.snapshot.borrow().room_read["Room A"], 3);
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.reserve_message_ids().unwrap().0 > 1024);
    }
    #[test]
    fn informational_notices_expire_without_changing_connection_or_files() {
        let dir = tempfile::tempdir().unwrap();
        let h = start(
            Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            Secrets::new("s".repeat(32), "".into(), "".into()).unwrap(),
        )
        .unwrap();
        h.commands
            .send(Command::ClearTransfers {
                direction: HistoryDirection::Downloads,
                status: HistoryStatus::All,
            })
            .unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        while h.snapshot.borrow().notice.is_none() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        let end = Instant::now() + Duration::from_secs(8);
        while h.snapshot.borrow().notice.is_some() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(h.snapshot.borrow().connection, "disconnected");
        assert!(h.snapshot.borrow().downloads.is_empty());
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
    }
    #[test]
    fn corrupt_database_is_never_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rustysoul.sqlite3"), b"broken").unwrap();
        let c = Config {
            data_dir: dir.path().into(),
            ..Default::default()
        };
        assert!(
            start(
                c,
                Secrets::new("a".repeat(32), "".into(), "".into()).unwrap()
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(dir.path().join("rustysoul.sqlite3")).unwrap(),
            b"broken"
        );
    }
    #[test]
    fn prepared_profile_updates_reject_a_stale_revision_without_overwriting_changes() {
        let dir = tempfile::tempdir().unwrap();
        let h = start(
            Config {
                data_dir: dir.path().into(),
                ..Default::default()
            },
            Secrets::new("s".repeat(32), "".into(), "".into()).unwrap(),
        )
        .unwrap();
        let settings = crate::profile::Profile {
            description: "Saved first".into(),
            ..Default::default()
        };
        h.commands
            .send(Command::UpdateProfilePrepared {
                settings,
                expected_revision: 0,
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while h.snapshot.borrow().profile.revision == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        h.commands
            .send(Command::UpdateProfilePrepared {
                settings: crate::profile::Profile {
                    description: "Stale overwrite".into(),
                    ..Default::default()
                },
                expected_revision: 0,
            })
            .unwrap();
        while h.snapshot.borrow().error.is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            h.snapshot
                .borrow()
                .error
                .as_ref()
                .unwrap()
                .contains("profile changed")
        );
        assert_eq!(h.snapshot.borrow().profile.revision, 1);
        h.stop.send(()).unwrap();
        h.thread.join().unwrap();
        let store = Store::open(&dir.path().join("rustysoul.sqlite3")).unwrap();
        assert_eq!(
            crate::profile::load(&store.connection).unwrap().description,
            "Saved first"
        );
    }
    #[test]
    fn daemon_starts_offline_and_stops_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config {
            data_dir: dir.path().into(),
            ..Default::default()
        };
        let handle = start(
            c,
            Secrets::new("a".repeat(32), "".into(), "".into()).unwrap(),
        )
        .unwrap();
        assert_eq!(handle.snapshot.borrow().connection, "disconnected");
        handle
            .commands
            .send(Command::Search {
                query: "offline".into(),
            })
            .unwrap();
        let end = Instant::now() + Duration::from_secs(3);
        while handle.snapshot.borrow().error.is_none() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        handle.commands.send(Command::DismissMessages).unwrap();
        while handle.snapshot.borrow().error.is_some() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(handle.snapshot.borrow().notice.is_none());
        handle.commands.send(Command::RescanShares).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while handle.snapshot.borrow().library.revision == 0
            || handle.snapshot.borrow().library.scanning
            || handle.snapshot.borrow().notice.is_some()
        {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(20));
        }
        handle.stop.send(()).unwrap();
        handle.thread.join().unwrap();
    }
}
