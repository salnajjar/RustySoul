export interface FileResult {
  username: string;
  path: string;
  size: number;
  free_slots: number;
  speed: number;
  attributes: Record<string, number>;
}
export interface Transfer {
  priority?: number;
  local_queue_order?: number;
  username: string;
  path: string;
  size: number;
  transferred: number;
  speed: number;
  status: string;
  queue_position: number | null;
  retry_at?: number | null;
  local_path?: string;
  detail?: string;
  added_at?: number;
  updated_at?: number;
  started_at?: number | null;
  finished_at?: number | null;
}
export interface Search {
  id: string;
  query: string;
  results: FileResult[];
  collecting: boolean;
  total?: number;
  revision?: number;
  truncated?: boolean;
  scope?:
    | { type: "global" }
    | { type: "user"; username: string }
    | { type: "room"; room: string };
  wishlist_id?: string | null;
  pinned?: boolean;
}
export interface Message {
  id?: number;
  target: string;
  username: string;
  text: string;
  private: boolean;
  timestamp: number;
}
export interface ProfileSettings {
  description: string;
  picture: string | null;
  likes: string[];
  dislikes: string[];
  away_on_startup: boolean;
  auto_away_minutes?: number;
  away_message: string;
}
export interface SharingSettings {
  mode: "off" | "sharing" | "buddies";
  min_files: number;
  min_folders: number;
  exempt_buddies: boolean;
  exceptions: string[];
  allow_unknown: boolean;
  check_seconds: number;
  cache_seconds: number;
}
export interface PrivilegeStatus {
  official_server: boolean;
  remaining_seconds: number | null;
  checked_at: number | null;
  expires_at: number | null;
  pending: boolean;
  watching: boolean;
  error: string | null;
}
export interface Snapshot {
  message_revision?: number;
  message_reset?: number;
  privileges?: PrivilegeStatus;
  system?: SystemStatus;
  room_management?: RoomManagement;
  discovery?: DiscoveryView;
  sharing_policy?: SharingSettings;
  room_read?: Record<string, number>;
  room_unread?: Record<string, number>;
  subscribed_rooms?: string[];
  effective_peer_limit?: number;
  profile?: Omit<ProfileSettings, "picture"> & {
    has_picture: boolean;
    revision: number;
  };
  traffic?: { downloaded: number; uploaded: number; since: number };
  downloads_total?: number;
  uploads_total?: number;
  transfer_revision?: number;
  retry_policy: {
    enabled: boolean;
    max_attempts: number;
    base_delay_seconds: number;
    max_delay_seconds: number;
  };
  download_capacity_used?: number;
  wishlist: {
    id: string;
    query: string;
    enabled: boolean;
    last_run: number | null;
    search_id: string | null;
    last_count: number;
  }[];
  wishlist_next_at: number;
  user_rules: { username: string; ignored: boolean; banned: boolean }[];
  users: Record<string, UserInfo>;
  buddies: string[];
  room_members: Record<string, string[]>;
  library_settings: { index_on_startup: boolean };
  http_bind: string;
  port_binding: { fixed_port: number | null };
  network: {
    minimum_free_space_mib?: number;
    port: number;
    upnp: boolean;
    auto_reconnect: boolean;
    upload_slots: number;
    max_peers: number;
    download_limit_kib: number;
    upload_limit_kib: number;
    download_per_user_limit_kib: number;
    upload_per_user_limit_kib: number;
    download_slots: number;
    download_slots_per_user: number;
    upload_slots_per_user: number;
    connect_on_startup: boolean;
    user_limits: {
      username: string;
      upload_limit_kib: number;
      download_limit_kib: number;
    }[];
  };
  connectivity: {
    state: string;
    message: string;
    gateway: string | null;
    external_ip: string | null;
    local_ip: string | null;
    mapped: boolean;
    lease_seconds: number;
    checked_at: number;
    listener_ready: boolean;
  };
  transfer_capacity: number;
  location_roots: string[];
  locations: { downloads: string; shares: string[] };
  library: {
    trigger?: string;
    phase: string;
    scan_id: number;
    processed: number;
    folders: number;
    bytes: number;
    scanning: boolean;
    indexed: number;
    changed: number;
    skipped: number;
    removed: number;
    elapsed_ms: number;
    revision: number;
    watcher: string;
  };
  notice: string | null;
  credentials_configured: boolean;
  credentials_revision: number;
  version: string;
  connection: string;
  error: string | null;
  username: string;
  away: boolean;
  away_automatic?: boolean;
  shares: { folders: number; files: number };
  searches: Search[];
  downloads: Transfer[];
  uploads: Transfer[];
  rooms: { name: string; users: number; last_activity?: number | null }[];
  joined_rooms: string[];
  messages: Message[];
  browse: Record<
    string,
    {
      pending: boolean;
      files: FileResult[];
      truncated: boolean;
      total?: number;
      revision?: number;
      error?: string | null;
    }
  >;
}

