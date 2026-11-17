import { useEffect, useState, type FormEvent } from "react";
import type { Snapshot } from "./types";
import { api } from "./api";
import SystemMessage from "./SystemMessage";
export default function RetrySettings({
  state,
  token,
}: {
  state: Snapshot;
  token: string;
}) {
  const [settings, setSettings] = useState(state.retry_policy);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(
    () => setSettings(state.retry_policy),
    [JSON.stringify(state.retry_policy)],
  );
  async function save(e: FormEvent) {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      await api(token, "commands", { type: "update_retry_policy", settings });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel wide">
      <h2>Download retries</h2>
      <form className="credentials-form" onSubmit={save}>
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={settings.enabled}
            onChange={(e) =>
              setSettings({ ...settings, enabled: e.target.checked })
            }
          />
          Automatically retry failed or timed-out downloads
        </label>
        <p className="hint">
          Enabled by default. Only new failures are scheduled. Delays increase
          after each attempt; disabling retries cancels pending retry timers.
          Files already downloaded are retained.
        </p>
        <div className="network-fields">
          <label>
            Maximum retry attempts
            <input
              aria-label="Maximum retry attempts"
              type="number"
              min={1}
              max={10}
              required
              value={settings.max_attempts}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  max_attempts: Number(e.target.value),
                })
              }
            />
          </label>
          <label>
            First retry delay (seconds)
            <input
              aria-label="First retry delay (seconds)"
              type="number"
              min={5}
              max={3600}
              required
              value={settings.base_delay_seconds}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  base_delay_seconds: Number(e.target.value),
                })
              }
            />
          </label>
          <label>
            Maximum retry delay (seconds)
            <input
              aria-label="Maximum retry delay (seconds)"
              type="number"
              min={settings.base_delay_seconds}
              max={86400}
              required
              value={settings.max_delay_seconds}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  max_delay_seconds: Number(e.target.value),
                })
              }
            />
          </label>
        </div>
        <button className="button secondary" disabled={busy}>
          {busy ? "Saving…" : "Save retry settings"}
        </button>
        {error && (
          <SystemMessage error onDismiss={() => setError("")}>
            {error}
          </SystemMessage>
        )}
      </form>
    </section>
  );
}
