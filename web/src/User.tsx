import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { FolderOpen, MessageCircle, RefreshCw, Star, X } from "lucide-react";
import type { Snapshot } from "./types";
import { bytes } from "./api";
import { userKey } from "./chatState";

type UserActions = { state: Snapshot; open: (username: string) => void };
export const UserContext = createContext<UserActions | null>(null);
export function UserStatus({ username }: { username: string }) {
  const context = useContext(UserContext);
  const info = context?.state.users?.[username];
  const status =
    context?.state.connection === "connected" ||
    username === context?.state.username
      ? info?.status || "unknown"
      : "unknown";
  const label = info?.pending
    ? "Checking"
    : {
        online: "Online",
        away: "Away",
        offline: "Offline",
        not_found: "Not found",
      }[status] || "Unknown";
  return (
    <span
      className={`user-status ${status}`}
      title={status === "unknown" ? "Status is not currently known" : label}
    >
      <span aria-hidden="true" className="presence-dot" />
      {label}
    </span>
  );
}
export default function User({ username }: { username: string }) {
  const context = useContext(UserContext);
  if (!context) return <span>{username}</span>;
  return (
    <span className="user-identity">
      <button
        type="button"
        className="text-link user-link"
        aria-label={`User options for ${username}`}
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          context.open(username);
        }}
      >
        {username}
      </button>
      <UserStatus username={username} />
    </span>
  );
}