export interface SystemStorage {
  path: string;
  total_bytes: number | null;
  used_bytes: number | null;
  available_bytes: number | null;
  error: string | null;
}
export interface SystemTransfers {
  total: number;
  states: Record<string, number>;
  speed: number;
}
export interface SystemStatus {
  sampled_at: number;
  uptime_seconds: number;
  process: {
    resident_bytes: number | null;
    threads: number | null;
    cpu_seconds: number | null;
    cpu_percent: number | null;
  };
  limits: {
    memory_scope?: "self" | "ancestor" | null;
    tasks_scope?: "self" | "ancestor" | null;
    memory_bytes: number | null;
    memory_used_bytes: number | null;
    tasks: number | null;
    tasks_used: number | null;
    cpu_cores: number | null;
  };
  data: SystemStorage;
  downloads: SystemStorage;
  database_bytes: number | null;
  wal_bytes: number | null;
  download_transfers: SystemTransfers | null;
  upload_transfers: SystemTransfers | null;
  process_error: string | null;
  database_error: string | null;
}
export const emptySnapshot: Snapshot = {
  retry_policy: {
    enabled: true,
    max_attempts: 3,
    base_delay_seconds: 15,
    max_delay_seconds: 300,
  },
  wishlist: [],
  wishlist_next_at: 0,
  user_rules: [],
  users: {},
  buddies: [],
  room_members: {},
  library_settings: { index_on_startup: false },
  http_bind: "127.0.0.1:8484",
  port_binding: { fixed_port: null },
  network: {
    minimum_free_space_mib: 1024,
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
    upload_slots_per_user: 1,
    connect_on_startup: true,
    user_limits: [],
  },
  connectivity: {
    state: "disabled",
    message: "",
    gateway: null,
    external_ip: null,
    local_ip: null,
    mapped: false,
    lease_seconds: 0,
    checked_at: 0,
    listener_ready: false,
  },
  transfer_capacity: 128,
  location_roots: [],
  locations: { downloads: "", shares: [] },
  library: {
    phase: "idle",
    scan_id: 0,
    processed: 0,
    folders: 0,
    bytes: 0,
    scanning: false,
    indexed: 0,
    changed: 0,
    skipped: 0,
    removed: 0,
    elapsed_ms: 0,
    revision: 0,
    watcher: "",
  },
  notice: null,
  credentials_configured: false,
  credentials_revision: 0,
  version: "0.1.0-beta.2",
  connection: "disconnected",
  error: null,
  username: "",
  away: false,
  shares: { folders: 0, files: 0 },
  searches: [],
  downloads: [],
  uploads: [],
  rooms: [],
  joined_rooms: [],
  messages: [],
  browse: {},
};
export interface UserInfo {
  interests?: { likes: string[]; dislikes: string[]; updated_at: number };
  interests_pending?: boolean;
  interests_error?: string | null;
  has_picture?: boolean;
  picture_revision?: string;
  status: string;
  exists: boolean | null;
  pending: boolean;
  error: string | null;
  average_speed: number | null;
  shared_files: number | null;
  shared_folders: number | null;
  description: string | null;
  upload_slots: number | null;
  queue_size: number | null;
  slots_free: boolean | null;
  profile_pending: boolean;
  profile_error: string | null;
}
export interface RoomSummary {
  private: boolean;
  owner: string | null;
  member: boolean;
  operator: boolean;
  confirmed: boolean;
  owner_live: boolean;
  members_known: boolean;
  operators_known: boolean;
  board_known: boolean;
  members: number;
  operators: number;
  board: number;
  own_ticker: string;
  own_ticker_set: boolean;
  updated_at: number;
  revision: number;
  pending: string | null;
  error: string | null;
}
export interface RoomManagement {
  accept_invitations: boolean;
  invitations_confirmed: boolean | null;
  invitations_error: string | null;
  rooms: Record<string, RoomSummary>;
}
export type DiscoveryQuery =
  | { mode: "personal" }
  | { mode: "popular" }
  | { mode: "topic"; item: string }
  | { mode: "dislikes"; item: string };
export interface DiscoveryView {
  dislike_evidence?: {
    checked: number;
    matched: number;
    cached: number;
    requested: number;
    timed_out: number;
    limited: boolean;
    missing_sources: number;
  } | null;
  query: DiscoveryQuery;
  recommendations: { item: string; rating: number }[];
  against: { item: string; rating: number }[];
  people: { username: string; weight: number | null }[];
  recommendations_at: number | null;
  people_at: number | null;
  recommendations_pending: boolean;
  people_pending: boolean;
  recommendations_error: string | null;
  people_error: string | null;
}
