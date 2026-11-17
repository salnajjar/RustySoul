import { useEffect, useState, type FormEvent } from "react";
import { api } from "./api";
import type { Snapshot, SharingSettings, Transfer } from "./types";
const defaults: SharingSettings = {
  mode: "sharing",
  min_files: 1,
  min_folders: 0,
  exempt_buddies: true,
  exceptions: [],
  allow_unknown: false,
  check_seconds: 30,
  cache_seconds: 300,
};
export default function SharingSettingsForm({
  state,
  token,
}: {
  state: Snapshot;
  token: string;
}) {
  const [settings, setSettings] = useState(state.sharing_policy || defaults);
  const [exceptions, setExceptions] = useState(settings.exceptions.join("\n"));
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [declines, setDeclines] = useState<Transfer[]>([]);
  useEffect(() => {
    const next = state.sharing_policy || defaults;
    setSettings(next);
    setExceptions(next.exceptions.join("\n"));
  }, [JSON.stringify(state.sharing_policy)]);
  useEffect(() => {
    const abort = new AbortController();
    const load = () =>
      api<{ transfers: Transfer[] }>(
        token,
        "transfers/uploads?status=blocked&order=updated",
        undefined,
        abort.signal,
      )
        .then((p) => setDeclines(p.transfers.slice(0, 10)))
        .catch(() => {});
    void load();
    const timer = setInterval(() => void load(), 10000);
    return () => {
      abort.abort();
      clearInterval(timer);
    };
  }, [token]);
  async function save(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      await api(token, "commands", {
        type: "update_sharing_policy",
        settings: {
          ...settings,
          exceptions: exceptions
            .split("\n")
            .map((s) => s.trim())
            .filter(Boolean),
        },
      });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel wide sharing-settings">
      <h2>Sharing protection</h2>
      <p>
        Ask other users to share a library before downloading from you. Declined
        requests can be retried after their sharing settings change; users are
        not permanently banned.
      </p>
      <form onSubmit={save}>
        <label>
          Who can download from me?
          <select
            aria-label="Upload sharing policy"
            value={settings.mode}
            onChange={(e) =>
              setSettings({
                ...settings,
                mode: e.target.value as SharingSettings["mode"],
              })
            }
          >
            <option value="sharing">
              Users with shared files (recommended)
            </option>
            <option value="buddies">Buddies only</option>
            <option value="off">Everyone — protection off</option>
          </select>
        </label>
        {settings.mode !== "off" && (
          <>
            <div className="network-fields">
              <label>
                Minimum shared files
                <input
                  aria-label="Minimum shared files"
                  type="number"
                  min={0}
                  max={100000000}
                  required
                  value={settings.min_files}
                  onChange={(e) =>
                    setSettings({
                      ...settings,
                      min_files: Number(e.target.value),
                    })
                  }
                />
              </label>
              <label>
                Minimum shared folders
                <input
                  aria-label="Minimum shared folders"
                  type="number"
                  min={0}
                  max={10000000}
                  required
                  value={settings.min_folders}
                  onChange={(e) =>
                    setSettings({
                      ...settings,
                      min_folders: Number(e.target.value),
                    })
                  }
                />
              </label>
            </div>
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={settings.exempt_buddies}
                onChange={(e) =>
                  setSettings({ ...settings, exempt_buddies: e.target.checked })
                }
              />
              Exempt buddies from the sharing requirements
            </label>
            <label>
              Always allow these users
              <textarea
                aria-label="Sharing protection exceptions"
                value={exceptions}
                rows={3}
                onChange={(e) => setExceptions(e.target.value)}
                placeholder="One trusted username per line"
              />
            </label>
            <details>
              <summary>Unknown users & check timing</summary>
              <p>
                A new request waits for server sharing statistics without using
                an upload slot. If the check times out, the default is to
                decline with an explanation.
              </p>
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={settings.allow_unknown}
                  onChange={(e) =>
                    setSettings({
                      ...settings,
                      allow_unknown: e.target.checked,
                    })
                  }
                />
                Allow uploads if sharing cannot be verified before the timeout
              </label>
              <div className="network-fields">
                <label>
                  Check timeout (seconds)
                  <input
                    aria-label="Sharing check timeout"
                    type="number"
                    min={5}
                    max={120}
                    value={settings.check_seconds}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        check_seconds: Number(e.target.value),
                      })
                    }
                  />
                </label>
                <label>
                  Refresh sharing statistics after (seconds)
                  <input
                    aria-label="Sharing statistics cache duration"
                    type="number"
                    min={30}
                    max={3600}
                    value={settings.cache_seconds}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        cache_seconds: Number(e.target.value),
                      })
                    }
                  />
                </label>
              </div>
            </details>
          </>
        )}
        <p className="hint">
          Soulseek forwards counts reported by clients. These checks discourage
          non-sharing clients; they cannot prove that every advertised file is
          accessible. Existing bans still apply, including to exceptions. Active
          uploads can finish when this policy changes.
        </p>
        <button className="button secondary" disabled={busy}>
          {busy ? "Saving…" : "Save sharing protection"}
        </button>
        {error && <p role="alert">{error}</p>}
      </form>
      {declines.length > 0 && (
        <details>
          <summary>
            Recently declined upload requests ({declines.length})
          </summary>
          {declines.map((t) => (
            <p key={t.username + t.path}>
              <strong>{t.username}</strong>:{" "}
              {t.detail || "Sharing requirements were not met."}
            </p>
          ))}
        </details>
      )}
    </section>
  );
}
