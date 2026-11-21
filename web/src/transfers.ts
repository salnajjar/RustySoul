import type { Transfer } from "./types";
import { basename } from "./api";
export function isQueuedTransfer(transfer: Transfer) {
  return [
    "waiting",
    "queued",
    "waiting_for_space",
    "retry_waiting",
    "paused",
    "paused_waiting",
    "checking_shares",
  ].includes(transfer.status);
}
export const transferSorts = [
  ["activity", "Active transfers first"],
  ["speed", "Speed"],
  ["progress", "Progress"],
  ["eta", "Time remaining"],
  ["remaining", "Bytes remaining"],
  ["size", "File size"],
  ["name", "Filename"],
  ["folder", "Folder"],
  ["user", "Username"],
  ["status", "Status"],
  ["queue", "Queue position"],
  ["added", "Date added"],
  ["local_queue", "Local priority & queue order"],
] as const;
export type TransferSort = (typeof transferSorts)[number][0];
const active: Record<string, number> = {
  downloading: 0,
  uploading: 0,
  queued: 1,
  waiting: 2,
  waiting_for_space: 2,
  retry_waiting: 3,
  paused: 3,
  paused_waiting: 3,
  interrupted: 4,
  failed: 5,
  timed_out: 5,
  cancelled: 6,
  completed: 7,
};
export function progress(t: Transfer) {
  return t.status === "completed"
    ? 1
    : t.size > 0
      ? Math.min(1, Math.max(0, t.transferred / t.size))
      : 0;
}
export function remaining(t: Transfer) {
  return Math.max(0, t.size - t.transferred);
}
export function eta(t: Transfer) {
  return t.status === "completed"
    ? 0
    : t.speed > 0
      ? remaining(t) / t.speed
      : Infinity;
}
export function sortTransfers(
  items: Transfer[],
  sort: TransferSort,
  reverse = false,
  query = "",
) {
  return items
    .map((t, index) => ({ t, index }))
    .filter(({ t }) =>
      `${t.path} ${t.username}`.toLowerCase().includes(query.toLowerCase()),
    )
    .sort((a, b) => {
      let result = 0;
      switch (sort) {
        case "local_queue":
          result =
            (b.t.priority ?? 0) - (a.t.priority ?? 0) ||
            (a.t.local_queue_order ?? a.index) -
              (b.t.local_queue_order ?? b.index);
          break;
        case "activity":
          result =
            (active[a.t.status] ?? 9) - (active[b.t.status] ?? 9) ||
            b.t.speed - a.t.speed;
          break;
        case "speed":
          result = b.t.speed - a.t.speed;
          break;
        case "progress":
          result = progress(b.t) - progress(a.t);
          break;
        case "eta":
          result = eta(a.t) - eta(b.t);
          break;
        case "remaining":
          result = remaining(a.t) - remaining(b.t);
          break;
        case "size":
          result = b.t.size - a.t.size;
          break;
        case "name":
          result = basename(a.t.path).localeCompare(basename(b.t.path));
          break;
        case "folder":
          result = a.t.path.localeCompare(b.t.path);
          break;
        case "user":
          result = a.t.username.localeCompare(b.t.username);
          break;
        case "status":
          result = a.t.status.localeCompare(b.t.status);
          break;
        case "queue":
          result =
            (a.t.queue_position ?? Infinity) - (b.t.queue_position ?? Infinity);
          break;
        case "added":
          result = a.index - b.index;
          break;
      }
      // Unknown ETA/queue values stay at the end in both directions.
      if (
        sort === "eta" &&
        Number.isFinite(eta(a.t)) !== Number.isFinite(eta(b.t))
      )
        return Number.isFinite(eta(a.t)) ? -1 : 1;
      if (
        sort === "queue" &&
        (a.t.queue_position == null) !== (b.t.queue_position == null)
      )
        return a.t.queue_position == null ? 1 : -1;
      return (
        (Number.isFinite(result) && result ? result * (reverse ? -1 : 1) : 0) ||
        a.index - b.index
      );
    })
    .map(({ t }) => t);
}
