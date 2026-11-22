import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  ArrowDown,
  ArrowUp,
  AudioLines,
  Search as SearchIcon,
  FolderOpen,
  MessageCircle,
  Radio,
  Settings,
  ChevronRight,
  X,
  Download,
  Pause,
  Play,
  Trash2,
  RefreshCw,
  LockKeyhole,
  LogOut,
  Check,
  Users,
  HardDrive,
  ShieldCheck,
  Compass,
  ListOrdered,
} from "lucide-react";
import { api, subscribe, bytes, basename, ApiError } from "./api";
import Credentials from "./Credentials";
import ApiAccess from "./ApiAccess";
import Maintenance from "./Maintenance";
import { useActivity } from "./useActivity";
import NetworkStatus from "./NetworkStatus";
import { PrivilegeBadge } from "./Privileges";
import Locations from "./Locations";
import ConnectionSettings from "./ConnectionSettings";
import SettingsNavigation, {
  settingsTabs,
  type SettingsTab,
} from "./SettingsNavigation";
import SystemMessage from "./SystemMessage";
import Appearance, { usePreferences } from "./Preferences";
import LibraryProgress from "./LibraryProgress";
import User, { UserContext, UserDialog, UserStatus } from "./User";
import ShareBrowser from "./ShareBrowser";
import SearchResults from "./SearchResults";
import SearchTabs from "./SearchTabs";
import Wishlist from "./Wishlist";
import PeopleSettings from "./PeopleSettings";
import SharingSettingsForm from "./SharingSettings";
import RetrySettings from "./RetrySettings";
import ReleaseNotice, { ReleaseSettings } from "./ReleaseUpdates";
import Profile from "./Profile";
import Traffic from "./Traffic";
import LoadMore from "./LoadMore";
import TransferRows from "./TransferRows";
import TransferTiming from "./TransferTiming";
import TransferSpeed from "./TransferSpeed";
import TransferQueueControls from "./TransferQueueControls";
import {
  useTransferSelection,
  transferKey,
  canCancelTransfer,
  canSelectTransfer,
  canRetryDownload,
  cancelTransferBatches,
  retryDownloadBatches,
} from "./TransferSelection";
import { useTransferFeed } from "./useTransferFeed";
import AudioPlayer from "./AudioPlayer";
import Logs from "./Logs";
import Backups from "./Backups";
import Discover from "./Discover";
import RoomTools, { RoomJoin } from "./RoomTools";
import ChatComposer from "./ChatComposer";
import HistoryDelete from "./HistoryDelete";
import MessageSearch from "./MessageSearch";
import AdvancedSearch, { type FileCriteria } from "./AdvancedSearch";
import SystemStatus from "./SystemStatus";
import ConversationMessages from "./ConversationMessages";
import { useConversations, useMessageHistory } from "./useMessageHistory";
import { usePrivateConversations } from "./usePrivateConversations";
import { SelectionProvider, useChoice, accepted } from "./WorkspaceState";
import { useRoomRead } from "./RoomRead";
import { userKey, isIgnored, unreadTotal } from "./chatState";
import {
  sortTransfers,
  progress,
  transferSorts,
  type TransferSort,
  eta,
  isQueuedTransfer,
} from "./transfers";
import { sortRooms, type RoomSort } from "./rooms";
import GroupedResults from "./GroupedResults";
import type { AudioFormat } from "./results";
import { LoginScreen, AccountForm, type Access } from "./Access";
import {
  emptySnapshot,
  type Snapshot,
  type FileResult,
  type Transfer,
} from "./types";

type Page =
  | "discover"
  | "search"
  | "downloads"
  | "uploads"
  | "browse"
  | "rooms"
  | "messages"
  | "settings";
const nav: { id: Page; label: string; icon: typeof SearchIcon }[] = [
  { id: "search", label: "Search", icon: SearchIcon },
  { id: "discover", label: "Discover", icon: Compass },
  { id: "downloads", label: "Downloads", icon: ArrowDown },
  { id: "uploads", label: "Uploads", icon: ArrowUp },
  { id: "browse", label: "Browse users", icon: FolderOpen },
  { id: "rooms", label: "Rooms", icon: Radio },
  { id: "messages", label: "Messages", icon: MessageCircle },
];

