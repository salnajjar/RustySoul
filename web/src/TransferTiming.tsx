import { useMemo, useSyncExternalStore } from "react";
import type { Transfer } from "./types";

const terminal = new Set([
  "completed",
  "failed",
  "timed_out",
  "cancelled",
  "interrupted",
  "blocked",
]);
// One clock for all mounted active rows; no per-file timer or API polling.
let clock = Math.floor(Date.now() / 1000);
let timer: ReturnType<typeof setInterval> | undefined;
const listeners = new Set<() => void>();
const snapshot = () => clock;
const idleSubscribe = () => () => {};
function subscribe(listener: () => void) {
  listeners.add(listener);
  clock = Math.floor(Date.now() / 1000);
  if (!timer)
    timer = setInterval(() => {
      clock = Math.floor(Date.now() / 1000);
      listeners.forEach((notify) => notify());
    }, 1000);
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}

export function duration(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  const days = Math.floor(total / 86400);
  const parts = [
    Math.floor((total % 86400) / 3600),
    Math.floor((total % 3600) / 60),
    total % 60,
  ];
  return `${days ? `${days}d ` : ""}${parts.map((v) => String(v).padStart(2, "0")).join(":")}`;
}

export default function TransferTiming({
  transfer: t,
}: {
  transfer: Transfer;
}) {
  const start = useMemo(() => {
    if (t.started_at == null) return null;
    const date = new Date(t.started_at * 1000);
    if (!Number.isFinite(date.getTime())) return null;
    return {
      at: t.started_at,
      iso: date.toISOString(),
      title: date.toLocaleString(undefined, { timeZoneName: "short" }),
      label: date.toLocaleString(undefined, {
        year: "numeric",
        month: "short",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      }),
    };
  }, [t.started_at]);
  const running =
    start !== null && t.finished_at == null && !terminal.has(t.status);
  const now = useSyncExternalStore(
    running ? subscribe : idleSubscribe,
    snapshot,
    snapshot,
  );
  if (!start)
    return (
      <small className="transfer-timing">
        {[
          "queued",
          "waiting",
          "checking_shares",
          "paused_waiting",
          "retry_waiting",
        ].includes(t.status)
          ? "Not started"
          : "Timing unavailable"}
      </small>
    );
  const end = t.finished_at ?? (running ? now : null);
  return (
    <small className="transfer-timing">
      <span>
        Started{" "}
        <time dateTime={start.iso} title={start.title}>
          {start.label}
        </time>
      </span>
      <span title="Wall-clock time since this transfer started, including pauses and automatic retry waits. A manual retry after a terminal result starts a new timer.">
        {end == null
          ? "Duration unavailable"
          : `${t.finished_at == null ? "Elapsed" : "Total time"} ${duration(end - start.at)}`}
      </span>
    </small>
  );
}