export function UserDialog({
  username,
  state,
  close,
  send,
  browse,
  message,
  search,
  discover,
}: {
  username: string;
  state: Snapshot;
  close: () => void;
  send: (command: object) => unknown;
  browse: () => void;
  message: () => void;
  search?: () => void;
  discover?: (topic: string, dislike?: boolean) => void;
}) {
  const [pictureFailed, setPictureFailed] = useState(false);
  useEffect(
    () => setPictureFailed(false),
    [username, state.users?.[username]?.picture_revision],
  );
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    const previous = document.activeElement as HTMLElement | null;
    dialog?.showModal();
    return () => {
      dialog?.close();
      previous?.focus();
    };
  }, []);
  const user =
    state.users?.[username] ??
    (userKey(username) === userKey(state.username)
      ? state.users?.[state.username]
      : undefined);
  const self = userKey(username) === userKey(state.username);
  const online = state.connection === "connected";
  const buddy = state.buddies?.includes(username);
  const rule = state.user_rules.find(
    (r) => userKey(r.username) === userKey(username),
  );
  const [controls, setControls] = useState({
    ignored: rule?.ignored ?? false,
    banned: rule?.banned ?? false,
  });
  const [savingControls, setSavingControls] = useState(false);
  const [controlsError, setControlsError] = useState("");
  const actualControls = useRef(controls);
  actualControls.current = {
    ignored: rule?.ignored ?? false,
    banned: rule?.banned ?? false,
  };
  const pendingControls = useRef<typeof controls | null>(null);
  const matches = (a: typeof controls, b: typeof controls) =>
    a.ignored === b.ignored && a.banned === b.banned;
  useEffect(() => {
    const pending = pendingControls.current;
    if (pending && !matches(actualControls.current, pending)) return;
    pendingControls.current = null;
    setControls(actualControls.current);
    setSavingControls(false);
    setControlsError("");
  }, [username, rule?.ignored, rule?.banned]);
  useEffect(() => {
    if (!savingControls) return;
    const timer = setTimeout(() => {
      if (!pendingControls.current) return;
      pendingControls.current = null;
      setControls(actualControls.current);
      setSavingControls(false);
      setControlsError(
        "Could not confirm user controls. Check the connection and try again.",
      );
    }, 8000);
    return () => clearTimeout(timer);
  }, [savingControls, username]);
  async function updateControls(next: typeof controls) {
    pendingControls.current = next;
    setControls(next);
    setSavingControls(true);
    setControlsError("");
    try {
      if ((await send({ type: "set_user_rule", username, ...next })) === false)
        throw Error("Could not save user controls. Please try again.");
      if (
        pendingControls.current === next &&
        matches(actualControls.current, next)
      ) {
        pendingControls.current = null;
        setSavingControls(false);
      }
    } catch (error) {
      if (pendingControls.current !== next) return;
      pendingControls.current = null;
      setControls(actualControls.current);
      setControlsError((error as Error).message);
      setSavingControls(false);
    }
  }
  const detail = (label: string, value: ReactNode) => (
    <>
      <dt>{label}</dt>
      <dd>{value ?? "Unknown"}</dd>
    </>
  );
  return (
    <dialog
      ref={ref}
      className="user-dialog"
      aria-labelledby="user-dialog-title"
      onCancel={close}
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div className="user-dialog-heading">
        <div>
          <h2 id="user-dialog-title">
            {username}
            {self ? " (you)" : ""}
          </h2>
          <UserStatus username={username} />
        </div>
        <button
          type="button"
          className="icon-button"
          aria-label="Close user details"
          onClick={close}
        >
          <X size={20} />
        </button>
      </div>
      <div className="user-actions">
        {search && (
          <button
            className="button secondary"
            disabled={!online}
            onClick={search}
          >
            Search files
          </button>
        )}
        <button className="button" disabled={!online && !self} onClick={browse}>
          <FolderOpen size={16} />
          Browse shares
        </button>
        {!self && (
          <button className="button secondary" onClick={message}>
            <MessageCircle size={16} />
            Message
          </button>
        )}
        {!self && (
          <button
            className="button secondary"
            onClick={() => send({ type: "set_buddy", username, buddy: !buddy })}
          >
            <Star size={16} />
            {buddy ? "Remove buddy" : "Add buddy"}
          </button>
        )}
        <button
          className="button small secondary"
          disabled={!online && !self}
          onClick={() => send({ type: "lookup_user", username })}
        >
          <RefreshCw size={14} />
          Refresh details
        </button>
      </div>
      {!self && (
        <fieldset className="user-controls" disabled={savingControls}>
          <legend>Message & sharing controls</legend>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={controls.ignored}
              onChange={(e) =>
                void updateControls({
                  ignored: e.target.checked,
                  banned: controls.banned,
                })
              }
            />
            Ignore messages from this user
          </label>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={controls.banned}
              onChange={(e) =>
                void updateControls({
                  ignored: controls.ignored,
                  banned: e.target.checked,
                })
              }
            />
            Ban this user from my shares
          </label>
          <p className="hint">
            Ignoring hides this user's messages in every room and private chat,
            including saved history, and suppresses their unread alerts.
            Existing history is retained. Banning denies share lists and new
            upload requests, and cancels current uploads to this username.
            Soulseek usernames are not cryptographically authenticated.
          </p>
          {savingControls && <p role="status">Saving user controls…</p>}
          {controlsError && <p role="alert">{controlsError}</p>}
        </fieldset>
      )}
      {user?.pending && <p role="status">Looking up this exact username…</p>}
      {user?.error && <p role="alert">{user.error}</p>}
      <dl className="user-details">
        {detail("Shared files", user?.shared_files?.toLocaleString())}
        {detail("Shared folders", user?.shared_folders?.toLocaleString())}
        {detail(
          "Average upload speed",
          self && state.profile?.report_upload_speed === false
            ? "Not reported"
            : user?.average_speed == null
              ? null
              : `${bytes(user.average_speed)}/s`,
        )}
        {detail("Upload slots", user?.upload_slots)}
        {detail("Queued transfers", user?.queue_size)}
        {detail(
          "Free upload slot",
          user?.slots_free == null ? null : user.slots_free ? "Yes" : "No",
        )}
      </dl>
      {user?.has_picture && !pictureFailed && (
        <img
          className="user-profile-picture"
          alt={`${username}'s profile picture`}
          src={`/api/v1/users/picture?${new URLSearchParams({ username, v: user.picture_revision || "" })}`}
          onError={() => setPictureFailed(true)}
        />
      )}
      {pictureFailed && (
        <p className="hint">
          The profile picture is unavailable. Refresh details to try again.
        </p>
      )}
      <h3>About this user</h3>
      <p className="profile-description">
        {user?.description ||
          (user?.profile_pending
            ? "Fetching peer profile…"
            : "No description available.")}
      </p>
      {user?.profile_error && <p className="hint">{user.profile_error}</p>}
      <section className="user-interests" aria-label="User interests">
        <h3>Interests</h3>
        {user?.interests_pending && (
          <p role="status">Fetching public interests…</p>
        )}
        {user?.interests_error && (
          <p className="hint">{user.interests_error}</p>
        )}
        {(
          [
            [
              "Likes",
              user?.interests?.likes ||
                (self ? state.profile?.likes : []) ||
                [],
            ],
            [
              "Dislikes",
              user?.interests?.dislikes ||
                (self ? state.profile?.dislikes : []) ||
                [],
            ],
          ] as const
        ).map(([label, values]) => (
          <div key={label}>
            <h4>{label}</h4>
            <div className="interest-tags">
              {values.map((item) =>
                discover ? (
                  <button
                    className="button small secondary"
                    key={item}
                    disabled={Boolean(
                      state.discovery?.recommendations_pending ||
                      state.discovery?.people_pending,
                    )}
                    onClick={() => discover(item, label === "Dislikes")}
                    aria-label={`Explore ${item} from ${username}`}
                  >
                    {item}
                  </button>
                ) : (
                  <span key={item}>{item}</span>
                ),
              )}
            </div>
            {!values.length && (
              <p className="hint">
                {user?.interests_pending
                  ? "Waiting for the server…"
                  : user?.interests?.updated_at || self
                    ? "None listed."
                    : "No interests available yet."}
              </p>
            )}
          </div>
        ))}
        {!!user?.interests?.updated_at && !self && (
          <p className="hint">
            Public interests last updated{" "}
            {new Date(user.interests.updated_at * 1000).toLocaleString()}.
            Refresh details to update them.
          </p>
        )}
      </section>
      <p className="hint">
        Status and share counts come from Soulseek. The description and queue
        details are reported by the peer. Unknown status does not mean offline.
      </p>
    </dialog>
  );
}