export default function App() {
  const [preferences, setPreferences] = usePreferences();
  const [dismissedError, setDismissedError] = useState("");
  const [dismissedNotice, setDismissedNotice] = useState<string | null>(null);
  const [dismissedConnection, setDismissedConnection] = useState("");
  const [access, setAccess] = useState<Access | null>(null);
  const [restoring, setRestoring] = useState(true);
  function acceptSession(session: Access) {
    setAccess({ token: "", account: session.account });
    setRestoring(false);
  }
  useEffect(() => {
    const abort = new AbortController();
    void api<Access["account"]>("", "auth/me", undefined, abort.signal)
      .then((account) => {
        if (account?.username) setAccess({ token: "", account });
      })
      .catch(() => {})
      .finally(() => {
        if (!abort.signal.aborted) setRestoring(false);
      });
    return () => abort.abort();
  }, []);
  const token = access?.token || "";
  const [state, setState] = useState<Snapshot>(emptySnapshot);
  const [scopePreset, setScopePreset] = useState<{
    type: "user" | "room";
    target: string;
  } | null>(null);
  const [selectedUser, setSelectedUser] = useState<string | null>(null);
  const [browseUser, setBrowseUser] = useState("");
  const [playing, setPlaying] = useState<Transfer | null>(null);
  const [roomTarget, setRoomTarget] = useState("");
  const [chatDrafts, setChatDrafts] = useState<Record<string, string>>({});
  const [messageUser, setMessageUser] = useState("");
  const [page, setPage] = useChoice<Page>("section", "search", [
    "search",
    "discover",
    "downloads",
    "uploads",
    "browse",
    "rooms",
    "messages",
    "settings",
  ]);
  const [settingsTab, setSettingsTab] = useChoice<SettingsTab>(
    "settingsSection",
    "accounts",
    settingsTabs.map((t) => t[0]),
  );
  const [error, setError] = useState("");
  const [live, setLive] = useState(false);
  const [retry, setRetry] = useState(0);
  const [busy, setBusy] = useState(false);
  const plannedRestart = useRef(0);
  useActivity(
    token,
    !!access &&
      !access.account.must_change &&
      state.connection === "connected" &&
      (state.profile?.auto_away_minutes ?? 0) > 0,
    !!state.away_automatic,
  );
  useEffect(() => {
    if (!state.error) setDismissedError("");
    if (!state.notice) setDismissedNotice(null);
  }, [state.error, state.notice]);

  useEffect(() => {
    if (!access || access.account.must_change) return;
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let received = false;
    subscribe(
      token,
      (s) => {
        if (!received) {
          received = true;
          plannedRestart.current = 0;
        }
        setState(s);
        setLive(true);
      },
      abort.signal,
    ).catch((e) => {
      if (!abort.signal.aborted) {
        if (e instanceof ApiError && e.status === 401) {
          setPlaying(null);
          setAccess(null);
          setState(emptySnapshot);
          setLive(false);
          return;
        }
        setLive(false);
        if (Date.now() >= plannedRestart.current) setError(e.message);
        timer = setTimeout(() => setRetry((n) => n + 1), 4000);
      }
    });
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [access, token, retry]);

  async function send(command: object) {
    setDismissedError("");
    setDismissedNotice(null);
    setBusy(true);
    setError("");
    try {
      if (
        ["download_batch", "download"].includes(
          (command as { type: string }).type,
        )
      ) {
        await api(token, "downloads/batch?confirm=true", {
          files:
            (command as { type: string }).type === "download"
              ? [
                  {
                    username: (command as { username: string }).username,
                    path: (command as { path: string }).path,
                    size: (command as { size: number }).size,
                  },
                ]
              : (command as { files: unknown }).files,
        });
      } else if ((command as { type: string }).type === "retry_downloads") {
        const retry = command as { username: string | null; paths?: string[] };
        await api(token, "downloads/retry", {
          username: retry.username,
          paths: retry.paths ?? [],
        });
      } else {
        await api(token, "commands", command);
      }
      return true;
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        void lock(false);
        return false;
      }
      setError((e as Error).message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  function openUser(username: string) {
    setSelectedUser(username);
    void send({ type: "lookup_user", username });
  }

  async function lock(revoke = true) {
    if (access && revoke) {
      try {
        await api(token, "auth/logout", {});
      } catch (e) {
        setError(`Could not sign out: ${(e as Error).message}. Please retry.`);
        return;
      }
    }

    setSelectedUser(null);
    setBrowseUser("");
    setMessageUser("");
    setPlaying(null);
    setAccess(null);
    setState(emptySnapshot);
    setLive(false);
    setError("");
  }
  if (restoring)
    return (
      <div className="login-screen">
        <p role="status">Restoring your session…</p>
      </div>
    );
  if (!access) return <LoginScreen onLogin={acceptSession} />;
  if (access.account.must_change)
    return (
      <div className="login-screen">
        <div style={{ width: "min(600px,100%)" }}>
          <AccountForm
            access={access}
            onChange={acceptSession}
            onCancel={() => void lock()}
          />
        </div>
      </div>
    );
  const online = state.connection === "connected";
  const activeDownloads = state.downloads.filter((t) =>
    ["queued", "downloading"].includes(t.status),
  ).length;
  return (
    <SelectionProvider>
      <UserContext.Provider value={{ state, open: openUser }}>
        <div className="app-shell">
          <aside className="sidebar">
            <a
              className="brand"
              href="#"
              onClick={(e) => {
                e.preventDefault();
                setPage("search");
              }}
            >
              <img
                className="brand-icon"
                src="/assets/icons/rustysoul.svg"
                width="40"
                height="40"
                alt=""
              />
              rusty<span>soul</span>
            </a>
            <div className="workspace">
              <span className="workspace-avatar">RS</span>
              <div>
                My server<small>Personal workspace</small>
              </div>
              <ShieldCheck size={16} />
            </div>
            <span className="nav-label">LIBRARY & DISCOVERY</span>
            <nav aria-label="Main navigation">
              {nav.map(({ id, label, icon: Icon }) => (
                <button
                  key={id}
                  aria-label={label}
                  aria-describedby={
                    id === "rooms" || id === "messages"
                      ? `${id}-unread-total`
                      : undefined
                  }
                  className={`nav-item ${page === id ? "selected" : ""}`}
                  onClick={() => setPage(id)}
                  aria-current={page === id ? "page" : undefined}
                >
                  <Icon size={18} />
                  {label}
                  {(id === "rooms" || id === "messages") &&
                    unreadTotal(
                      id === "rooms" ? state.room_unread : state.private_unread,
                    ) > 0 && (
                      <span
                        className="count unread-count"
                        id={`${id}-unread-total`}
                        aria-label={`${unreadTotal(id === "rooms" ? state.room_unread : state.private_unread)} unread ${id === "rooms" ? "room" : "private"} messages`}
                      >
                        {unreadTotal(
                          id === "rooms"
                            ? state.room_unread
                            : state.private_unread,
                        )}
                      </span>
                    )}
                  {id === "downloads" && activeDownloads > 0 && (
                    <span className="count">{activeDownloads}</span>
                  )}
                  {page === id && <span className="active-dot" />}
                </button>
              ))}
            </nav>
            <div className="sidebar-bottom">
              <button
                className={`nav-item ${page === "settings" ? "selected" : ""}`}
                onClick={() => setPage("settings")}
              >
                <Settings size={18} />
                Settings & status
              </button>
              <button
                className="nav-item"
                onClick={() => {
                  void lock();
                }}
              >
                <LogOut size={18} />
                Sign out
              </button>
              <div className="server-card">
                <span className={`dot ${online ? "online" : ""}`} />
                <div>
                  {online
                    ? "Connected to Soulseek"
                    : ["connecting", "verifying"].includes(state.connection)
                      ? "Connecting…"
                      : "Soulseek disconnected"}
                  <small>
                    {live
                      ? "Daemon connected · live updates"
                      : "Reconnecting to daemon…"}
                  </small>
                </div>
              </div>
              <span className="version">
                RustySoul {state.version}
                <span>Made for your server.</span>
              </span>
            </div>
          </aside>
          <div className="main-shell">
            <header className="topbar">
              <div className="breadcrumb">
                Workspace <ChevronRight size={14} />
                <span>
                  {page === "settings"
                    ? "Settings & status"
                    : nav.find((n) => n.id === page)?.label}
                </span>
              </div>
              <div className="topbar-right">
                <PrivilegeBadge
                  state={state}
                  open={() => {
                    setPage("settings");
                    setSettingsTab("accounts");
                  }}
                />
                <button
                  className="icon-button"
                  aria-label="Sign out of workspace"
                  title="Sign out of workspace"
                  onClick={() => {
                    void lock();
                  }}
                >
                  <LockKeyhole size={16} />
                </button>
                <NetworkStatus
                  state={state}
                  busy={busy}
                  send={send}
                  openAccounts={() => {
                    setPage("settings");
                    setSettingsTab("accounts");
                  }}
                />
                <button
                  className="avatar"
                  title="Settings"
                  aria-label="Open settings"
                  onClick={() => setPage("settings")}
                >
                  {state.username.slice(0, 2).toUpperCase() || "RS"}
                </button>
              </div>
            </header>
            {playing && (
              <AudioPlayer file={playing} close={() => setPlaying(null)} />
            )}
            <main>
              <ReleaseNotice
                token={token}
                enabled={state.network.check_for_updates}
              />
              {(error || (state.error && state.error !== dismissedError)) && (
                <SystemMessage
                  error
                  onDismiss={() => {
                    setError("");
                    setDismissedError(state.error || "");
                    void api(token, "commands", {
                      type: "dismiss_messages",
                    }).catch(() => {});
                  }}
                >
                  {!error && state.error && (
                    <strong>Background daemon error: </strong>
                  )}
                  {error || state.error}
                  <button
                    className="text-link"
                    onClick={() => {
                      setPage("settings");
                      setSettingsTab("logs");
                    }}
                  >
                    View logs & activity
                  </button>
                </SystemMessage>
              )}
              {state.notice && state.notice !== dismissedNotice && (
                <SystemMessage
                  onDismiss={() => {
                    setDismissedNotice(state.notice);
                    void api(token, "commands", {
                      type: "dismiss_messages",
                    }).catch(() => {});
                  }}
                >
                  {state.notice}
                </SystemMessage>
              )}
              {preferences.libraryBanner &&
                state.library.scanning &&
                state.library.trigger !== "scheduled" && (
                  <LibraryProgress status={state.library} banner />
                )}
              {!online && dismissedConnection !== state.connection && (
                <div className="offline-banner">
                  <button
                    className="icon-button"
                    aria-label="Dismiss connection hint"
                    onClick={() => setDismissedConnection(state.connection)}
                  >
                    <X size={15} />
                  </button>
                  <span>
                    <span className="dot" />
                    {["connecting", "verifying"].includes(state.connection)
                      ? "Connecting to the Soulseek network…"
                      : "Your daemon is ready. Connect to start discovering."}
                  </span>
                  <button
                    className="button small"
                    disabled={
                      busy ||
                      ["connecting", "verifying"].includes(state.connection) ||
                      !live
                    }
                    onClick={() =>
                      state.credentials_configured
                        ? send({ type: "connect" })
                        : setPage("settings")
                    }
                  >
                    {["connecting", "verifying"].includes(state.connection)
                      ? "Connecting…"
                      : state.credentials_configured
                        ? "Connect to Soulseek"
                        : "Set up Soulseek"}
                    <ChevronRight size={14} />
                  </button>
                </div>
              )}
              {page === "search" && (
                <SearchPage
                  state={state}
                  token={token}
                  scopePreset={scopePreset}
                  send={send}
                  online={online}
                  busy={busy}
                  onSignin={() => {
                    setPage("settings");
                    setSettingsTab("accounts");
                    requestAnimationFrame(() =>
                      document
                        .getElementById("soulseek-account")
                        ?.scrollIntoView(),
                    );
                  }}
                />
              )}
              {page === "discover" && (
                <Discover
                  state={state}
                  send={send}
                  search={async (query) => {
                    if (await send({ type: "search", query })) {
                      setScopePreset(null);
                      setPage("search");
                    }
                  }}
                  profile={() => {
                    setPage("settings");
                    setSettingsTab("profile");
                  }}
                  signin={() => {
                    setPage("settings");
                    setSettingsTab("accounts");
                  }}
                />
              )}
              {(page === "downloads" || page === "uploads") && (
                <TransfersPage
                  key={page}
                  play={setPlaying}
                  openStorage={() => {
                    setPage("settings");
                    setSettingsTab("library");
                  }}
                  token={token}
                  total={
                    page === "uploads"
                      ? state.uploads_total
                      : state.downloads_total
                  }
                  revision={state.transfer_revision}
                  traffic={state.traffic}
                  uploads={page === "uploads"}
                  items={page === "uploads" ? state.uploads : state.downloads}
                  send={send}
                  online={online}
                />
              )}
              {page === "browse" && (
                <ShareBrowser
                  state={state}
                  token={token}
                  send={send}
                  selected={browseUser}
                  onSelect={setBrowseUser}
                  openUser={openUser}
                />
              )}
              {(page === "rooms" || page === "messages") && (
                <ChatPage
                  state={state}
                  send={send}
                  online={online}
                  privateChat={page === "messages"}
                  token={token}
                  readingEnabled={!selectedUser}
                  initialTarget={page === "messages" ? messageUser : roomTarget}
                  changeTarget={
                    page === "messages" ? setMessageUser : setRoomTarget
                  }
                  drafts={chatDrafts}
                  changeDrafts={setChatDrafts}
                  searchRoom={(room) => {
                    setScopePreset({ type: "room", target: room });
                    setPage("search");
                  }}
                />
              )}
              {page === "settings" && (
                <>
                  <PageHeader
                    eyebrow="YOUR SERVER"
                    title="Settings & status"
                    description="A quiet home for your Soulseek connection."
                  />
                  <SettingsNavigation
                    selected={settingsTab}
                    onChange={setSettingsTab}
                  />
                  <div
                    className="settings-grid"
                    role="tabpanel"
                    id="settings-content"
                    aria-labelledby={"settings-tab-" + settingsTab}
                  >
                    {settingsTab === "accounts" && (
                      <>
                        <Credentials token={token} state={state} />
                        <AccountForm access={access} onChange={acceptSession} />
                      </>
                    )}
                    {settingsTab === "logs" && <Logs token={token} />}
                    {settingsTab === "backups" && (
                      <Backups
                        token={token}
                        onRestored={() => setAccess(null)}
                      />
                    )}
                    {settingsTab === "maintenance" && (
                      <Maintenance
                        token={token}
                        onRestarting={(value) => {
                          plannedRestart.current = value
                            ? Date.now() + 1800000
                            : 0;
                          if (value) setError("");
                        }}
                      />
                    )}
                    {settingsTab === "api" && <ApiAccess token={token} />}
                    {settingsTab === "profile" && (
                      <Profile
                        state={state}
                        token={token}
                        openUser={openUser}
                      />
                    )}
                    {settingsTab === "library" && (
                      <Locations state={state} token={token} />
                    )}
                    {settingsTab === "people" && (
                      <>
                        <SharingSettingsForm state={state} token={token} />
                        <PeopleSettings state={state} send={send} />
                      </>
                    )}
                    {settingsTab === "appearance" && (
                      <Appearance
                        preferences={preferences}
                        onChange={setPreferences}
                      />
                    )}
                    {settingsTab === "network" && (
                      <>
                        <ConnectionSettings state={state} token={token} />
                        <RetrySettings state={state} token={token} />
                        <section className="panel">
                          <h2>Network</h2>
                          <dl>
                            <dt>Status</dt>
                            <dd>{state.connection}</dd>
                            <dt>Username</dt>
                            <dd>
                              {state.username ? (
                                <User username={state.username} />
                              ) : (
                                "Not configured"
                              )}
                            </dd>
                            <dt>Presence</dt>
                            <dd>
                              {state.username ? (
                                <UserStatus username={state.username} />
                              ) : (
                                "Not configured"
                              )}
                            </dd>
                          </dl>
                          <div className="button-row">
                            <button
                              className="button"
                              disabled={!online}
                              onClick={() =>
                                send({ type: "set_away", away: !state.away })
                              }
                            >
                              {state.away ? "Set online" : "Set away"}
                            </button>
                            <button
                              className="button secondary"
                              onClick={() =>
                                send({
                                  type: online ? "disconnect" : "connect",
                                })
                              }
                            >
                              {online ? "Disconnect" : "Connect"}
                            </button>
                          </div>
                        </section>
                      </>
                    )}
                    {settingsTab === "status" && (
                      <>
                        <ReleaseSettings state={state} token={token} />
                        <SystemStatus
                          state={state}
                          live={live}
                          openLogs={() => setSettingsTab("logs")}
                          openNetwork={() => setSettingsTab("network")}
                          openLibrary={() => setSettingsTab("library")}
                        />
                        <section className="panel wide">
                          <h2>
                            <ShieldCheck size={18} />
                            Access & privacy
                          </h2>
                          <p>
                            Your sign-in is remembered in a protected browser
                            cookie. Sign out to clear it, or change your
                            password to revoke all sessions. Manage your
                            Soulseek account above.
                          </p>
                          <p className="hint">
                            Soulseek supports Online and Away while connected.
                            Away keeps searches and transfers working; the
                            protocol has no supported invisible mode. Set Away
                            from the top-right status menu or configure
                            automatic Away in Profile.
                          </p>
                          <p className="hint">
                            Soulseek traffic is not end-to-end encrypted. Use
                            HTTPS for remote browser access.
                          </p>
                        </section>
                      </>
                    )}
                  </div>
                </>
              )}
            </main>
            <footer>
              <span>
                <span className={`dot ${live ? "online" : ""}`} />
                {live
                  ? "Live daemon connection"
                  : "Daemon connection interrupted"}
              </span>
              <span>
                <ArrowDown size={13} />
                {bytes(state.downloads.reduce((s, t) => s + t.speed, 0))}/s
                <span className="footer-divider" />
                <ArrowUp size={13} />
                {bytes(state.uploads.reduce((s, t) => s + t.speed, 0))}/s
              </span>
            </footer>
            {selectedUser && (
              <UserDialog
                discover={(item, dislike = false) => {
                  setSelectedUser(null);
                  setPage("discover");
                  void send({
                    type: "discover",
                    query: { mode: dislike ? "dislikes" : "topic", item },
                  });
                }}
                key={selectedUser}
                username={selectedUser}
                state={state}
                close={() => setSelectedUser(null)}
                send={send}
                search={() => {
                  setScopePreset({ type: "user", target: selectedUser });
                  setPage("search");
                  setSelectedUser(null);
                }}
                browse={() => {
                  setBrowseUser(selectedUser);
                  setPage("browse");
                  void send({ type: "browse", username: selectedUser });
                  setSelectedUser(null);
                }}
                message={() => {
                  setMessageUser(selectedUser);
                  setPage("messages");
                  setSelectedUser(null);
                }}
              />
            )}
          </div>
        </div>
      </UserContext.Provider>
    </SelectionProvider>
  );
}

function PageHeader({
  eyebrow,
  title,
  description,
  action,
}: {
  eyebrow: string;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="page-header">
      <div>
        <span className="eyebrow">{eyebrow}</span>
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      {action}
    </div>
  );
}
function Empty({
  icon,
  title,
  detail,
}: {
  icon: ReactNode;
  title: string;
  detail: string;
}) {
  return (
    <div className="empty">
      <div className="empty-icon">{icon}</div>
      <h2>{title}</h2>
      <p>{detail}</p>
    </div>
  );
}

function SearchPage({
  state,
  token,
  scopePreset,
  send,
  online,
  busy,
  onSignin,
}: {
  state: Snapshot;
  token: string;
  scopePreset: { type: "user" | "room"; target: string } | null;
  send: (c: object) => void;
  online: boolean;
  busy: boolean;
  onSignin: () => void;
}) {
  const [scope, setScope] = useState<"global" | "user" | "room">("global");
  const [target, setTarget] = useState("");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState("");
  const [filter, setFilter] = useState("");
  const [format, setFormat] = useState<AudioFormat>("all");
  const [criteria, setCriteria] = useState<FileCriteria>({});
  const [repeatQuery, setRepeatQuery] = useState("");
  useEffect(() => {
    if (scopePreset) {
      setScope(scopePreset.type);
      setTarget(scopePreset.target);
    }
  }, [scopePreset]);
  const searches = state.searches.filter((s) => !s.wishlist_id || s.pinned);
  const active = searches.find((s) => s.id === selected) || searches.at(-1);
  useEffect(
    () => setRepeatQuery(active?.query || ""),
    [active?.id, active?.query],
  );
  return (
    <>
      <PageHeader
        eyebrow="DISCOVER SOMETHING GOOD"
        title="Find your next favourite."
        description="Explore the collections of people who love music as much as you do."
      />
      <div className="search-scope">
        <label>
          Search in
          <select
            aria-label="Search scope"
            value={scope}
            onChange={(e) => setScope(e.target.value as typeof scope)}
          >
            <option value="global">Entire network</option>
            <option value="user">A user's shares</option>
            <option value="room">A room's members</option>
          </select>
        </label>
        {scope !== "global" && (
          <label>
            {scope === "user" ? "Username" : "Room name"}
            <input
              aria-label={
                scope === "user" ? "Search username" : "Search room name"
              }
              value={target}
              maxLength={scope === "user" ? 30 : 256}
              onChange={(e) => setTarget(e.target.value)}
            />
          </label>
        )}
      </div>
      <form
        className="search-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (!online) return;
          send(
            scope === "global"
              ? { type: "search", query: query.trim() }
              : {
                  type: "search_scoped",
                  query: query.trim(),
                  scope: {
                    type: scope,
                    [scope === "user" ? "username" : "room"]: target.trim(),
                  },
                },
          );
          setSelected("");
        }}
      >
        <SearchIcon size={22} />
        <input
          aria-label="Search the Soulseek network"
          disabled={!online}
          placeholder="Artist, album, track… follow your curiosity"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          maxLength={256}
        />
        <kbd>↵</kbd>
        <button
          className="button"
          disabled={
            !online ||
            busy ||
            !query.trim() ||
            (scope !== "global" && !target.trim())
          }
        >
          Search network
          <ChevronRight size={16} />
        </button>
      </form>
      {!online && (
        <p className="network-signin-prompt">
          Search is unavailable while disconnected.{" "}
          <button className="text-link" onClick={onSignin}>
            Sign in to the Soulseek network
          </button>
        </p>
      )}
      <div className="format-filter">
        <label>
          Search format{" "}
          <select
            aria-label="Search format"
            value={format}
            onChange={(e) => setFormat(e.target.value as AudioFormat)}
          >
            <option value="all">All formats</option>
            <option value="lossless">Lossless</option>
            <option value="lossy">Lossy</option>
          </select>
        </label>
        <small>
          Format filters use known file extensions; ambiguous containers remain
          under All.
        </small>
      </div>
      <Wishlist state={state} send={send} open={setSelected} />
      <AdvancedSearch value={criteria} change={setCriteria} />
      <div className="search-hints">
        <span>
          <ShieldCheck size={13} />
          Direct from the community
        </span>
        <span>Try an artist + album for better results</span>
      </div>
      {searches.length > 0 ? (
        <section className="results-panel">
          <SearchTabs
            searches={searches}
            activeId={active?.id}
            select={(id) => {
              setSelected(id);
              setFilter("");
            }}
            close={(id) => {
              send({ type: "forget_search", id });
            }}
          />
          {active && (
            <form
              className="search-repeat"
              onSubmit={(e) => {
                e.preventDefault();
                send({
                  type: "restart_search",
                  id: active.id,
                  query: repeatQuery.trim(),
                });
              }}
            >
              <label>
                Search terms
                <input
                  aria-label="Repeat search terms"
                  value={repeatQuery}
                  maxLength={256}
                  onChange={(e) => setRepeatQuery(e.target.value)}
                  required
                />
              </label>
              <button
                className="button small secondary"
                disabled={!online || busy || !repeatQuery.trim()}
              >
                <RefreshCw size={15} />
                Search again
              </button>
              <small>
                Uses the same search scope and replaces the previous results.
              </small>
            </form>
          )}
          <div className="results-toolbar">
            <div>
              <strong>
                {(
                  active?.total ??
                  active?.results.length ??
                  0
                ).toLocaleString()}{" "}
                files
              </strong>
              <span>
                {active?.collecting
                  ? "Listening for results (up to 2 minutes)…"
                  : "Search window complete · all collected files remain browsable"}
              </span>
            </div>
            <input
              aria-label="Filter results"
              className="filter-input"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter these results…"
            />
          </div>
          {active && (
            <SearchResults
              key={active.id}
              search={active}
              token={token}
              filter={filter}
              format={format}
              criteria={criteria}
              send={send}
              online={online}
              capacity={Math.max(
                0,
                state.transfer_capacity -
                  (state.download_capacity_used ?? state.downloads.length),
              )}
            />
          )}
        </section>
      ) : (
        <>
          <div className="discovery-card">
            <div className="discovery-art">
              <div className="vinyl">
                <div />
              </div>
              <div className="record-sleeve">
                <span>
                  THE ART OF
                  <br />
                  FINDING
                  <br />
                  <b>
                    SOMETHING
                    <br />
                    NEW.
                  </b>
                </span>
                <AudioLines size={32} />
              </div>
              <span className="art-dot" />
            </div>
            <div className="discovery-copy">
              <span className="eyebrow">GO BEYOND THE ALGORITHM</span>
              <h2>
                Great music is
                <br />a shared experience.
              </h2>
              <p>
                Rare finds, familiar favourites, and the album you didn’t know
                you needed. It all starts with a search.
              </p>
              <div className="card-foot">
                <span className="mini-avatars">
                  <i>A</i>
                  <i>M</i>
                  <i>J</i>
                </span>
                <span>
                  Shared by people.
                  <br />
                  <strong>Discovered by you.</strong>
                </span>
              </div>
            </div>
          </div>
          <div className="feature-strip">
            <div>
              <SearchIcon size={19} />
              <span>
                <strong>Search freely</strong>
                <small>Find a track. Discover a collection.</small>
              </span>
            </div>
            <div>
              <FolderOpen size={19} />
              <span>
                <strong>Explore a little deeper</strong>
                <small>Browse another listener’s library.</small>
              </span>
            </div>
            <div>
              <HardDrive size={19} />
              <span>
                <strong>Make yourself at home</strong>
                <small>Your daemon works in the background.</small>
              </span>
            </div>
          </div>
        </>
      )}
    </>
  );
}

function TransfersPage({
  play,
  openStorage,
  uploads,
  token,
  total,
  revision,
  traffic,
  items,
  send,
  online,
}: {
  play: (file: Transfer) => void;
  uploads: boolean;
  token: string;
  total?: number;
  revision?: number;
  traffic?: Snapshot["traffic"];
  items: Transfer[];
  send: (c: object) => void;
  online: boolean;
  openStorage: () => void;
}) {
  const [groupUsers, setGroupUsers] = useState(uploads);
  const [userOrder, setUserOrder] = useChoice<string>(
    uploads ? "uploads.userOrder" : "downloads.userOrder",
    "",
    ["", "most", "least", "oldest", "newest", "recent", "name"],
  );
  const [cancelling, setCancelling] = useState(false);
  const [selectionError, setSelectionError] = useState("");
  const direction = uploads ? "uploads" : "downloads";
  const [filter, setFilter] = useState("all");
  const [order, setOrder] = useState<TransferSort>("activity");
  const [reverse, setReverse] = useState(false);
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(100);
  const [clearStatus, setClearStatus] = useState("all");
  useEffect(
    () => setLimit(100),
    [filter, query, order, reverse, userOrder, groupUsers],
  );
  const localClearable = items.filter(
    (t) =>
      [
        "completed",
        "failed",
        "cancelled",
        "interrupted",
        "timed_out",
        "blocked",
      ].includes(t.status) &&
      (clearStatus === "all" || t.status === clearStatus),
  ).length;
  const matching = sortTransfers(
    items.filter((t) => filter === "all" || t.status === filter),
    order,
    reverse,
    query,
  );
  const [defaultCollapsed, setDefaultCollapsed] = useState(uploads);
  const [collapseExceptions, setCollapseExceptions] = useState<Set<string>>(
    new Set(),
  );
  const isCollapsed = (user: string) =>
    defaultCollapsed !== collapseExceptions.has(user);
  const paged = total !== undefined;
  const feedQuery = new URLSearchParams({
    group_users: String(groupUsers),
    user_sort: groupUsers ? userOrder : "",
    status: filter,
    filter: query,
    order,
    reverse: String(reverse),
  }).toString();
  const { data, error, loading, load, retry } = useTransferFeed(
    token,
    uploads ? "uploads" : "downloads",
    feedQuery,
    paged,
    revision,
    total,
  );
  // The preview-only fallback follows the same whole-user boundary rule.
  const localUsers = new Set(matching.slice(0, limit).map((t) => t.username));
  const shown = paged
    ? (data?.transfers ?? [])
    : groupUsers
      ? matching.filter((t) => localUsers.has(t.username))
      : matching.slice(0, limit);
  const clearable = paged
    ? (data?.summary.clearable[clearStatus] ?? 0)
    : localClearable;
  const groups = groupUsers
    ? Array.from(
        shown
          .reduce((m, t) => {
            const rows = m.get(t.username) ?? [];
            rows.push(t);
            m.set(t.username, rows);
            return m;
          }, new Map<string, Transfer[]>())
          .entries(),
      )
    : [["", shown] as [string, Transfer[]]];
  const visibleOrder = groups
    .filter(([u]) => !groupUsers || !isCollapsed(u))
    .flatMap(([, rows]) => rows)
    .filter((t) => canSelectTransfer(t, uploads));
  const selection = useTransferSelection(
    shown,
    visibleOrder,
    feedQuery,
    uploads,
  );
  const { selected, setSelected } = selection;
  const selectedRetryCount = (user?: string) =>
    [...selected.values()].filter(
      (t) => canRetryDownload(t) && (!user || t.username === user),
    ).length;
  const selectedCancelCount = (user?: string) =>
    [...selected.values()].filter(
      (t) => canCancelTransfer(t, uploads) && (!user || t.username === user),
    ).length;
  async function submitCancellation(command: object, retry = false) {
    const deadline = Date.now() + 30000;
    for (;;) {
      try {
        const action = command as { username: string | null; paths: string[] };
        await api(
          token,
          retry ? "downloads/retry" : "commands",
          retry ? { username: action.username, paths: action.paths } : command,
        );
        return true;
      } catch (e) {
        if (
          e instanceof ApiError &&
          e.status === 429 &&
          Date.now() < deadline
        ) {
          await new Promise((resolve) => setTimeout(resolve, 2100));
          continue;
        }
        throw e;
      }
    }
  }
  async function cancelSelected(user?: string) {
    setCancelling(true);
    setSelectionError("");
    try {
      const entries = [...selected.values()].filter(
        (t) => canCancelTransfer(t, uploads) && (!user || t.username === user),
      );
      for (const username of new Set(entries.map((t) => t.username))) {
        for (const batch of cancelTransferBatches(
          uploads,
          username,
          entries.filter((t) => t.username === username),
        )) {
          if (!(await submitCancellation(batch))) return;
          const keys = new Set(
            batch.paths.map((path) => transferKey({ username, path })),
          );
          setSelected(
            (old) => new Map([...old].filter(([key]) => !keys.has(key))),
          );
        }
      }
    } catch (e) {
      setSelectionError((e as Error).message);
    } finally {
      setCancelling(false);
    }
  }
  async function retrySelected(user?: string, allForUser = false) {
    setCancelling(true);
    setSelectionError("");
    try {
      const entries = [...selected.values()].filter(
        (t) => canRetryDownload(t) && (!user || t.username === user),
      );
      if (allForUser) {
        await submitCancellation({ username: user ?? null, paths: [] }, true);
        const keys = new Set(entries.map(transferKey));
        setSelected(
          (old) => new Map([...old].filter(([key]) => !keys.has(key))),
        );
        return;
      }
      for (const username of new Set(entries.map((t) => t.username))) {
        for (const batch of retryDownloadBatches(
          username,
          entries.filter((t) => t.username === username),
        )) {
          await submitCancellation(batch, true);
          const keys = new Set(
            batch.paths.map((path) => transferKey({ username, path })),
          );
          setSelected(
            (old) => new Map([...old].filter(([key]) => !keys.has(key))),
          );
        }
      }
    } catch (e) {
      setSelectionError((e as Error).message);
    } finally {
      setCancelling(false);
    }
  }
  async function cancelUser(username: string) {
    setCancelling(true);
    setSelectionError("");
    try {
      if (
        await submitCancellation({
          type: uploads ? "cancel_uploads" : "cancel_downloads",
          username,
          paths: [],
        })
      )
        selection.clear(username);
    } catch (e) {
      setSelectionError((e as Error).message);
    } finally {
      setCancelling(false);
    }
  }
  const interactive = (target: EventTarget | null) =>
    target instanceof Element &&
    !!target.closest(
      "button,a,input,select,textarea,summary,.transfer-queue-controls,[role=button]",
    );
  const matchingTotal = paged ? (data?.total ?? 0) : matching.length;
  return (
    <>
      <PageHeader
        eyebrow={uploads ? "GIVE SOMETHING BACK" : "YOUR GROWING COLLECTION"}
        title={uploads ? "Share the good stuff." : "On its way to you."}
        description={
          uploads
            ? "Your shared library, finding a new home with other listeners."
            : "Follow your downloads. The daemon keeps working when you close this tab."
        }
      />
      <div className="stat-grid transfer-stats">
        <Stat
          label="Total transfers"
          value={(
            data?.summary.total ??
            total ??
            items.length
          ).toLocaleString()}
          icon={<FolderOpen size={18} />}
        />
        <Stat
          label="Transferring"
          value={(
            data?.summary.active ??
            items.filter((t) => ["downloading", "uploading"].includes(t.status))
              .length
          ).toString()}
          icon={<RefreshCw size={18} />}
        />
        <Stat
          label="In queue"
          value={
            paged
              ? (data?.summary.queued?.toLocaleString() ?? "…")
              : items.filter(isQueuedTransfer).length.toLocaleString()
          }
          detail="Waiting transfers, including paused items, retries and storage or share checks."
          icon={<ListOrdered size={18} />}
        />
        <Stat
          label="Current speed (5s average)"
          value={
            <TransferSpeed
              speed={
                paged
                  ? data?.summary.speed
                  : items
                      .filter((t) =>
                        ["downloading", "uploading"].includes(t.status),
                      )
                      .reduce((n, t) => n + t.speed, 0)
              }
            />
          }
          icon={uploads ? <ArrowUp size={18} /> : <ArrowDown size={18} />}
        />
      </div>
      {traffic && <Traffic token={token} totals={traffic} />}
      <section className="results-panel">
        <div className="transfer-cleanup">
          <label>
            Clear history
            <select
              aria-label="Clear transfer history status"
              value={clearStatus}
              onChange={(e) => setClearStatus(e.target.value)}
            >
              {[
                "all",
                "blocked",
                "completed",
                "failed",
                "cancelled",
                "interrupted",
                "timed_out",
              ].map((s) => (
                <option key={s} value={s}>
                  {s === "all"
                    ? "All finished records"
                    : s.replaceAll("_", " ")}
                </option>
              ))}
            </select>
          </label>
          <button
            className="button small secondary"
            disabled={!clearable}
            onClick={() =>
              send({
                type: "clear_transfers",
                direction: uploads ? "uploads" : "downloads",
                status: clearStatus,
              })
            }
          >
            Clear {clearable} history records
          </button>
          <small>
            Active, queued and paused transfers are kept. Downloaded files stay
            on disk.
          </small>
        </div>
        <div className="results-toolbar">
          <strong>{uploads ? "Upload activity" : "Download queue"}</strong>
          {
            <button
              className="button small secondary"
              disabled={
                cancelling ||
                (!uploads && !online) ||
                ((data?.summary.clearable.failed || 0) +
                  (data?.summary.clearable.timed_out || 0) +
                  (data?.summary.clearable.interrupted || 0) ===
                  0 &&
                  !items.some((t) =>
                    ["failed", "timed_out", "interrupted"].includes(t.status),
                  ))
              }
              onClick={() =>
                uploads
                  ? send({ type: "retry_uploads", username: null })
                  : void retrySelected(undefined, true)
              }
            >
              Retry failed {uploads ? "uploads" : "downloads"}
            </button>
          }
          <input
            aria-label="Filter transfers"
            placeholder="Find a file or user…"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setLimit(100);
            }}
          />
          <label>
            Order transfers
            <select
              aria-label="Order transfers"
              value={order}
              onChange={(e) => {
                setOrder(e.target.value as TransferSort);
                setLimit(100);
              }}
            >
              {transferSorts.map(([value, label]) => (
                <option value={value} key={value}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <button
            className="button small secondary"
            aria-label="Reverse transfer order"
            aria-pressed={reverse}
            onClick={() => setReverse((r) => !r)}
          >
            {reverse ? "Reversed" : "Default direction"}
          </button>
          <select
            aria-label="Filter transfer status"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          >
            {[
              "all",
              "waiting",
              "waiting_for_space",
              "retry_waiting",
              "checking_shares",
              "blocked",
              "queued",
              "downloading",
              "uploading",
              "paused",
              "paused_waiting",
              "interrupted",
              "completed",
              "failed",
              "cancelled",
              "timed_out",
            ].map((f) => (
              <option key={f} value={f}>
                {f === "all" ? "All statuses" : f.replaceAll("_", " ")}
              </option>
            ))}
          </select>
        </div>
        <div className="results-toolbar">
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={groupUsers}
              onChange={(e) => setGroupUsers(e.target.checked)}
            />
            Group by user
          </label>
          {groupUsers && (
            <label>
              Order users{" "}
              <select
                aria-label="Order transfer users"
                value={userOrder}
                onChange={(e) => setUserOrder(e.target.value)}
              >
                {[
                  ["", "Follow file order"],
                  ["most", "Most data transferred"],
                  ["least", "Least data transferred"],
                  ["newest", "Newest first transfer"],
                  ["oldest", "Oldest first transfer"],
                  ["recent", "Most recent activity"],
                  ["name", "Username"],
                ].map(([v, l]) => (
                  <option key={v} value={v}>
                    {l}
                  </option>
                ))}
              </select>
            </label>
          )}
          {data?.users_since && (
            <small>
              Per-user payload totals recorded since{" "}
              {new Date(data.users_since * 1000).toLocaleString()}; kept when
              history is cleared.
            </small>
          )}
        </div>
        {error && (
          <div role="alert">
            {error}{" "}
            <button className="button small secondary" onClick={retry}>
              Retry loading transfers
            </button>
          </div>
        )}
        {selectionError && (
          <SystemMessage error onDismiss={() => setSelectionError("")}>
            {selectionError}
          </SystemMessage>
        )}
        {cancelling && <p role="status">Submitting transfer requests…</p>}
        <div className="results-selection upload-selection">
          <button
            className="button small secondary"
            disabled={cancelling}
            onClick={() => selection.add(shown)}
          >
            Select loaded {uploads ? "active uploads" : "downloads"}
          </button>
          <button
            className="button small secondary"
            disabled={cancelling || !selected.size}
            onClick={() => selection.clear()}
          >
            Clear {uploads ? "upload" : "download"} selection
          </button>
          <span>
            {selected.size} {direction} selected ·{" "}
            {bytes([...selected.values()].reduce((n, t) => n + t.size, 0))}
          </span>
          {!uploads && (
            <button
              className="button small secondary"
              disabled={cancelling || !online || !selectedRetryCount()}
              onClick={() => void retrySelected()}
              title="Queue selected failed, timed-out or interrupted downloads for another attempt."
            >
              Retry selected downloads ({selectedRetryCount()})
            </button>
          )}
          <button
            className="button small secondary"
            disabled={cancelling || !selectedCancelCount()}
            onClick={() => void cancelSelected()}
          >
            Cancel selected {direction}
          </button>
          <small>
            Click a file to select it. Ctrl/Cmd-click toggles files; Shift-click
            selects the range in the current order. Checkboxes add or remove
            individual files.
          </small>
        </div>
        {groupUsers && shown.length > 0 && (
          <div className="button-row transfer-group-tools">
            <button
              className="button small secondary"
              onClick={() => {
                setDefaultCollapsed(true);
                setCollapseExceptions(new Set());
              }}
            >
              Collapse all users
            </button>
            <button
              className="button small secondary"
              onClick={() => {
                setDefaultCollapsed(false);
                setCollapseExceptions(new Set());
              }}
            >
              Expand all users
            </button>
          </div>
        )}
        {shown.length ? (
          <div className="transfer-list">
            {groups.map(([username, rows]) => (
              <section
                className={groupUsers ? "upload-group" : "download-rows"}
                key={username}
              >
                {groupUsers && (
                  <div className="upload-group-heading">
                    <button
                      className="tree-toggle"
                      aria-label={`${isCollapsed(username) ? "Expand" : "Collapse"} ${uploads ? "uploads" : "downloads"} for ${username}`}
                      aria-expanded={!isCollapsed(username)}
                      onClick={() =>
                        setCollapseExceptions((old) => {
                          const next = new Set(old);
                          if (next.has(username)) next.delete(username);
                          else next.add(username);
                          return next;
                        })
                      }
                    >
                      <ChevronRight
                        size={17}
                        className={isCollapsed(username) ? "" : "expanded"}
                      />
                    </button>
                    <User username={username} />
                    <div className="button-row upload-bulk-controls">
                      {!uploads && (
                        <>
                          <button
                            className="button small secondary"
                            aria-label={`Select failed downloads for ${username}`}
                            disabled={
                              cancelling || !rows.some(canRetryDownload)
                            }
                            onClick={() =>
                              selection.add(rows.filter(canRetryDownload))
                            }
                          >
                            Select failed
                          </button>
                          <button
                            className="button small secondary"
                            aria-label={`Retry failed downloads for ${username}`}
                            disabled={cancelling || !online}
                            title="Retry all this user's failed, timed-out or interrupted downloads, including files hidden by filters."
                            onClick={() => void retrySelected(username, true)}
                          >
                            Retry failed
                          </button>
                          <button
                            className="button small secondary"
                            aria-label={`Retry selected downloads for ${username} (${selectedRetryCount(username)})`}
                            disabled={
                              cancelling ||
                              !online ||
                              !selectedRetryCount(username)
                            }
                            onClick={() => void retrySelected(username)}
                          >
                            Retry selected ({selectedRetryCount(username)})
                          </button>
                        </>
                      )}
                      {uploads && (
                        <button
                          className="button small secondary"
                          aria-label={`Retry failed uploads for ${username}`}
                          disabled={
                            !rows.some((t) =>
                              ["failed", "timed_out", "interrupted"].includes(
                                t.status,
                              ),
                            )
                          }
                          onClick={() =>
                            send({ type: "retry_uploads", username })
                          }
                        >
                          Retry failed
                        </button>
                      )}
                      <button
                        className="button small secondary"
                        disabled={
                          cancelling ||
                          !rows.some((t) => canSelectTransfer(t, uploads))
                        }
                        aria-label={`Select all ${direction} for ${username}`}
                        onClick={() => selection.add(rows)}
                      >
                        Select all
                      </button>
                      <button
                        className="button small secondary"
                        disabled={
                          cancelling ||
                          ![...selected.values()].some(
                            (t) => t.username === username,
                          )
                        }
                        aria-label={`Clear selection for ${username}`}
                        onClick={() => selection.clear(username)}
                      >
                        Clear selection
                      </button>
                      <button
                        className="button small secondary"
                        disabled={cancelling}
                        aria-label={`Cancel all ${direction} for ${username}`}
                        title="Cancel this user's unfinished transfers, including files hidden by filters. Completed files are kept."
                        onClick={() => void cancelUser(username)}
                      >
                        Cancel all
                      </button>
                      <button
                        className="button small secondary"
                        disabled={cancelling || !selectedCancelCount(username)}
                        aria-label={`Cancel selected ${direction} for ${username} (${selectedCancelCount(username)})`}
                        onClick={() => void cancelSelected(username)}
                      >
                        Cancel selected ({selectedCancelCount(username)})
                      </button>
                    </div>
                    {data?.users?.[username] && (
                      <small className="user-transfer-totals">
                        Uploaded {bytes(data.users[username].uploaded)} ·
                        Downloaded {bytes(data.users[username].downloaded)}
                        {data.users[username].first_at
                          ? ` · First ${new Date(data.users[username].first_at * 1000).toLocaleDateString()}`
                          : ""}
                      </small>
                    )}
                    <span>
                      {rows.length} {rows.length === 1 ? "file" : "files"} ·{" "}
                      {bytes(rows.reduce((n, t) => n + t.speed, 0))}
                      /s
                    </span>
                  </div>
                )}
                {(!groupUsers || !isCollapsed(username)) && (
                  <TransferRows
                    rows={rows}
                    render={(t) => (
                      <div
                        className={`transfer-row${selected.has(transferKey(t)) ? " selected" : ""}`}
                        key={transferKey(t)}
                        tabIndex={canSelectTransfer(t, uploads) ? 0 : undefined}
                        role="group"
                        aria-label={`${uploads ? "Upload" : "Download"} ${t.path} for ${t.username}`}
                        onMouseDown={(e) => {
                          if (e.shiftKey && !interactive(e.target))
                            e.preventDefault();
                        }}
                        onClick={(e) => {
                          if (
                            !cancelling &&
                            !interactive(e.target) &&
                            !window.getSelection()?.toString()
                          )
                            selection.select(t, e);
                        }}
                        onKeyDown={(e) => {
                          if (
                            e.target === e.currentTarget &&
                            e.key === " " &&
                            !cancelling
                          ) {
                            e.preventDefault();
                            selection.select(t, e, true);
                          }
                        }}
                      >
                        {canSelectTransfer(t, uploads) && (
                          <input
                            type="checkbox"
                            aria-label={`Select ${uploads ? "upload" : "download"} ${t.path} for ${t.username}`}
                            disabled={cancelling}
                            checked={selected.has(transferKey(t))}
                            onChange={() => {}}
                            onClick={(e) => selection.select(t, e, true)}
                          />
                        )}
                        <span className="file-icon">
                          <AudioLines size={19} />
                        </span>
                        <div className="transfer-detail">
                          <strong title={t.path}>{basename(t.path)}</strong>
                          {!uploads &&
                            t.status === "completed" &&
                            t.local_path && (
                              <small
                                className="download-destination"
                                title={t.local_path}
                              >
                                Saved to {t.local_path}
                              </small>
                            )}
                          <small>
                            <User username={t.username} /> · {bytes(t.size)}
                            {Number.isFinite(eta(t)) && t.status !== "completed"
                              ? ` · ~${Math.ceil(eta(t) / 60)} min remaining`
                              : ""}
                            {t.queue_position
                              ? ` · Queue #${t.queue_position}`
                              : ""}
                          </small>
                          {canCancelTransfer(t, uploads) &&
                            !["downloading", "uploading"].includes(t.status) &&
                            (!uploads || t.queue_position !== 0) && (
                              <TransferQueueControls
                                token={token}
                                file={t}
                                uploads={uploads}
                                changed={() => {
                                  setOrder("local_queue");
                                  setReverse(false);
                                }}
                              />
                            )}
                          <div
                            className="progress"
                            role="progressbar"
                            aria-label={`Progress for ${basename(t.path)}`}
                            aria-valuenow={Math.round(progress(t) * 100)}
                            aria-valuemin={0}
                            aria-valuemax={100}
                            aria-valuetext={`${t.transferred.toLocaleString()} of ${t.size.toLocaleString()} bytes transferred`}
                          >
                            <span
                              style={{
                                width: `${progress(t) * 100}%`,
                              }}
                            />
                          </div>
                          {t.status !== "completed" && (
                            <small
                              className="transfer-progress-count"
                              title={`${t.transferred.toLocaleString()} of ${t.size.toLocaleString()} bytes transferred`}
                            >
                              {bytes(t.transferred)} of {bytes(t.size)}{" "}
                              transferred
                            </small>
                          )}
                        </div>
                        <div className="transfer-state">
                          <span className={`status-tag ${t.status}`}>
                            {t.status === "completed" && <Check size={12} />}
                            {t.status === "retry_waiting"
                              ? `Retry scheduled${t.retry_at ? " · " + new Date(t.retry_at * 1000).toLocaleTimeString() : ""}`
                              : t.status === "waiting_for_space"
                                ? "Waiting for storage"
                                : t.status === "waiting"
                                  ? "Waiting for a download slot"
                                  : t.status === "paused_waiting"
                                    ? "Paused in local queue"
                                    : t.status.replaceAll("_", " ")}
                          </span>
                          <TransferTiming transfer={t} />
                          <small>{bytes(t.speed)}/s</small>
                          {t.detail && (
                            <small className="transfer-reason">
                              {t.detail}
                            </small>
                          )}
                        </div>
                        {uploads &&
                          [
                            "queued",
                            "uploading",
                            "checking_shares",
                            "retry_waiting",
                          ].includes(t.status) && (
                            <button
                              className="button small secondary"
                              disabled={!online && t.status !== "retry_waiting"}
                              onClick={() =>
                                send({
                                  type: "cancel_upload",
                                  username: t.username,
                                  path: t.path,
                                })
                              }
                            >
                              Cancel upload
                            </button>
                          )}
                        {!uploads && (
                          <div className="button-row">
                            {t.status === "waiting_for_space" && (
                              <button
                                className="button small secondary"
                                onClick={openStorage}
                              >
                                Storage settings
                              </button>
                            )}
                            <button
                              className="icon-button"
                              aria-label={`${t.status === "completed" ? (t.size === 0 ? "Open" : "Play") : ["failed", "timed_out", "interrupted"].includes(t.status) ? "Retry" : ["waiting", "waiting_for_space", "queued", "downloading", "retry_waiting"].includes(t.status) ? "Pause" : "Resume"} ${basename(t.path)}`}
                              disabled={
                                t.status !== "completed" &&
                                t.status !== "waiting_for_space" &&
                                !online
                              }
                              onClick={() =>
                                t.status === "completed"
                                  ? play(t)
                                  : send({
                                      type: "transfer",
                                      username: t.username,
                                      path: t.path,
                                      action: [
                                        "waiting",
                                        "waiting_for_space",
                                        "queued",
                                        "downloading",
                                        "retry_waiting",
                                      ].includes(t.status)
                                        ? "pause"
                                        : "resume",
                                    })
                              }
                            >
                              {[
                                "waiting",
                                "waiting_for_space",
                                "queued",
                                "downloading",
                                "retry_waiting",
                              ].includes(t.status) ? (
                                <Pause size={16} />
                              ) : (
                                <Play size={16} />
                              )}
                            </button>
                            <button
                              className="icon-button"
                              aria-label={`Cancel ${basename(t.path)}`}
                              disabled={
                                (!online && t.status !== "waiting_for_space") ||
                                [
                                  "completed",
                                  "cancelled",
                                  "interrupted",
                                ].includes(t.status)
                              }
                              onClick={() =>
                                send({
                                  type: "transfer",
                                  username: t.username,
                                  path: t.path,
                                  action: "cancel",
                                })
                              }
                            >
                              <X size={16} />
                            </button>
                            <button
                              className="icon-button"
                              aria-label={`Remove ${basename(t.path)}`}
                              disabled={
                                ["queued", "downloading", "paused"].includes(
                                  t.status,
                                ) && online
                              }
                              onClick={() =>
                                send({
                                  type: "transfer",
                                  username: t.username,
                                  path: t.path,
                                  action: "remove",
                                })
                              }
                            >
                              <Trash2 size={16} />
                            </button>
                          </div>
                        )}
                      </div>
                    )}
                  />
                )}
              </section>
            ))}
          </div>
        ) : loading && paged ? (
          <p role="status">Loading transfers…</p>
        ) : (
          <Empty
            icon={uploads ? <ArrowUp size={27} /> : <ArrowDown size={27} />}
            title={uploads ? "Ready to share" : "A little room for discovery"}
            detail={
              uploads
                ? "Configure share folders on your server. Uploads appear here when a peer requests a file."
                : "Find something you love, then use the download button to add it here."
            }
          />
        )}
        <div className="result-pagination" aria-live="polite">
          <span>
            {matchingTotal.toLocaleString()} matching transfers ·{" "}
            {shown.length.toLocaleString()} loaded
            {loading ? " · Updating…" : ""}
          </span>
        </div>
        {!error &&
          (paged
            ? shown.length < matchingTotal
            : shown.length < matching.length) && (
            <LoadMore
              loading={loading && paged}
              trigger={shown.length}
              load={paged ? load : () => setLimit(shown.length + 100)}
            >
              Load more transfers
            </LoadMore>
          )}
      </section>
    </>
  );
}
function Stat({
  label,
  value,
  icon,
  detail,
}: {
  label: string;
  value: ReactNode;
  icon: ReactNode;
  detail?: string;
}) {
  return (
    <div className="stat-card" title={detail}>
      <span>
        {label}
        {icon}
      </span>
      <strong>{value}</strong>
    </div>
  );
}

function ChatPage({
  state,
  send,
  online,
  privateChat,
  readingEnabled,
  initialTarget,
  token,
  changeTarget,
  drafts,
  changeDrafts,
  searchRoom,
}: {
  state: Snapshot;
  send: (c: object) => unknown;
  online: boolean;
  privateChat: boolean;
  readingEnabled: boolean;
  initialTarget: string;
  token: string;
  changeTarget: (s: string) => void;
  drafts: Record<string, string>;
  changeDrafts: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  searchRoom: (room: string) => void;
}) {
  const target = initialTarget;
  const [roomPane, setRoomPane] = useState<"chat" | "board" | "access">("chat");
  useEffect(() => setRoomPane("chat"), [target, privateChat]);
  const setTarget = changeTarget;
  const joined = [
    ...new Set([...(state.subscribed_rooms || []), ...state.joined_rooms]),
  ];
  useEffect(() => {
    if (!privateChat && !target && joined.length) setTarget(joined[0]);
  }, [privateChat, target, joined.join("\0")]);
  const [memberFilter, setMemberFilter] = useState("");
  const [memberLimit, setMemberLimit] = useState(100);
  const draftKey = (privateChat ? "private:" : "room:") + target;
  const draft = drafts[draftKey] || "";
  const setDraft = (value: string) =>
    changeDrafts((old) => ({ ...old, [draftKey]: value }));
  const [filter, setFilter] = useState("");
  const [roomSort, setRoomSort] = useChoice<RoomSort>("roomOrder", "users", [
    "users",
    "name",
    "activity",
  ]);
  const [roomKind, setRoomKind] = useState("all");
  const allRooms = [...state.rooms];
  for (const [name, info] of Object.entries(
    state.room_management?.rooms || {},
  )) {
    if (
      info.private &&
      (info.member || info.owner || joined.includes(name)) &&
      !allRooms.some((r) => r.name === name)
    )
      allRooms.push({
        name,
        users: state.room_members[name]?.length || 0,
        last_activity: null,
      });
  }
  const savedConversations = useConversations(
    token,
    state.message_revision !== undefined,
    state.message_reset || 0,
    state.message_revision,
  );
  const privateConversations = usePrivateConversations(
    state,
    savedConversations.data,
    privateChat ? target : "",
    setTarget,
  );
  const conversations = privateConversations.names;
  const liveMessages = state.messages.filter(
    (m) =>
      m.private === privateChat &&
      (privateChat
        ? userKey(m.target) === userKey(target)
        : m.target === target),
  );
  const history = useMessageHistory(
    token,
    target,
    privateChat,
    state.message_revision,
    state.message_reset || 0,
    liveMessages,
    JSON.stringify(
      state.user_rules
        .filter((r) => r.ignored)
        .map((r) => userKey(r.username))
        .sort(),
    ),
  );
  const messages = history.messages.filter(
    (m) => !isIgnored(state, m.username),
  );
  const [deleteHistory, setDeleteHistory] = useState(false);
  const [searchMessages, setSearchMessages] = useState(false);
  useEffect(() => setSearchMessages(false), [target, privateChat]);
  useEffect(() => setDeleteHistory(false), [target, privateChat]);
  const read = useRoomRead({
    enabled:
      readingEnabled && !searchMessages && (privateChat || roomPane === "chat"),
    privateChat,
    room: target,
    messages,
    readTo: privateChat
      ? state.private_read?.[userKey(target)] || 0
      : state.room_read?.[target] || 0,
    token,
  });
  return (
    <>
      <PageHeader
        eyebrow="THE PEOPLE BEHIND THE COLLECTIONS"
        title={
          privateChat
            ? "Keep the conversation going."
            : "Find your kind of people."
        }
        description={
          privateChat
            ? "Say thanks for a great find, or trade a recommendation."
            : "Join a room, share a recommendation, and discover something together."
        }
        action={
          !privateChat && (
            <button
              className="button secondary"
              disabled={!online}
              onClick={() => send({ type: "refresh_rooms" })}
            >
              <RefreshCw size={16} />
              Refresh rooms
            </button>
          )
        }
      />
      {!privateChat && roomPane === "chat" && (
        <RoomJoin state={state} send={send} choose={setTarget} />
      )}
      {!privateChat && joined.length > 0 && (
        <div
          className="room-tabs"
          role="tablist"
          aria-label="Joined rooms"
          onKeyDown={(e) => {
            const i = joined.indexOf(target);
            const next =
              e.key === "ArrowRight"
                ? (i + 1) % joined.length
                : e.key === "ArrowLeft"
                  ? (i + joined.length - 1) % joined.length
                  : e.key === "Home"
                    ? 0
                    : e.key === "End"
                      ? joined.length - 1
                      : -1;
            if (next >= 0) {
              e.preventDefault();
              setTarget(joined[next]);
              document.getElementById("room-tab-" + next)?.focus();
            }
          }}
        >
          {joined.map((room, i) => (
            <button
              role="tab"
              id={"room-tab-" + i}
              aria-label={room}
              aria-selected={target === room}
              tabIndex={target === room ? 0 : -1}
              key={room}
              onClick={() => setTarget(room)}
            >
              {room}
              {!state.joined_rooms.includes(room) && (
                <small>
                  {state.room_management?.rooms[room]?.error
                    ? "Needs attention"
                    : online
                      ? "Joining…"
                      : "Offline"}
                </small>
              )}
              {(state.room_unread?.[room] || 0) > 0 && (
                <span
                  className="unread-count"
                  aria-label={`${state.room_unread?.[room]} unread messages`}
                >
                  {state.room_unread?.[room]}
                </span>
              )}
            </button>
          ))}
        </div>
      )}
      <div
        className={`chat-layout ${!privateChat && roomPane !== "chat" ? "room-focused" : ""}`}
      >
        {(privateChat || roomPane === "chat") && (
          <section className="panel room-list">
            <h2>{privateChat ? "Conversations" : "Browse rooms"}</h2>
            {!privateChat && (
              <>
                <label className="room-sort-label">
                  Order rooms
                  <select
                    aria-label="Order rooms"
                    value={roomSort}
                    onChange={(e) => setRoomSort(e.target.value as RoomSort)}
                  >
                    <option value="users">Most users</option>
                    <option value="name">Name A–Z</option>
                    <option value="activity">Last observed activity</option>
                  </select>
                </label>
                <p className="hint">
                  Activity is observed while connected to joined rooms. Other
                  rooms show unknown.
                </p>
                <label className="room-sort-label">
                  Show rooms
                  <select
                    aria-label="Room visibility"
                    value={roomKind}
                    onChange={(e) => setRoomKind(e.target.value)}
                  >
                    <option value="all">All rooms</option>
                    <option value="public">Public rooms</option>
                    <option value="private">My private rooms</option>
                  </select>
                </label>
              </>
            )}
            <input
              aria-label={privateChat ? "Message username" : "Filter rooms"}
              placeholder={privateChat ? "Enter username…" : "Find a room…"}
              value={privateChat ? target : filter}
              onChange={(e) =>
                privateChat
                  ? setTarget(e.target.value)
                  : setFilter(e.target.value)
              }
            />
            {privateChat
              ? conversations.map((u) => (
                  <div
                    className={`room-item ${userKey(target) === userKey(u) ? "chosen" : ""}`}
                    key={u}
                    role="button"
                    tabIndex={0}
                    aria-label={`Open conversation with ${u}`}
                    aria-pressed={userKey(target) === userKey(u)}
                    onClick={() => setTarget(u)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        setTarget(u);
                      }
                    }}
                  >
                    <span className="user-identity">
                      <strong>{u}</strong>
                      <UserStatus username={u} />
                    </span>
                    {(state.private_unread?.[userKey(u)] || 0) > 0 && (
                      <span
                        className="unread-count"
                        aria-label={`${state.private_unread?.[userKey(u)]} unread messages`}
                      >
                        {state.private_unread?.[userKey(u)]}
                      </span>
                    )}
                    <ChevronRight size={16} aria-hidden="true" />
                  </div>
                ))
              : sortRooms(allRooms, roomSort)
                  .filter(
                    (r) =>
                      r.name.toLowerCase().includes(filter.toLowerCase()) &&
                      (roomKind === "all" ||
                        (roomKind === "private") ==
                          Boolean(
                            state.room_management?.rooms[r.name]?.private,
                          )),
                  )
                  .map((r) => (
                    <button
                      className={`room-item ${target === r.name ? "chosen" : ""}`}
                      key={r.name}
                      onClick={() => setTarget(r.name)}
                    >
                      <span
                        title={
                          state.room_management?.rooms[r.name]?.private
                            ? "Private room"
                            : "Public room"
                        }
                      >
                        {state.room_management?.rooms[r.name]?.private ? (
                          <LockKeyhole size={14} />
                        ) : (
                          "#"
                        )}
                      </span>
                      <strong>
                        {r.name}
                        {(state.room_unread?.[r.name] || 0) > 0 && (
                          <span className="unread-count">
                            {state.room_unread?.[r.name]}
                          </span>
                        )}
                      </strong>
                      <small>
                        {state.room_management?.rooms[r.name]?.private &&
                        !state.joined_rooms.includes(r.name)
                          ? "Private membership"
                          : `${r.users.toLocaleString()} users`}
                        {roomSort === "activity"
                          ? ` · ${r.last_activity ? new Date(r.last_activity * 1000).toLocaleString() : "Unknown activity"}`
                          : ""}
                      </small>
                    </button>
                  ))}
            {savedConversations.error && (
              <p role="alert">
                {savedConversations.error}{" "}
                <button
                  className="text-link"
                  onClick={savedConversations.retry}
                >
                  Retry saved conversations
                </button>
              </p>
            )}
            {!privateChat &&
              savedConversations.data.some((c) => !c.private) && (
                <details>
                  <summary>Saved room history</summary>
                  {savedConversations.data
                    .filter((c) => !c.private)
                    .map((c) => (
                      <button
                        key={c.target}
                        className="room-item"
                        onClick={() => setTarget(c.target)}
                      >
                        {c.target}
                      </button>
                    ))}
                </details>
              )}
          </section>
        )}
        <section className="panel conversation">
          <div className="conversation-header">
            <strong>
              {privateChat && target ? (
                <User username={target} />
              ) : (
                target ||
                (privateChat ? "Start a conversation" : "Choose a room")
              )}
            </strong>
            {target && (
              <button
                className="button small secondary"
                aria-pressed={searchMessages}
                onClick={() => {
                  setSearchMessages(!searchMessages);
                  setRoomPane("chat");
                }}
              >
                Search messages
              </button>
            )}
            {target && (
              <button
                className="button small secondary"
                onClick={() => setDeleteHistory(true)}
              >
                Delete history
              </button>
            )}
            {privateChat && target && (
              <button
                className="button small secondary"
                onClick={privateConversations.close}
              >
                Close chat
              </button>
            )}
            {!privateChat && target && (
              <button
                className="button small secondary"
                disabled={!online}
                onClick={() => searchRoom(target)}
              >
                Search room files
              </button>
            )}
            {!privateChat && target && (
              <button
                className="button small secondary"
                disabled={
                  !online || !!state.room_management?.rooms[target]?.pending
                }
                onClick={() =>
                  send({
                    type: state.joined_rooms.includes(target)
                      ? "leave_room"
                      : state.room_management?.rooms[target]?.private &&
                          !state.room_management.rooms[target].member
                        ? "create_private_room"
                        : "join_room",
                    room: target,
                  })
                }
              >
                {state.room_management?.rooms[target]?.pending
                  ? "Joining…"
                  : state.joined_rooms.includes(target)
                    ? "Leave room"
                    : "Join room"}
              </button>
            )}
          </div>
          {deleteHistory && target && (
            <HistoryDelete
              target={target}
              privateChat={privateChat}
              send={send}
              close={() => setDeleteHistory(false)}
            />
          )}
          {!privateChat && target && (
            <div
              className="room-view-tabs"
              role="tablist"
              aria-label={`Views for ${target}`}
              onKeyDown={(e) => {
                const tabs = Array.from(
                  e.currentTarget.querySelectorAll<HTMLButtonElement>(
                    '[role="tab"]',
                  ),
                );
                const at = tabs.indexOf(
                  document.activeElement as HTMLButtonElement,
                );
                const next =
                  e.key === "ArrowRight"
                    ? (at + 1) % tabs.length
                    : e.key === "ArrowLeft"
                      ? (at + tabs.length - 1) % tabs.length
                      : e.key === "Home"
                        ? 0
                        : e.key === "End"
                          ? tabs.length - 1
                          : -1;
                if (next >= 0) {
                  e.preventDefault();
                  tabs[next]?.focus();
                  tabs[next]?.click();
                }
              }}
            >
              {(
                [
                  "chat",
                  "board",
                  ...(state.room_management?.rooms[target]?.private
                    ? ["access"]
                    : []),
                ] as const
              ).map((pane) => (
                <button
                  key={pane}
                  type="button"
                  role="tab"
                  aria-selected={roomPane === pane}
                  tabIndex={roomPane === pane ? 0 : -1}
                  aria-controls={`room-${pane}-content`}
                  id={`room-view-${pane}`}
                  className="button small secondary"
                  onClick={() => setRoomPane(pane as typeof roomPane)}
                >
                  {pane === "chat"
                    ? "Chat"
                    : pane === "board"
                      ? "Room board"
                      : "Room access"}
                </button>
              ))}
            </div>
          )}
          {!privateChat &&
            roomPane === "chat" &&
            state.joined_rooms.includes(target) && (
              <details className="room-roster">
                <summary>
                  Room members (
                  {(state.room_members[target] || []).length.toLocaleString()})
                </summary>
                <input
                  aria-label="Filter room members"
                  value={memberFilter}
                  onChange={(e) => {
                    setMemberFilter(e.target.value);
                    setMemberLimit(100);
                  }}
                  placeholder="Find a member…"
                />
                <div className="buddy-list">
                  {(state.room_members[target] || [])
                    .filter((u) =>
                      u.toLowerCase().includes(memberFilter.toLowerCase()),
                    )
                    .slice(0, memberLimit)
                    .map((u) => (
                      <User key={u} username={u} />
                    ))}
                </div>
                {(state.room_members[target] || []).length > memberLimit && (
                  <button
                    className="button small secondary"
                    onClick={() => setMemberLimit((n) => n + 100)}
                  >
                    Show more members
                  </button>
                )}
                <p className="hint">
                  Up to 2,000 members are displayed. Presence subscriptions are
                  limited to 512 users; click a username to refresh its details.
                </p>
              </details>
            )}
          {!privateChat && target && roomPane !== "chat" && (
            <RoomTools
              key={target}
              state={state}
              room={target}
              token={token}
              send={send}
              tab={roomPane}
            />
          )}
          <div
            className="chat-pane"
            id="room-chat-content"
            hidden={!privateChat && roomPane !== "chat"}
          >
            {searchMessages && (
              <MessageSearch
                key={`${privateChat}:${target}`}
                token={token}
                target={target}
                privateChat={privateChat}
                visibility={JSON.stringify([
                  state.message_reset,
                  state.user_rules
                    .filter((r) => r.ignored)
                    .map((r) => userKey(r.username))
                    .sort(),
                ])}
                close={() => setSearchMessages(false)}
              />
            )}
            <div
              className="message-list"
              hidden={searchMessages}
              role="log"
              aria-label="Conversation"
              ref={read.viewport}
              onScroll={read.onScroll}
            >
              {history.older && (
                <button
                  className="button small secondary"
                  disabled={history.loading}
                  onClick={() => {
                    read.preserve();
                    history.loadOlder();
                  }}
                >
                  Load older messages
                </button>
              )}
              {history.loading && <p role="status">Loading message history…</p>}
              {history.error && (
                <p role="alert">
                  {history.error}{" "}
                  <button className="text-link" onClick={history.retry}>
                    Retry message history
                  </button>
                </p>
              )}
              {messages.length ? (
                <ConversationMessages messages={messages} />
              ) : (
                <Empty
                  icon={<MessageCircle size={24} />}
                  title="Good conversations start here."
                  detail={
                    privateChat
                      ? "Choose a username and send your first message."
                      : "Join a room to receive new messages."
                  }
                />
              )}
              <div
                ref={read.end}
                className="chat-read-sentinel"
                aria-hidden="true"
              />
            </div>
            {!read.bottom &&
              (privateChat
                ? state.private_unread?.[userKey(target)] || 0
                : state.room_unread?.[target] || 0) > 0 && (
                <button className="button small secondary" onClick={read.jump}>
                  New messages · Jump to latest
                </button>
              )}
            <ChatComposer
              draft={draft}
              setDraft={setDraft}
              send={send}
              target={target}
              privateChat={privateChat}
              disabled={
                !online ||
                !target ||
                (!privateChat && !state.joined_rooms.includes(target))
              }
            />
          </div>
        </section>
      </div>
    </>
  );
}
