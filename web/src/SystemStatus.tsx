import { useEffect, useState, type ReactNode } from "react";
import { bytes } from "./api";
import type { Snapshot, SystemStorage, SystemTransfers } from "./types";

const amount = (n: number | null | undefined) =>
  n == null ? "Not available" : bytes(n);
const count = (n: number | null | undefined) =>
  n == null ? "Not available" : n.toLocaleString();
export function duration(n: number) {
  const seconds = Math.max(0, Math.floor(n));
  const days = Math.floor(seconds / 86400),
    hours = Math.floor((seconds % 86400) / 3600),
    minutes = Math.floor((seconds % 3600) / 60);
  return days
    ? `${days}d ${hours}h ${minutes}m`
    : hours
      ? `${hours}h ${minutes}m`
      : minutes
        ? `${minutes}m ${seconds % 60}s`
        : `${seconds}s`;
}
function Card({
  title,
  children,
  detail,
}: {
  title: string;
  children: ReactNode;
  detail?: ReactNode;
}) {
  return (
    <div className="system-card">
      <h3>{title}</h3>
      <div className="system-card-value">{children}</div>
      {detail && <p>{detail}</p>}
    </div>
  );
}
function Budget({
  used,
  limit,
  unit = "bytes",
  label,
}: {
  used: number | null | undefined;
  limit: number | null | undefined;
  unit?: "bytes" | "tasks";
  label: string;
}) {
  const percent =
    used != null && limit != null && limit > 0
      ? Math.min(100, (used / limit) * 100)
      : null;
  return (
    <>
      <span>{unit === "bytes" ? amount(used) : count(used)}</span>
      {limit != null && (
        <small> / {unit === "bytes" ? amount(limit) : count(limit)}</small>
      )}
      {percent != null && (
        <>
          <meter
            aria-label={label}
            min={0}
            max={100}
            value={percent}
            low={70}
            high={85}
            optimum={0}
          />
          <p className={percent >= 85 ? "system-warning" : ""}>
            {percent.toFixed(1)}% of reported limit
            {percent >= 85 ? " · Near the limit" : ""}
          </p>
        </>
      )}
      {limit == null && <p>No cgroup limit reported</p>}
    </>
  );
}
function Storage({
  title,
  value,
}: {
  title: string;
  value: SystemStorage | undefined;
}) {
  const low =
    value?.available_bytes != null &&
    value.total_bytes != null &&
    value.total_bytes > 0 &&
    value.available_bytes / value.total_bytes < 0.05;
  return (
    <div className="system-storage">
      <h3>{title}</h3>
      <code>{value?.path || "Waiting for a sample"}</code>
      {value?.error ? (
        <p role="status">Capacity unavailable: {value.error}</p>
      ) : (
        <>
          <p>
            <strong>{amount(value?.available_bytes)}</strong> available
            {low && <span className="system-warning"> · Low free space</span>}
          </p>
          <p>
            {amount(value?.used_bytes)} used · {amount(value?.total_bytes)}{" "}
            total
          </p>
          {value?.total_bytes != null &&
            value.total_bytes > 0 &&
            value.used_bytes != null && (
              <meter
                aria-label={`${title} used space`}
                min={0}
                max={value.total_bytes}
                value={value.used_bytes}
                low={value.total_bytes * 0.7}
                high={value.total_bytes * 0.95}
                optimum={0}
              />
            )}
        </>
      )}
    </div>
  );
}
function Transfers({
  title,
  value,
}: {
  title: string;
  value: SystemTransfers | null | undefined;
}) {
  const states = value?.states || {};
  return (
    <div className="system-transfer-summary">
      <h3>{title}</h3>
      {value ? (
        <>
          <p>
            <strong>{value.total.toLocaleString()}</strong> retained records ·{" "}
            {bytes(value.speed)}/s
          </p>
          <dl>
            {[
              [
                "Transferring",
                (states.downloading || 0) + (states.uploading || 0),
              ],
              [
                "Waiting",
                (states.waiting || 0) +
                  (states.queued || 0) +
                  (states.checking_shares || 0),
              ],
              ["Waiting for storage", states.waiting_for_space || 0],
              ["Retry scheduled", states.retry_waiting || 0],
              ["Paused", states.paused || 0],
              ["Finalizing", states.finalizing || 0],
              ["Completed", states.completed || 0],
              [
                "Failed / interrupted",
                (states.failed || 0) +
                  (states.interrupted || 0) +
                  (states.timed_out || 0),
              ],
              ["Cancelled", states.cancelled || 0],
              ["Blocked", states.blocked || 0],
              ["Other", states.other || 0],
            ].map(([label, total]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>{(total as number).toLocaleString()}</dd>
              </div>
            ))}
          </dl>
        </>
      ) : (
        <p>Transfer statistics are not available yet.</p>
      )}
    </div>
  );
}
export default function SystemStatus({
  state,
  live,
  openLogs,
  openNetwork,
  openLibrary,
}: {
  state: Snapshot;
  live: boolean;
  openLogs: () => void;
  openNetwork: () => void;
  openLibrary: () => void;
}) {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now() / 1000), 5000);
    return () => clearInterval(timer);
  }, []);
  const v = state.system,
    age = v?.sampled_at ? Math.max(0, now - v.sampled_at) : null;
  const stale = age != null && age > 20;
  return (
    <section className="panel wide system-status">
      <div className="system-heading">
        <div>
          <h2>System status</h2>
          <p className="system-subtitle">
            RustySoul {state.version} ·{" "}
            {v?.sampled_at
              ? `Up ${duration(v.uptime_seconds)}`
              : "Starting system monitoring…"}
          </p>
        </div>
        <span
          className={`system-freshness ${!live || stale ? "system-warning" : ""}`}
          role="status"
        >
          {!live
            ? "Live updates interrupted"
            : stale
              ? "System readings delayed"
              : v?.sampled_at
                ? "Live · sampled every 5 seconds"
                : "Waiting for the first sample"}
        </span>
      </div>
      <p className="system-subtitle">
        {v?.sampled_at
          ? `Last sampled ${new Date(v.sampled_at * 1000).toLocaleString()}. `
          : ""}
        Resource samples are kept in memory; no extra history is written.
      </p>
      <div className="system-grid">
        <Card
          title="Soulseek network"
          detail={
            state.connectivity.message ||
            "Connection status reported by the daemon."
          }
        >
          {state.connection.replaceAll("_", " ")}
        </Card>
        <Card
          title="CPU usage"
          detail={`100% means one fully occupied CPU core.${v?.limits.cpu_cores != null ? ` Cgroup CPU quota: ${v.limits.cpu_cores.toLocaleString()} cores.` : ""}`}
        >
          {v?.process.cpu_percent == null
            ? v?.process_error
              ? "Not available"
              : "Measuring…"
            : `${v.process.cpu_percent.toFixed(1)}%`}
        </Card>
        <Card
          title="Daemon memory"
          detail={`${count(v?.process.threads)} daemon threads`}
        >
          {amount(v?.process.resident_bytes)}
        </Card>
        <Card
          title="Container / service memory"
          detail={
            v?.limits.memory_scope === "ancestor"
              ? "Shared parent budget, including other processes."
              : "Cgroup usage includes memory charged beyond the daemon's resident pages."
          }
        >
          <Budget
            used={v?.limits.memory_used_bytes}
            limit={v?.limits.memory_bytes}
            label="Cgroup memory usage"
          />
        </Card>
        <Card
          title="Container / service tasks"
          detail={`${v?.limits.tasks_scope === "ancestor" ? "Shared parent task budget. " : ""}Effective peer capacity: ${count(state.effective_peer_limit)}`}
        >
          <Budget
            used={v?.limits.tasks_used}
            limit={v?.limits.tasks}
            unit="tasks"
            label="Cgroup task usage"
          />
        </Card>
        <Card
          title="Download queue"
          detail="Unfinished files using the configured queue capacity."
        >
          {count(state.download_capacity_used)}
          <small> / {count(state.transfer_capacity)}</small>
        </Card>
        <Card
          title="Shared library"
          detail={`${state.shares.folders.toLocaleString()} folders${state.library.scanning ? " · Indexing now" : ""}`}
        >
          {state.shares.files.toLocaleString()} files
        </Card>
        <Card
          title="SQLite database"
          detail={`Write-ahead log: ${amount(v?.wal_bytes)}`}
        >
          {amount(v?.database_bytes)}
        </Card>
      </div>
      {v?.process_error && (
        <p className="system-warning">
          Some process readings are unavailable: {v.process_error}
        </p>
      )}
      <h3 className="system-section-title">Storage space</h3>
      <div className="system-storage-grid">
        <Storage title="Application data" value={v?.data} />
        <Storage title="Download destination" value={v?.downloads} />
      </div>
      <p className="system-subtitle">
        Available space excludes filesystem reserves. Folders on the same
        filesystem share this capacity, and other applications can use it.
      </p>
      <h3 className="system-section-title">Transfers across all history</h3>
      <div className="system-storage-grid">
        <Transfers title="Downloads" value={v?.download_transfers} />
        <Transfers title="Uploads" value={v?.upload_transfers} />
      </div>
      {v?.database_error && (
        <p role="status">
          Transfer statistics are temporarily unavailable: {v.database_error}
        </p>
      )}
      <div className="button-row system-actions">
        <button className="button secondary" onClick={openLogs}>
          View logs & activity
        </button>
        <button className="button secondary" onClick={openNetwork}>
          Open network settings
        </button>
        <button className="button secondary" onClick={openLibrary}>
          Open file locations
        </button>
      </div>
      <details className="system-monitoring">
        <summary>External monitoring</summary>
        <p>
          Prometheus-compatible metrics are available at{" "}
          <code>/api/v1/metrics</code> using the same authentication as the API.
          Metrics contain aggregate readings, without usernames, paths or chat
          contents.
        </p>
        <a
          className="button small secondary"
          href="/api/v1/metrics"
          target="_blank"
          rel="noreferrer"
        >
          View metrics
        </a>
      </details>
    </section>
  );
}
