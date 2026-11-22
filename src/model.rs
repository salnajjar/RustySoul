use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub privileges: crate::privileges::View,
    pub system: crate::system::View,
    pub room_management: crate::rooms::View,
    pub discovery: crate::discovery::View,
    pub sharing_policy: crate::sharing::Settings,
    pub room_read: BTreeMap<String, u64>,
    pub room_unread: BTreeMap<String, usize>,
    pub private_read: BTreeMap<String, u64>,
    pub private_unread: BTreeMap<String, usize>,
    pub subscribed_rooms: Vec<String>,
    pub profile: crate::profile::View,
    pub effective_peer_limit: usize,
    pub traffic: crate::traffic::Totals,
    pub http_bind: String,
    pub port_binding: PortBinding,
    pub network: NetworkSettings,
    pub connectivity: Connectivity,
    pub transfer_capacity: usize,
    pub location_roots: Vec<String>,
    pub locations: FileLocations,
    pub library: LibraryStatus,
    pub notice: Option<String>,
    pub credentials_configured: bool,
    pub credentials_revision: u64,
    pub version: &'static str,
    pub connection: String,
    pub error: Option<String>,
    pub username: String,
    pub away: bool,
    pub away_automatic: bool,
    pub shares: ShareCounts,
    pub searches: Vec<SearchView>,
    pub downloads: Vec<TransferView>,
    pub uploads: Vec<TransferView>,
    pub rooms: Vec<RoomView>,
    pub joined_rooms: Vec<String>,
    pub messages: Vec<MessageView>,
    pub message_revision: u64,
    pub message_reset: u64,
    pub browse: BTreeMap<String, BrowseView>,
    pub users: BTreeMap<String, UserView>,
    pub buddies: Vec<String>,
    pub room_members: BTreeMap<String, Vec<String>>,
    pub library_settings: LibrarySettings,
    pub wishlist: Vec<WishView>,
    pub wishlist_next_at: u64,
    pub user_rules: Vec<UserRule>,
    pub retry_policy: RetryPolicy,
    pub download_capacity_used: usize,
    pub downloads_total: usize,
    pub uploads_total: usize,
    pub transfer_revision: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LibrarySettings {
    pub index_on_startup: bool,
}
#[derive(Clone, Default, Serialize)]
pub struct UserView {
    pub interests: crate::discovery::Interests,
    pub interests_pending: bool,
    pub interests_error: Option<String>,
    pub has_picture: bool,
    pub picture_revision: String,
    pub status: String,
    pub exists: Option<bool>,
    pub pending: bool,
    pub error: Option<String>,
    pub average_speed: Option<u32>,
    pub shared_files: Option<u32>,
    pub shared_folders: Option<u32>,
    pub description: Option<String>,
    pub upload_slots: Option<u32>,
    pub queue_size: Option<u32>,
    pub slots_free: Option<bool>,
    pub profile_pending: bool,
    pub profile_error: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileLocations {
    pub downloads: String,
    pub shares: Vec<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct LibraryStatus {
    pub scanning: bool,
    pub phase: String,
    pub scan_id: u64,
    pub trigger: String,
    pub processed: usize,
    pub folders: usize,
    pub bytes: u64,
    pub indexed: usize,
    pub changed: usize,
    pub skipped: usize,
    pub removed: usize,
    pub elapsed_ms: u64,
    pub revision: u64,
    pub watcher: String,
}
#[derive(Clone, Default, Serialize)]
pub struct ShareCounts {
    pub folders: u32,
    pub files: u32,
}
#[derive(Clone, Default, Serialize)]
pub struct SearchView {
    pub id: String,
    pub query: String,
    pub results: Vec<FileView>,
    pub collecting: bool,
    pub total: usize,
    pub revision: u64,
    pub truncated: bool,
    pub scope: SearchScope,
    pub wishlist_id: Option<String>,
    pub pinned: bool,
}
#[derive(Clone, Serialize)]
pub struct WishView {
    pub id: String,
    pub query: String,
    pub enabled: bool,
    pub last_run: Option<u64>,
    pub search_id: Option<String>,
    pub last_count: usize,
}
#[derive(Clone, Serialize)]
pub struct UserRule {
    pub username: String,
    pub ignored: bool,
    pub banned: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SearchScope {
    #[default]
    Global,
    User {
        username: String,
    },
    Room {
        room: String,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FileView {
    pub username: String,
    pub path: String,
    pub size: u64,
    pub free_slots: u8,
    pub speed: u32,
    pub attributes: BTreeMap<u32, u32>,
}
#[derive(Clone, Default, Serialize)]
pub struct TransferView {
    pub priority: i8,
    pub local_queue_order: u64,
    pub username: String,
    pub path: String,
    pub size: u64,
    pub transferred: u64,
    pub speed: f64,
    pub status: String,
    pub queue_position: Option<u32>,
    pub retry_at: Option<u64>,
    pub added_at: u64,
    pub updated_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub local_path: String,
    pub detail: String,
}
#[derive(Clone, Serialize)]
pub struct RoomView {
    pub name: String,
    pub users: u32,
    pub last_activity: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetryPolicy {
    pub enabled: bool,
    pub uploads_enabled: bool,
    pub max_attempts: u8,
    pub base_delay_seconds: u32,
    pub max_delay_seconds: u32,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            uploads_enabled: true,
            max_attempts: 3,
            base_delay_seconds: 15,
            max_delay_seconds: 300,
        }
    }
}
impl RetryPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=10).contains(&self.max_attempts)
            || !(5..=3600).contains(&self.base_delay_seconds)
            || self.max_delay_seconds < self.base_delay_seconds
            || self.max_delay_seconds > 86400
        {
            return Err("Use 1..10 retries, a base delay of 5..3600 seconds and a maximum delay up to one day".into());
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkSettings {
    pub check_for_updates: bool,
    pub minimum_free_space_mib: u64,
    pub port: u16,
    pub upnp: bool,
    pub auto_reconnect: bool,
    pub upload_slots: usize,
    pub max_peers: usize,
    pub download_limit_kib: u32,
    pub upload_limit_kib: u32,
    pub download_per_user_limit_kib: u32,
    pub upload_per_user_limit_kib: u32,
    pub download_slots: usize,
    pub download_slots_per_user: usize,
    pub automatic_download_parallelism: bool,
    pub upload_slots_per_user: usize,
    pub connect_on_startup: bool,
    pub user_limits: Vec<UserLimit>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct UserLimit {
    pub username: String,
    pub upload_limit_kib: u32,
    pub download_limit_kib: u32,
}
#[derive(Clone, Default, Serialize)]
pub struct PortBinding {
    pub fixed_port: Option<u16>,
}
impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            check_for_updates: true,
            minimum_free_space_mib: crate::storage::DEFAULT_MINIMUM_MIB,
            port: 2234,
            upnp: false,
            auto_reconnect: true,
            upload_slots: 2,
            max_peers: 64,
            download_limit_kib: 0,
            upload_limit_kib: 0,
            download_per_user_limit_kib: 0,
            upload_per_user_limit_kib: 0,
            download_slots: 4,
            download_slots_per_user: 1,
            automatic_download_parallelism: true,
            upload_slots_per_user: 1,
            connect_on_startup: true,
            user_limits: vec![],
        }
    }
}
impl NetworkSettings {
    pub fn effective_download_slots_per_user(&self) -> usize {
        if self.automatic_download_parallelism {
            self.download_slots.max(1)
        } else {
            self.download_slots_per_user
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.minimum_free_space_mib > crate::storage::MAX_MINIMUM_MIB {
            return Err("Choose a free-space reserve from 0 to 1,000,000 GiB".into());
        }
        if self.port < 1024 {
            return Err("Choose a transfer port from 1024 to 65535".into());
        }
        if self.upload_slots > 16
            || self.download_slots > 32
            || !(1..=16).contains(&self.upload_slots_per_user)
            || !(1..=32).contains(&self.download_slots_per_user)
            || !(1..=256).contains(&self.max_peers)
            || [
                self.download_limit_kib,
                self.upload_limit_kib,
                self.download_per_user_limit_kib,
                self.upload_per_user_limit_kib,
            ]
            .iter()
            .any(|r| *r > 1_000_000)
        {
            return Err("Invalid transfer resource limits".into());
        }
        if self.user_limits.len() > 32 {
            return Err("At most 32 individual user limits are supported".into());
        }
        let mut users = std::collections::HashSet::new();
        for u in &self.user_limits {
            crate::validation::username(&u.username)?;
            if !users.insert(&u.username) {
                return Err("A user can only have one speed override".into());
            }
            if u.upload_limit_kib > 1_000_000 || u.download_limit_kib > 1_000_000 {
                return Err("Invalid user speed limit".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Default, Serialize)]
pub struct Connectivity {
    pub state: String,
    pub message: String,
    pub gateway: Option<String>,
    pub external_ip: Option<String>,
    pub local_ip: Option<String>,
    pub mapped: bool,
    pub lease_seconds: u32,
    pub checked_at: u64,
    pub listener_ready: bool,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadFile {
    pub username: String,
    pub path: String,
    pub size: u64,
}
#[derive(Clone, Default, Serialize)]
pub struct MessageView {
    pub id: u64,
    pub target: String,
    pub username: String,
    pub text: String,
    pub private: bool,
    pub timestamp: u64,
}
#[derive(Clone, Default, Serialize)]
pub struct BrowseView {
    pub pending: bool,
    pub files: Vec<FileView>,
    pub truncated: bool,
    pub total: usize,
    pub revision: u64,
    pub error: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    RetryUploads {
        username: Option<String>,
    },
    UpdateStorageReserve {
        minimum_free_space_mib: u64,
    },
    SetRoomInvitations {
        enabled: bool,
    },
    CreatePrivateRoom {
        room: String,
    },
    SetRoomTicker {
        room: String,
        text: String,
    },
    PrivateRoomAction {
        room: String,
        action: crate::rooms::Action,
        username: Option<String>,
    },
    SetInterest {
        item: String,
        disposition: InterestDisposition,
    },
    Discover {
        query: crate::discovery::Query,
        #[serde(default)]
        refresh: bool,
    },
    #[serde(skip)]
    DownloadBatchConfirmed {
        files: Vec<DownloadFile>,
        reply: std::sync::mpsc::SyncSender<Result<usize, String>>,
    },
    RestartSearch {
        id: String,
        query: String,
    },
    RetryDownloads {
        username: Option<String>,
    },
    UpdateTransferQueue {
        direction: HistoryDirection,
        username: String,
        path: String,
        priority: Option<i8>,
        movement: Option<QueueMove>,
    },
    CancelDownloads {
        username: String,
        #[serde(default)]
        paths: Vec<String>,
    },
    CancelUploads {
        username: String,
        #[serde(default)]
        paths: Vec<String>,
    },
    UpdateSharingPolicy {
        settings: crate::sharing::Settings,
    },
    ReadRoom {
        room: String,
        through_id: u64,
    },
    ReadPrivate {
        username: String,
        through_id: u64,
    },
    UpdateProfile {
        settings: crate::profile::Profile,
    },
    #[serde(skip)]
    UpdateProfilePrepared {
        settings: crate::profile::Profile,
        expected_revision: u64,
    },
    UpdateRetryPolicy {
        settings: RetryPolicy,
    },
    DownloadFolder {
        username: String,
        folder: String,
        recursive: bool,
    },
    SetUserRule {
        username: String,
        ignored: bool,
        banned: bool,
    },
    CancelUpload {
        username: String,
        path: String,
    },
    AddWish {
        query: String,
    },
    UpdateWish {
        id: String,
        enabled: bool,
    },
    RemoveWish {
        id: String,
    },
    RunWish {
        id: String,
    },
    ClearWishResults {
        id: String,
    },
    OpenWish {
        id: String,
    },
    WatchUsers {
        usernames: Vec<String>,
    },
    LookupUser {
        username: String,
    },
    SetBuddy {
        username: String,
        buddy: bool,
    },
    CloseBrowse {
        username: String,
    },
    UpdateLibrarySettings {
        settings: LibrarySettings,
    },
    ClearTransfers {
        direction: HistoryDirection,
        status: HistoryStatus,
    },
    DismissMessages,
    ClearMessageHistory {
        target: String,
        private: bool,
        #[serde(default)]
        range: crate::messages::DeleteRange,
    },
    UpdateNetwork {
        settings: NetworkSettings,
    },
    CheckNetwork,
    UiActivity {
        #[serde(default)]
        idle_seconds: u32,
    },
    CheckPrivileges {
        #[serde(default)]
        watch_for_activation: bool,
    },
    DownloadBatch {
        files: Vec<DownloadFile>,
    },
    UpdateLocations {
        downloads: String,
        shares: Vec<String>,
    },
    UpdateCredentials {
        username: String,
        password: Option<String>,
    },
    ClearCredentials,
    Connect,
    Disconnect,
    Search {
        query: String,
    },
    SearchScoped {
        query: String,
        scope: SearchScope,
    },
    ForgetSearch {
        id: String,
    },
    Download {
        username: String,
        path: String,
        size: u64,
    },
    Transfer {
        username: String,
        path: String,
        action: TransferAction,
    },
    RefreshRooms,
    JoinRoom {
        room: String,
    },
    LeaveRoom {
        room: String,
    },
    SendMessage {
        target: String,
        text: String,
        private: bool,
    },
    Browse {
        username: String,
    },
    RescanShares,
    SetAway {
        away: bool,
    },
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueMove {
    Top,
    Up,
    Down,
    Bottom,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryDirection {
    Downloads,
    Uploads,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterestDisposition {
    Like,
    Dislike,
    Remove,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Blocked,
    All,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    TimedOut,
}
impl HistoryStatus {
    pub fn matches(&self, status: &str) -> bool {
        match self {
            Self::All => matches!(
                status,
                "completed" | "failed" | "cancelled" | "interrupted" | "timed_out" | "blocked"
            ),
            Self::Blocked => status == "blocked",
            Self::Completed => status == "completed",
            Self::Failed => status == "failed",
            Self::Cancelled => status == "cancelled",
            Self::Interrupted => status == "interrupted",
            Self::TimedOut => status == "timed_out",
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferAction {
    Pause,
    Resume,
    Cancel,
    Remove,
}
impl Command {
    pub fn validate(&self) -> Result<(), String> {
        use crate::validation::{remote_path, text, username};
        match self {
            Self::CreatePrivateRoom { room } => crate::validation::room(room),
            Self::SetRoomTicker { room, text } => {
                crate::validation::room(room)?;
                crate::rooms::ticker_valid(text)
            }
            Self::PrivateRoomAction {
                room,
                action,
                username,
            } => {
                crate::validation::room(room)?;
                action.validate(username)
            }
            Self::SetInterest { item, .. } => text(item, 80),
            Self::Discover { query, .. } => query.validate(),
            Self::RestartSearch { id, query } => {
                uuid::Uuid::parse_str(id).map_err(|_| "Invalid search ID")?;
                text(query, 256)
            }
            Self::UpdateTransferQueue {
                username: u,
                path,
                priority,
                movement,
                ..
            } => {
                username(u)?;
                remote_path(path)?;
                if priority.is_some() == movement.is_some()
                    || priority.is_some_and(|p| !(-1..=1).contains(&p))
                {
                    return Err("Choose one priority (-1, 0, 1) or queue movement".into());
                }
                Ok(())
            }
            Self::RetryDownloads { username: u } | Self::RetryUploads { username: u } => {
                if let Some(u) = u {
                    username(u)?;
                }
                Ok(())
            }
            Self::CancelUploads { username: u, paths }
            | Self::CancelDownloads { username: u, paths } => {
                username(u)?;
                if paths.len() > 256 {
                    return Err("Select at most 256 transfers per request".into());
                }
                for path in paths {
                    remote_path(path)?;
                }
                Ok(())
            }
            Self::UpdateSharingPolicy { settings } => settings.validate(),
            Self::ReadRoom {
                room: r,
                through_id,
            } => {
                crate::validation::room(r)?;
                if *through_id > i64::MAX as u64 {
                    return Err("Invalid room message cursor".into());
                }
                Ok(())
            }
            Self::ReadPrivate {
                username,
                through_id,
            } => {
                crate::validation::username(username)?;
                if *through_id > i64::MAX as u64 {
                    return Err("Invalid private message cursor".into());
                }
                Ok(())
            }
            Self::UpdateProfile { settings } | Self::UpdateProfilePrepared { settings, .. } => {
                settings.validate().map_err(|e| e.to_string())
            }
            Self::UpdateRetryPolicy { settings } => settings.validate(),
            Self::DownloadFolder {
                username: u,
                folder,
                ..
            } => {
                username(u)?;
                remote_path(folder)
            }
            Self::SetUserRule { username: u, .. } => username(u),
            Self::CancelUpload { username: u, path } => {
                username(u)?;
                remote_path(path)
            }
            Self::AddWish { query } => text(query, 256),
            Self::UpdateWish { id, .. }
            | Self::RemoveWish { id }
            | Self::ClearWishResults { id }
            | Self::RunWish { id }
            | Self::OpenWish { id } => uuid::Uuid::parse_str(id)
                .map(|_| ())
                .map_err(|_| "Invalid wishlist ID".into()),
            Self::WatchUsers { usernames } => {
                if usernames.len() > 100 {
                    return Err("At most 100 visible users per page".into());
                }
                for u in usernames {
                    username(u)?;
                }
                Ok(())
            }
            Self::UpdateNetwork { settings } => settings.validate(),
            Self::UpdateStorageReserve {
                minimum_free_space_mib,
            } => {
                if *minimum_free_space_mib <= crate::storage::MAX_MINIMUM_MIB {
                    Ok(())
                } else {
                    Err("Invalid free-space reserve".into())
                }
            }
            Self::DownloadBatch { files } | Self::DownloadBatchConfirmed { files, .. } => {
                if files.is_empty() || files.len() > 256 {
                    return Err("Select between 1 and 256 files".into());
                }
                for f in files {
                    Self::Download {
                        username: f.username.clone(),
                        path: f.path.clone(),
                        size: f.size,
                    }
                    .validate()?;
                }
                Ok(())
            }
            Self::UpdateLocations { downloads, shares } => {
                text(downloads, 4096)?;
                if shares.len() > 32 {
                    return Err("At most 32 share roots are supported".into());
                }
                for path in shares {
                    text(path, 4096)?;
                }
                Ok(())
            }
            Self::UpdateCredentials {
                username: u,
                password,
            } => {
                username(u)?;
                if let Some(p) = password
                    && (p.is_empty() || p.len() > 1024 || p.chars().any(char::is_control))
                {
                    return Err(
                        "Password must contain 1..1024 bytes without control characters".into(),
                    );
                }
                Ok(())
            }
            Self::UiActivity { idle_seconds } => {
                if *idle_seconds <= 86400 {
                    Ok(())
                } else {
                    Err("Activity age exceeds one day".into())
                }
            }
            Self::Search { query } => text(query, 256),
            Self::SearchScoped { query, scope } => {
                text(query, 256)?;
                match scope {
                    SearchScope::Global => Ok(()),
                    SearchScope::User { username: u } => username(u),
                    SearchScope::Room { room } => crate::validation::room(room),
                }
            }
            Self::ForgetSearch { id } => uuid::Uuid::parse_str(id)
                .map(|_| ())
                .map_err(|_| "Invalid search ID".into()),
            Self::Download {
                username: u,
                path,
                size,
            } => {
                username(u)?;
                remote_path(path)?;
                if *size > 16 * 1024_u64.pow(4) {
                    return Err("Invalid file size (0 bytes..16 TiB)".into());
                }
                Ok(())
            }
            Self::Transfer {
                username: u, path, ..
            } => {
                username(u)?;
                remote_path(path)
            }
            Self::JoinRoom { room } | Self::LeaveRoom { room } => crate::validation::room(room),
            Self::SendMessage {
                target,
                text: content,
                private,
            } => {
                if *private {
                    username(target)?;
                } else {
                    crate::validation::room(target)?;
                }
                text(content, 2000)
            }
            Self::ClearMessageHistory {
                target,
                private,
                range,
            } => {
                range.bounds(0).map_err(|e| e.to_string())?;
                if *private {
                    username(target)
                } else {
                    crate::validation::room(target)
                }
            }
            Self::Browse { username: u }
            | Self::LookupUser { username: u }
            | Self::SetBuddy { username: u, .. }
            | Self::CloseBrowse { username: u } => username(u),
            _ => Ok(()),
        }
    }
}
