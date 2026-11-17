import { useEffect, useState, type ReactNode } from "react";
import { api } from "./api";
import type { Snapshot } from "./types";
import User from "./User";
import SystemMessage from "./SystemMessage";
type Send = (c: object) => unknown;
export function RoomJoin({
  state,
  send,
  choose,
}: {
  state: Snapshot;
  send: Send;
  choose: (s: string) => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState("public");
  const [busy, setBusy] = useState(false);
  const [dismissed, setDismissed] = useState<string | null>(null);
  useEffect(
    () => setDismissed(null),
    [state.room_management?.invitations_error],
  );
  const [open, setOpen] = useState(
    !state.joined_rooms.length && !state.subscribed_rooms?.length,
  );
  const [invitations, setInvitations] = useState(
    Boolean(state.room_management?.accept_invitations),
  );
  useEffect(
    () => setInvitations(Boolean(state.room_management?.accept_invitations)),
    [state.room_management?.accept_invitations],
  );
  const online = state.connection === "connected";
  return (
    <details
      open={open}
      onToggle={(e) => setOpen(e.currentTarget.open)}
      className="panel room-join-settings"
      aria-label="Join rooms and invitations"
    >
      <summary>Join a room & invitations</summary>
      <div className="room-join-content">
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            const room = name.trim();
            if (!room) return;
            setBusy(true);
            try {
              if (
                (await send({
                  type:
                    kind === "private" ? "create_private_room" : "join_room",
                  room,
                })) !== false
              )
                choose(room);
            } finally {
              setBusy(false);
            }
          }}
        >
          <label>
            Room name
            <input
              aria-label="Room name to join"
              maxLength={24}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            Room type
            <select
              aria-label="Room type"
              value={kind}
              onChange={(e) => setKind(e.target.value)}
            >
              <option value="public">Public</option>
              <option value="private">Private</option>
            </select>
          </label>
          <button className="button" disabled={!online || busy || !name.trim()}>
            {kind === "private"
              ? "Create or join private room"
              : "Join room by name"}
          </button>
        </form>
        <p className="hint">
          Private rooms are visible to their members. A new private name makes
          you the owner. Leaving the chat keeps membership; cancelling
          membership is a separate action.
        </p>
        <label className="checkbox-label">
          <input
            type="checkbox"
            aria-label="Allow private-room invitations"
            checked={invitations}
            onChange={async (e) => {
              const enabled = e.target.checked;
              setInvitations(enabled);
              if (
                (await send({ type: "set_room_invitations", enabled })) ===
                false
              )
                setInvitations(
                  Boolean(state.room_management?.accept_invitations),
                );
            }}
          />
          Allow other people to add me to private rooms
        </label>
        {state.room_management?.invitations_error &&
          state.room_management.invitations_error !== dismissed && (
            <SystemMessage
              onDismiss={() => {
                setDismissed(state.room_management!.invitations_error);
                void send({ type: "dismiss_messages" });
              }}
            >
              {state.room_management.invitations_error}
            </SystemMessage>
          )}
        <p className="hint">
          {!online
            ? "Your preference is saved for the next connection."
            : state.room_management?.invitations_confirmed == null
              ? "Waiting for the server to confirm the invitation preference…"
              : state.room_management.invitations_confirmed ===
                  state.room_management.accept_invitations
                ? "Invitation preference confirmed by the server."
                : "The server reported a different invitation preference."}
        </p>
      </div>
    </details>
  );
}
type Confirmation = {
  action: string;
  username?: string;
  label: string;
  detail: string;
};
export default function RoomTools({
  state,
  room,
  token,
  send,
  tab,
}: {
  state: Snapshot;
  room: string;
  token: string;
  send: Send;
  tab: "board" | "access";
}) {
  const info = state.room_management?.rooms[room];
  const online = state.connection === "connected";
  const joined = state.joined_rooms.includes(room);
  const [ticker, setTicker] = useState(info?.own_ticker || "");
  const [username, setUsername] = useState("");
  const [confirm, setConfirm] = useState<Confirmation | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  useEffect(() => setDismissed(null), [info?.error]);
  const [busy, setBusy] = useState(false);
  useEffect(
    () => setConfirm(null),
    [room, info?.owner, info?.operator, info?.member, online],
  );
  useEffect(() => setTicker(info?.own_ticker || ""), [room, info?.own_ticker]);
  const owner = Boolean(
    info?.owner?.toLowerCase() === state.username.toLowerCase(),
  );
  const authorized = Boolean(
    online &&
    joined &&
    info?.confirmed &&
    info.owner_live &&
    (owner || info.operator) &&
    !info.pending,
  );
  async function act(action: string, user?: string) {
    setBusy(true);
    try {
      if (
        (await send({
          type: "private_room_action",
          room,
          action,
          username: user || null,
        })) !== false
      ) {
        setConfirm(null);
        setUsername("");
      }
    } finally {
      setBusy(false);
    }
  }
  function removal(action: string, user: string) {
    setConfirm({
      action,
      username: user,
      label: `${action === "remove_member" ? "Remove member" : "Remove operator"} ${user}`,
      detail:
        action === "remove_member"
          ? "They will lose access to this private room. Their files and your transfer history are unaffected."
          : "They will keep membership but lose operator privileges.",
    });
  }
  return (
    <section
      className="room-tools"
      role="tabpanel"
      id={`room-${tab}-content`}
      aria-label={`${tab === "board" ? "Room board" : "Room access"} for ${room}`}
    >
      <div className="room-tools-content">
        <h3>
          {tab === "board" ? "Room board" : "Room access"}{" "}
          <span className="hint">{room}</span>
        </h3>
        {info?.pending && <p role="status">{info.pending}</p>}
        {info?.error && info.error !== dismissed && (
          <SystemMessage
            error
            onDismiss={() => {
              setDismissed(info.error);
              void send({ type: "dismiss_messages" });
            }}
          >
            {info.error}
          </SystemMessage>
        )}
        {tab === "board" ? (
          <>
            <p className="hint">
              Short messages that room members see on the room board. Your saved
              message is published when you join this room.
            </p>
            <form
              className="room-ticker-form"
              onSubmit={async (e) => {
                e.preventDefault();
                setBusy(true);
                try {
                  await send({
                    type: "set_room_ticker",
                    room,
                    text: ticker.trim(),
                  });
                } finally {
                  setBusy(false);
                }
              }}
            >
              <label>
                My room-board message
                <input
                  aria-label="My room-board message"
                  maxLength={512}
                  value={ticker}
                  onChange={(e) => setTicker(e.target.value)}
                />
              </label>
              <button
                className="button small"
                disabled={
                  busy ||
                  Boolean(info?.pending) ||
                  (!joined && !state.subscribed_rooms?.includes(room))
                }
              >
                {online && joined
                  ? "Save and publish board message"
                  : "Save board message"}
              </button>
              <button
                type="button"
                className="button small secondary"
                disabled={
                  busy ||
                  Boolean(info?.pending) ||
                  (!joined && !state.subscribed_rooms?.includes(room))
                }
                onClick={async () => {
                  setTicker("");
                  await send({ type: "set_room_ticker", room, text: "" });
                }}
              >
                Clear my board message
              </button>
            </form>
            {!info?.board_known && (
              <p className="hint">
                Cached board. Join the room while connected to receive its
                current messages.
              </p>
            )}
            <RoomEntries
              key={room + ":board"}
              token={token}
              room={room}
              kind="board"
              revision={info?.revision || 0}
            />
          </>
        ) : (
          <>
            <dl className="room-access-summary">
              <dt>Owner</dt>
              <dd>
                {info?.owner ? <User username={info.owner} /> : "Not reported"}
              </dd>
              <dt>Your role</dt>
              <dd>
                {!info?.confirmed
                  ? "Cached — waiting for confirmation"
                  : owner
                    ? "Owner"
                    : info.operator
                      ? "Operator"
                      : info.member
                        ? "Member"
                        : "No membership"}
              </dd>
            </dl>
            {!joined && (
              <p className="hint">
                Join the room to manage access. Cached roles do not authorize
                changes.
              </p>
            )}
            {(owner || info?.operator) && (
              <form
                className="room-add-member"
                onSubmit={(e) => {
                  e.preventDefault();
                  void act("add_member", username.trim());
                }}
              >
                <label>
                  Invite a member
                  <input
                    aria-label="Username to add to private room"
                    maxLength={30}
                    value={username}
                    onChange={(e) => setUsername(e.target.value)}
                  />
                </label>
                <button
                  className="button small"
                  disabled={!authorized || busy || !username.trim()}
                >
                  Add member
                </button>
              </form>
            )}
            <h4>Members with access ({info?.members || 0})</h4>
            {!info?.members_known && (
              <p className="hint">
                The complete current membership list has not been received yet.
              </p>
            )}
            <RoomEntries
              key={room + ":member"}
              token={token}
              room={room}
              kind="member"
              revision={info?.revision || 0}
              actions={(entry) => {
                const self =
                  entry.username.toLowerCase() === state.username.toLowerCase();
                const isOwner =
                  entry.username.toLowerCase() === info?.owner?.toLowerCase();
                return !self && !isOwner && (owner || info?.operator) ? (
                  <div className="room-entry-actions">
                    {owner && (
                      <button
                        className="button small secondary"
                        disabled={!authorized || busy || !info?.members_known}
                        onClick={() =>
                          entry.operator
                            ? removal("remove_operator", entry.username)
                            : void act("add_operator", entry.username)
                        }
                        aria-label={`${entry.operator ? "Remove operator" : "Make operator"} ${entry.username}`}
                      >
                        {entry.operator ? "Remove operator" : "Make operator"}
                      </button>
                    )}
                    {(owner || (!entry.operator && info?.operators_known)) && (
                      <button
                        className="button small secondary"
                        disabled={!authorized || busy}
                        onClick={() => removal("remove_member", entry.username)}
                        aria-label={`Remove member ${entry.username}`}
                      >
                        Remove member
                      </button>
                    )}
                  </div>
                ) : null;
              }}
            />
            <h4>Operators ({info?.operators || 0})</h4>
            <RoomEntries
              key={room + ":operator"}
              token={token}
              room={room}
              kind="operator"
              revision={info?.revision || 0}
            />
            <div className="room-own-role-actions">
              {info?.confirmed && info.member && !owner && (
                <button
                  className="button small secondary"
                  disabled={!online || busy || Boolean(info.pending)}
                  onClick={() =>
                    setConfirm({
                      action: "leave_membership",
                      label: "Cancel my membership",
                      detail:
                        "You will leave the chat and lose access. You will need a new invitation to return.",
                    })
                  }
                >
                  Cancel my private-room membership
                </button>
              )}
              {info?.confirmed && info.operator && !owner && (
                <button
                  className="button small secondary"
                  disabled={!online || busy || Boolean(info.pending)}
                  onClick={() =>
                    setConfirm({
                      action: "resign_operator",
                      label: "Resign as operator",
                      detail:
                        "You will keep your room membership but lose operator privileges.",
                    })
                  }
                >
                  Resign as operator
                </button>
              )}
              {info?.confirmed && owner && (
                <button
                  className="button small secondary"
                  disabled={!online || busy || Boolean(info.pending)}
                  onClick={() =>
                    setConfirm({
                      action: "relinquish_ownership",
                      label: "Relinquish ownership",
                      detail:
                        "You will give up ownership of this private room. This cannot be undone here and does not transfer ownership to another user.",
                    })
                  }
                >
                  Relinquish room ownership
                </button>
              )}
            </div>
          </>
        )}
        {confirm && (
          <section
            className="room-action-confirm"
            role="group"
            aria-label="Confirm room access change"
          >
            <h4>{confirm.label}?</h4>
            <p>{confirm.detail}</p>
            <div className="button-row">
              <button
                className="button small"
                disabled={!online || busy || Boolean(info?.pending)}
                onClick={() => void act(confirm.action, confirm.username)}
              >
                Confirm {confirm.label}
              </button>
              <button
                className="button small secondary"
                disabled={busy}
                autoFocus
                onClick={() => setConfirm(null)}
              >
                Keep current access
              </button>
            </div>
          </section>
        )}
      </div>
    </section>
  );
}
type Entry = { username: string; text: string; operator: boolean };
function RoomEntries({
  token,
  room,
  kind,
  revision,
  actions,
}: {
  token: string;
  room: string;
  kind: "board" | "member" | "operator";
  revision: number;
  actions?: (e: Entry) => ReactNode;
}) {
  const [filter, setFilter] = useState("");
  const [offset, setOffset] = useState(0);
  const [data, setData] = useState<{ entries: Entry[]; total: number } | null>(
    null,
  );
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    const abort = new AbortController();
    setLoading(true);
    const timer = setTimeout(() => {
      void api<{ entries: Entry[]; total: number }>(
        token,
        "rooms/entries?" +
          new URLSearchParams({ room, kind, filter, offset: String(offset) }),
        undefined,
        abort.signal,
      )
        .then((result) => {
          setData(result);
          setError("");
          if (result.total > 0 && offset >= result.total)
            setOffset(Math.floor((result.total - 1) / 100) * 100);
        })
        .catch((e) => {
          if (!abort.signal.aborted) setError(e.message);
        })
        .finally(() => {
          if (!abort.signal.aborted) setLoading(false);
        });
    }, 150);
    return () => {
      clearTimeout(timer);
      abort.abort();
    };
  }, [token, room, kind, filter, offset, revision]);
  const label =
    kind === "board"
      ? "room-board messages"
      : kind === "member"
        ? "private-room members"
        : "private-room operators";
  return (
    <div className="room-entries" aria-busy={loading}>
      <label>
        Filter {label}
        <input
          aria-label={`Filter ${label}`}
          placeholder={
            kind === "board"
              ? "Search board messages or usernames in this room…"
              : "Find a person by username…"
          }
          value={filter}
          maxLength={128}
          onChange={(e) => {
            setFilter(e.target.value);
            setOffset(0);
          }}
        />
      </label>
      {error && <p role="alert">{error}</p>}
      <ul aria-label={label}>
        {data?.entries.map((e) => (
          <li key={e.username}>
            <div>
              <User username={e.username} />
              {kind === "board" && <p>{e.text}</p>}
              {kind === "member" && e.operator && (
                <span className="hint">Operator</span>
              )}
            </div>
            {actions?.(e)}
          </li>
        ))}
      </ul>
      {data && !data.total && (
        <p className="hint">
          {filter
            ? "No entries match this filter."
            : "No entries received yet."}
        </p>
      )}
      {!data && loading && <p role="status">Loading {label}…</p>}
      {data && data.total > 100 && (
        <div className="room-entry-pages">
          <button
            className="button small secondary"
            disabled={offset === 0 || loading}
            onClick={() => setOffset((n) => Math.max(0, n - 100))}
          >
            Previous {kind} page
          </button>
          <span>
            {offset + 1}–{Math.min(offset + 100, data.total)} of{" "}
            {data.total.toLocaleString()}
          </span>
          <button
            className="button small secondary"
            disabled={offset + 100 >= data.total || loading}
            onClick={() => setOffset((n) => n + 100)}
          >
            Next {kind} page
          </button>
        </div>
      )}
    </div>
  );
}
