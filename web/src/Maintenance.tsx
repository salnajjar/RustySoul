import { useEffect, useRef, useState } from "react";
import { api, bytes } from "./api";
import SystemMessage from "./SystemMessage";
interface Report {
  id: string;
  action: string;
  status: string;
  before_bytes: number;
  after_bytes: number;
  reclaimed_bytes: number;
  elapsed_ms: number;
  at: number;
  message: string;
}
interface Status {
  busy: boolean;
  pending: boolean;
  pending_id: string | null;
  can_restart: boolean;
  stats: {
    database_bytes: number;
    wal_bytes: number;
    reusable_bytes: number;
  } | null;
  stats_error: string | null;
  last_result: Report | null;
}
export default function Maintenance({
  token,
  onRestarting,
}: {
  token: string;
  onRestarting: (value: boolean) => void;
}) {
  const [status, setStatus] = useState<Status | null>(null),
    [error, setError] = useState(""),
    [requesting, setRequesting] = useState(false),
    [job, setJob] = useState<string | null>(null),
    [dismissed, setDismissed] = useState("");
  const callback = useRef(onRestarting);
  callback.current = onRestarting;
  useEffect(() => {
    let active = true;
    const load = () =>
      api<Status>(token, "maintenance")
        .then((s) => {
          if (!active) return;
          setStatus(s);
          if (s.pending_id && !job) setJob(s.pending_id);
          if (job && s.last_result?.id === job && !s.pending) {
            setJob(null);
            callback.current(false);
          }
        })
        .catch((e) => {
          if (active && !job) setError(e.message);
        });
    void load();
    const timer = setInterval(load, 3000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [token, job]);
  async function start(action: "optimize" | "compact") {
    setDismissed(status?.last_result?.id || "");
    setRequesting(true);
    setError("");
    callback.current(true);
    try {
      const result = await api<{ id: string }>(token, "maintenance", {
        action,
      });
      setJob(result.id);
    } catch (e) {
      setError((e as Error).message);
      callback.current(false);
    } finally {
      setRequesting(false);
    }
  }
  const busy = requesting || !!job || !!status?.pending || !!status?.busy;
  return (
    <section className="panel wide maintenance-panel">
      <h2>Database maintenance</h2>
      <p>
        Optimize query statistics to help searches and library browsing. Compact
        the database after clearing large amounts of history to reclaim unused
        space and rebuild its layout.
      </p>
      <p className="hint">
        Maintenance briefly restarts RustySoul so database writers can stop
        cleanly. Your browser session, accounts, settings and retained history
        are preserved. Transfers reconnect according to your startup settings.
        Media files stay in place.
      </p>
      {status?.stats && (
        <dl className="privileges-details">
          <dt>Database file</dt>
          <dd>{bytes(status.stats.database_bytes)}</dd>
          <dt>Write-ahead log</dt>
          <dd>{bytes(status.stats.wal_bytes)}</dd>
          <dt>Reusable database space</dt>
          <dd>{bytes(status.stats.reusable_bytes)}</dd>
        </dl>
      )}
      <p className="hint">
        Reusable pages can already hold new records. Compaction returns unused
        pages to the filesystem; the final saving can differ from this estimate.
      </p>
      <div className="button-row">
        <button
          className="button secondary"
          disabled={!status?.can_restart || busy}
          onClick={() => void start("optimize")}
        >
          Optimize query performance
        </button>
        <button
          className="button"
          disabled={!status?.can_restart || busy}
          onClick={() => void start("compact")}
        >
          Compact database and restart
        </button>
      </div>
      {(requesting || job || status?.pending) && (
        <p role="status">
          Database maintenance is running. RustySoul will reconnect here when it
          finishes; keep this tab open.
        </p>
      )}
      {status?.busy && !job && !status.pending && !requesting && (
        <p role="status">Another database or backup operation is active.</p>
      )}
      {status && !status.can_restart && (
        <p className="hint">
          Run the supervised daemon to enable database maintenance.
        </p>
      )}
      {status?.last_result && status.last_result.id !== dismissed && (
        <SystemMessage
          error={status.last_result.status !== "completed"}
          onDismiss={() => setDismissed(status.last_result!.id)}
        >
          <p>{status.last_result.message}</p>
          {status.last_result.status === "completed" && (
            <p>
              {bytes(status.last_result.before_bytes)} →{" "}
              {bytes(status.last_result.after_bytes)} · Database space
              reclaimed: {bytes(status.last_result.reclaimed_bytes)} ·{" "}
              {(status.last_result.elapsed_ms / 1000).toFixed(1)} seconds
            </p>
          )}
        </SystemMessage>
      )}
      {(error || status?.stats_error) && (
        <SystemMessage
          error
          onDismiss={() => {
            setError("");
            if (status) setStatus({ ...status, stats_error: null });
          }}
        >
          {error || status?.stats_error}
        </SystemMessage>
      )}
    </section>
  );
}
