import { useEffect, useState, useRef, type FormEvent } from "react";
import { api, bytes } from "./api";
import SystemMessage from "./SystemMessage";
interface Ready {
  id: string;
  kind: "export" | "restore";
  bytes: number;
  can_restart: boolean;
  summary: {
    administrator: string;
    created_at: number;
    database_bytes: number;
    schema: number;
    downloads: number;
    uploads: number;
    indexed_files: number;
    locations: { downloads: string; shares: string[] };
  };
}
interface Status {
  last_result?: {
    id: string;
    status: string;
    message: string;
    at: number;
  } | null;
  busy: boolean;
  ready: Ready | null;
  can_restart: boolean;
}
export default function Backups({
  token,
  onRestored,
}: {
  token: string;
  onRestored: () => void;
}) {
  const restoreCallback = useRef(onRestored);
  restoreCallback.current = onRestored;
  const [suppressed, setSuppressed] = useState(false),
    [dismissed, setDismissed] = useState("");
  const [status, setStatus] = useState<Status>({
    busy: false,
    ready: null,
    can_restart: false,
  });
  const [pass, setPass] = useState(""),
    [repeat, setRepeat] = useState(""),
    [current, setCurrent] = useState("");
  const [file, setFile] = useState<File | null>(null),
    [restorePass, setRestorePass] = useState(""),
    [restoreCurrent, setRestoreCurrent] = useState(""),
    [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(""),
    [error, setError] = useState(""),
    [restarting, setRestarting] = useState(false);
  useEffect(() => {
    let active = true;
    const load = () =>
      api<Status>(token, "backups")
        .then((s) => {
          if (active) {
            setStatus(s);
            if (restarting && s.last_result?.status === "rejected")
              setRestarting(false);
          }
        })
        .catch((e) => {
          if (active && !restarting) setError(e.message);
        });
    void load();
    const timer = setInterval(load, 2000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [token, restarting]);
  useEffect(() => {
    if (!restarting) return;
    const timer = setInterval(() => {
      void fetch("/api/v1/auth/me", {
        credentials: "same-origin",
        cache: "no-store",
      })
        .then((r) => {
          if (r.status === 401) restoreCallback.current();
        })
        .catch(() => {});
    }, 2000);
    return () => clearInterval(timer);
  }, [restarting]);
  async function run(label: string, fn: () => Promise<Ready | void>) {
    setBusy(label);
    setError("");
    try {
      const ready = await fn();
      if (ready) setStatus((s) => ({ ...s, ready, busy: false }));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy("");
    }
  }
  async function create(e: FormEvent) {
    e.preventDefault();
    if (pass !== repeat) {
      setError("Backup passphrases do not match");
      return;
    }
    await run("Creating encrypted backup…", async () => {
      const result = await api<Ready>(token, "backups", {
        passphrase: pass,
        current_password: current,
      });
      setPass("");
      setRepeat("");
      setCurrent("");
      return result;
    });
  }
  async function inspect(e: FormEvent) {
    e.preventDefault();
    if (!file) return;
    await run("Uploading and validating backup…", async () => {
      const form = new FormData();
      form.append("passphrase", restorePass);
      form.append("file", file);
      const response = await fetch("/api/v1/backups/inspect", {
        method: "POST",
        body: form,
        credentials: "same-origin",
        headers: {
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
          "X-RustySoul-Request": "1",
        },
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok)
        throw new Error(
          data.error || `Backup validation failed (${response.status})`,
        );
      setSuppressed(false);
      setRestorePass("");
      setRestoreCurrent("");
      setConfirm("");
      return data as Ready;
    });
  }
  async function restore(e: FormEvent) {
    e.preventDefault();
    const ready =
      suppressed && status.ready?.kind === "restore" ? null : status.ready;
    if (!ready || ready.kind !== "restore" || suppressed) return;
    await run("Preparing restore…", async () => {
      await api(token, `backups/${ready.id}/restore`, {
        current_password: restoreCurrent,
        confirmation: confirm,
      });
      setRestoreCurrent("");
      setConfirm("");
      setRestarting(true);
    });
  }
  const blocked = !!busy || status.busy || restarting;
  const ready =
    suppressed && status.ready?.kind === "restore" ? null : status.ready;
  if (restarting)
    return (
      <section className="panel backup-panel">
        <h2>Restoring RustySoul</h2>
        <p role="status">
          RustySoul is restarting. Sign in with the administrator account from
          the backup when it returns. Soulseek starts disconnected and the
          library index is rebuilt.
        </p>
        <p>
          If this daemon runs without a service manager or container restart
          policy, start it again manually.
        </p>
        <button className="button" onClick={onRestored}>
          Continue to sign in
        </button>
      </section>
    );
  return (
    <section className="panel backup-panel">
      <h2>Backup &amp; restore</h2>
      <p>
        Save an encrypted copy of your settings, accounts, chats, searches and
        transfer history. Media files and installation configuration are kept
        separately.
      </p>
      {status.last_result && status.last_result.id !== dismissed && (
        <SystemMessage
          error={status.last_result.status === "rejected"}
          onDismiss={() => setDismissed(status.last_result!.id)}
        >
          {status.last_result.message}
        </SystemMessage>
      )}
      {error && (
        <SystemMessage error onDismiss={() => setError("")}>
          {error}
        </SystemMessage>
      )}
      {busy && (
        <p className="backup-progress" role="status">
          {busy}
          {!busy.startsWith("Preparing restore") && (
            <button
              type="button"
              className="button small secondary"
              onClick={() =>
                void api(token, "backups/cancel", {}).catch((e) =>
                  setError(e.message),
                )
              }
            >
              Cancel backup operation
            </button>
          )}
        </p>
      )}
      {!busy && status.busy && (
        <p role="status">Another backup operation or download is active.</p>
      )}
      <form onSubmit={create}>
        <h3>Create a backup</h3>
        <p>
          Keep the backup file and its passphrase separately. RustySoul cannot
          recover a forgotten backup passphrase.
        </p>
        <label>
          Current administrator password for backup
          <input
            type="password"
            autoComplete="current-password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
            disabled={blocked}
          />
        </label>
        <label>
          New backup passphrase
          <input
            type="password"
            autoComplete="new-password"
            minLength={8}
            maxLength={1024}
            value={pass}
            onChange={(e) => setPass(e.target.value)}
            disabled={blocked}
          />
        </label>
        <label>
          Confirm backup passphrase
          <input
            type="password"
            autoComplete="new-password"
            value={repeat}
            onChange={(e) => setRepeat(e.target.value)}
            disabled={blocked}
          />
        </label>
        <button
          className="button"
          disabled={blocked || !current || pass.length < 8 || pass !== repeat}
        >
          Create encrypted backup
        </button>
      </form>
      {ready?.kind === "export" && (
        <div className="backup-review">
          <h3>Backup ready</h3>
          <p>
            {bytes(ready.bytes)} · Created{" "}
            {new Date(ready.summary.created_at * 1000).toLocaleString()}
          </p>
          <a
            className="button"
            href={`/api/v1/backups/${ready.id}/file`}
            aria-disabled={blocked}
            onClick={(e) => {
              if (blocked) e.preventDefault();
            }}
          >
            Download encrypted backup
          </a>
          <p className="hint">
            The browser saves the file directly; it is not loaded into browser
            memory. Download it before creating another backup or restarting.
          </p>
        </div>
      )}
      <form onSubmit={inspect}>
        <h3>Restore a backup</h3>
        <p>
          Validation checks encryption, database integrity, credentials and
          mapped folders before anything is replaced. The current installation
          must have access to the backup’s library and download paths.
        </p>
        <label>
          Encrypted backup file
          <input
            type="file"
            accept=".rsb,application/octet-stream"
            disabled={blocked}
            onChange={(e) => {
              setSuppressed(true);
              const next = e.target.files?.[0] || null;
              if (next && next.size > 2147483648 + 4194304) {
                setError("Backup file exceeds the 2 GiB limit");
                setFile(null);
                e.target.value = "";
              } else {
                setFile(next);
                setError("");
              }
            }}
          />
        </label>
        <label>
          Backup passphrase
          <input
            type="password"
            autoComplete="off"
            value={restorePass}
            onChange={(e) => {
              setSuppressed(true);
              setRestorePass(e.target.value);
            }}
            disabled={blocked}
          />
        </label>
        <button
          className="button secondary"
          disabled={blocked || !file || restorePass.length < 8}
        >
          Validate backup
        </button>
      </form>
      {ready?.kind === "restore" && (
        <form onSubmit={restore} className="backup-review">
          <h3>Review before restoring</h3>
          <dl>
            <dt>Backup administrator</dt>
            <dd>{ready.summary.administrator}</dd>
            <dt>Created</dt>
            <dd>
              {new Date(ready.summary.created_at * 1000).toLocaleString()}
            </dd>
            <dt>Application data</dt>
            <dd>
              {bytes(ready.summary.database_bytes)} · Schema{" "}
              {ready.summary.schema}
            </dd>
            <dt>Transfer records</dt>
            <dd>
              {ready.summary.downloads.toLocaleString()} downloads ·{" "}
              {ready.summary.uploads.toLocaleString()} uploads
            </dd>
            <dt>Indexed files in backup</dt>
            <dd>{ready.summary.indexed_files.toLocaleString()}</dd>
            <dt>Download folder</dt>
            <dd>{ready.summary.locations.downloads}</dd>
            <dt>Library folders</dt>
            <dd>{ready.summary.locations.shares.join(", ") || "None"}</dd>
          </dl>
          <p>
            This replaces current application state. Media files and
            installation settings are preserved. Everyone is signed out; use the
            backup administrator’s password afterward. Soulseek and UPnP start
            disabled, pending downloads are paused and the library index is
            rebuilt. A rollback copy is retained.
          </p>
          <label>
            Current administrator password for restore
            <input
              type="password"
              autoComplete="current-password"
              value={restoreCurrent}
              onChange={(e) => setRestoreCurrent(e.target.value)}
              disabled={blocked}
            />
          </label>
          <label>
            Type RESTORE to confirm
            <input
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              autoComplete="off"
              disabled={blocked}
            />
          </label>
          <button
            className="button danger"
            disabled={
              blocked ||
              !ready.can_restart ||
              !restoreCurrent ||
              confirm !== "RESTORE"
            }
          >
            Restore and restart
          </button>
          {!ready.can_restart && (
            <p>
              Automatic restart is unavailable in this instance. Use the offline
              restore command.
            </p>
          )}
        </form>
      )}
      {status.ready && (
        <button
          className="button small secondary"
          disabled={blocked}
          onClick={() =>
            void run("Discarding server copy…", async () => {
              await api(token, "backups/clear", {});
              setStatus((s) => ({ ...s, ready: null }));
            })
          }
        >
          Discard server copy
        </button>
      )}
      <details className="backup-offline">
        <summary>Offline recovery</summary>
        <p>
          If RustySoul cannot start, stop the daemon and use its command-line
          restore with <code>--restore-backup</code>,{" "}
          <code>--backup-passphrase-stdin</code> and{" "}
          <code>--confirm-restore</code>. Instructions for native and container
          installations are included in BACKUPS.md. Never pass a passphrase as a
          command-line argument.
        </p>
      </details>
    </section>
  );
}
