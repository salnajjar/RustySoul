import { useEffect, useMemo, useState } from "react";
import { isIgnored, userKey } from "./chatState";
import type { Snapshot } from "./types";
import type { Conversation } from "./useMessageHistory";

type Closed = Record<string, number>;
function readClosed(key: string): Closed {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) || "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(
      Object.entries(value).filter(
        ([, id]) =>
          typeof id === "number" && Number.isSafeInteger(id) && id >= 0,
      ),
    );
  } catch {
    return {};
  }
}
function persist(key: string, closed: Closed) {
  try {
    localStorage.setItem(key, JSON.stringify(closed));
  } catch {
    // Closing still works when browser storage is unavailable.
  }
}

export function nextPrivateConversation(
  names: string[],
  target: string,
  unread: Record<string, number> = {},
): string {
  const index = names.findIndex((name) => userKey(name) === userKey(target));
  const remaining = names.filter((name) => userKey(name) !== userKey(target));
  const forward = [
    ...names.slice(index + 1),
    ...names.slice(0, Math.max(0, index)),
  ];
  return (
    forward.find((name) => (unread[userKey(name)] || 0) > 0) ||
    (index > 0 ? names[index - 1] : "") ||
    remaining[Math.max(0, index)] ||
    ""
  );
}

export function usePrivateConversations(
  state: Snapshot,
  archive: Conversation[],
  selected: string,
  select: (username: string) => void,
) {
  const key = `rustysoul.closedChats.${userKey(state.username)}`;
  const [stored, setStored] = useState(() => ({
    key,
    closed: readClosed(key),
  }));
  const closed = useMemo(
    () => (stored.key === key ? stored.closed : readClosed(key)),
    [stored, key],
  );
  const entries = new Map<string, { name: string; latest: number }>();
  function add(name: string, id = 0) {
    const previous = entries.get(userKey(name));
    entries.set(userKey(name), {
      name: previous?.name || name,
      latest: Math.max(previous?.latest || 0, id),
    });
  }
  state.messages.filter((m) => m.private).forEach((m) => add(m.target, m.id));
  archive.filter((c) => c.private).forEach((c) => add(c.target, c.latest_id));
  const reopened = Object.keys(closed).filter(
    (name) =>
      name === userKey(selected) ||
      (!isIgnored(state, name) &&
        (entries.get(name)?.latest || 0) > closed[name]),
  );
  const reopenKey = JSON.stringify(reopened);
  useEffect(() => {
    if (stored.key !== key || reopened.length) {
      const next = { ...closed };
      for (const name of reopened) delete next[name];
      setStored({ key, closed: next });
      if (reopened.length) persist(key, next);
    }
  }, [key, stored.key, closed, reopenKey]);
  const names = [...entries.values()]
    .filter(
      ({ name }) =>
        !Object.hasOwn(closed, userKey(name)) ||
        reopened.includes(userKey(name)),
    )
    .map(({ name }) => name);
  function close() {
    if (!selected) return;
    // Use the latest known message ID, so late archive responses cannot reopen
    // old history. A genuinely newer message will reopen the conversation.
    const cutoff = [...entries.values()].reduce(
      (id, c) => Math.max(id, c.latest),
      state.messages.reduce((id, m) => Math.max(id, m.id || 0), 0),
    );
    const next = { ...closed, [userKey(selected)]: cutoff };
    persist(key, next);
    setStored({ key, closed: next });
    select(nextPrivateConversation(names, selected, state.private_unread));
  }
  return { names, close };
}
