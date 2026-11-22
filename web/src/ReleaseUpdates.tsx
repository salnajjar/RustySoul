import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import type { Snapshot } from "./types";
import SystemMessage from "./SystemMessage";

export interface ReleaseStatus {
  current_version: string;
  install_type: "native" | "docker" | "podman" | "container";
  enabled: boolean;
  checking: boolean;
  checked_at: number | null;
  next_check_at: number | null;
  error: string | null;
  available: null | {
    version: string;
    release_url: string;
    instructions_url: string;
    image: string | null;
    native_url: string | null;
    checksum_url: string | null;
  };
}
export function useReleaseUpdates(
  token: string,
  enabled: boolean,
  interval = 5 * 60 * 1000,
) {
  const [status, setStatus] = useState<ReleaseStatus | null>(null);
  const refresh = useCallback(async () => {
    try {
      setStatus(await api<ReleaseStatus>(token, "updates"));
    } catch {
      /* Optional service; ordinary navigation stays quiet when offline. */
    }
  }, [token]);
  useEffect(() => {
    if (!enabled) return;
    void refresh();
    const timer = window.setInterval(() => void refresh(), interval);
    const focus = () => void refresh();
    window.addEventListener("focus", focus);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", focus);
    };
  }, [enabled, refresh, interval]);
  return { status, refresh };
}
export default function ReleaseNotice({
  token,
  enabled,
}: {
  token: string;
  enabled: boolean;
}) {
  const { status } = useReleaseUpdates(token, enabled);
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem("rustysoul.dismissed-release") || "";
    } catch {
      return "";
    }
  });
  const release = status?.available;
  if (!enabled || !status?.enabled || !release || dismissed === release.version)
    return null;
  return (
    <SystemMessage
      onDismiss={() => {
        setDismissed(release.version);
        try {
          localStorage.setItem("rustysoul.dismissed-release", release.version);
        } catch {
          /* Storage is optional. */
        }
      }}
    >
      <strong>RustySoul {release.version} is available.</strong> You are running{" "}
      {status.current_version}.{" "}
      <a href={release.release_url} target="_blank" rel="noreferrer">
        Release notes
      </a>
      {" · "}
      <a
        href={release.native_url || release.instructions_url}
        target="_blank"
        rel="noreferrer"
      >
        {release.native_url
          ? "Download Linux update"
          : `Update ${status.install_type === "container" ? "your container" : status.install_type === "native" ? "RustySoul" : status.install_type === "podman" ? "Podman" : "Docker"}`}
      </a>
      {release.image && (
        <small className="transfer-reason">
          Pull <code>{release.image}</code>, then recreate your container with
          its existing data, mounts and settings.{" "}
          <a href={release.instructions_url} target="_blank" rel="noreferrer">
            Instructions
          </a>
        </small>
      )}
      {release.checksum_url && (
        <>
          {" · "}
          <a href={release.checksum_url} target="_blank" rel="noreferrer">
            SHA-256 checksum
          </a>
        </>
      )}
    </SystemMessage>
  );
}
export function ReleaseSettings({
  state,
  token,
}: {
  state: Snapshot;
  token: string;
}) {
  const [enabled, setEnabled] = useState(state.network.check_for_updates);
  useEffect(
    () => setEnabled(state.network.check_for_updates),
    [state.network.check_for_updates],
  );
  const { status, refresh } = useReleaseUpdates(token, enabled, 5000);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function change(value: boolean) {
    setBusy(true);
    setError("");
    setEnabled(value);
    try {
      await api(token, "commands", {
        type: "update_network",
        settings: { ...state.network, check_for_updates: value },
      });
    } catch (e) {
      setEnabled(state.network.check_for_updates);
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  async function check() {
    setBusy(true);
    setError("");
    try {
      const result = await api<{ queued: boolean }>(token, "updates/check", {});
      if (!result.queued)
        setError(
          "A check is already running or was recently requested. Please try again later.",
        );
      await refresh();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel wide">
      <h2>Software updates</h2>
      <label className="checkbox-label">
        <input
          type="checkbox"
          checked={enabled}
          disabled={busy}
          onChange={(e) => void change(e.target.checked)}
        />
        Check for new releases automatically
      </label>
      <p className="hint">
        The daemon checks GitHub every six hours and shows a dismissible notice
        with instructions for your installation. Updates are installed by you.
        Checks send the RustySoul version; no account details or library data
        are sent.
      </p>
      <p>
        Installed version: {state.version}
        {status?.checked_at
          ? ` · Last checked ${new Date(status.checked_at * 1000).toLocaleString()}`
          : ""}
      </p>
      {status?.available && (
        <p>
          <a
            href={status.available.instructions_url}
            target="_blank"
            rel="noreferrer"
          >
            Update to {status.available.version}
          </a>
          {" · "}
          <a
            href={status.available.release_url}
            target="_blank"
            rel="noreferrer"
          >
            Release notes
          </a>
        </p>
      )}
      {status?.error && <p className="hint">{status.error}</p>}
      <button
        className="button secondary"
        disabled={!enabled || busy || status?.checking}
        onClick={() => void check()}
      >
        {status?.checking ? "Checking…" : "Check for updates now"}
      </button>
      {error && (
        <SystemMessage error onDismiss={() => setError("")}>
          {error}
        </SystemMessage>
      )}
    </section>
  );
}
