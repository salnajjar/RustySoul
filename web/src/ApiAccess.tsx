import { useEffect, useState } from "react";
import { api } from "./api";
import SystemMessage from "./SystemMessage";
interface Key {
  id: string;
  name: string;
  scope: "read_only" | "control";
  created_at: number;
  expires_at: number | null;
  last_used_at: number | null;
}
interface View {
  enabled: boolean;
  keys: Key[];
  maximum_keys: number;
}
export default function ApiAccess({ token }: { token: string }) {
  const [view, setView] = useState<View | null>(null),
    [password, setPassword] = useState(""),
    [name, setName] = useState(""),
    [scope, setScope] = useState<Key["scope"]>("read_only"),
    [days, setDays] = useState("90"),
    [created, setCreated] = useState<{ key: Key; token: string } | null>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [copied, setCopied] = useState(false);
  useEffect(() => {
    let active = true;
    void api<View>(token, "api-access")
      .then((v) => {
        if (active) setView(v);
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [token]);
  async function change(body: object) {
    setBusy(true);
    setError("");
    try {
      const result = await api<View | { key: Key; token: string }>(
        token,
        "api-access",
        body,
      );
      if ("token" in result) {
        setCreated(result);
        setPassword("");
        setName("");
        setCopied(false);
        setView(await api<View>(token, "api-access"));
      } else setView(result);
      return true;
    } catch (e) {
      setError((e as Error).message);
      return false;
    } finally {
      setBusy(false);
    }
  }
  async function toggle(enabled: boolean) {
    const previous = view;
    if (!previous) return;
    setView({ ...previous, enabled });
    if (
      !(await change({
        action: "set_enabled",
        enabled,
        current_password: password,
      }))
    )
      setView(previous);
  }
  async function copy() {
    try {
      await navigator.clipboard.writeText(created!.token);
      setCopied(true);
    } catch {
      setError(
        "Could not copy automatically. Select and copy the new API key below.",
      );
    }
  }
  const when = (at: number | null) =>
    at ? new Date(at * 1000).toLocaleString() : "Never";
  return (
    <section className="panel wide api-access-panel">
      <h2>API access</h2>
      <p>
        Use a separate key for each integration. Browser sign-in continues to
        work when external API keys are disabled.
      </p>
      {error && (
        <SystemMessage error onDismiss={() => setError("")}>
          {error}
        </SystemMessage>
      )}
      <label>
        Current administrator password
        <input
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          disabled={busy}
        />
      </label>
      <p className="hint">
        Required to enable external API access or create a key.
      </p>
      <label className="checkbox-label">
        <input
          type="checkbox"
          checked={!!view?.enabled}
          disabled={busy || !view || (!view.enabled && !password)}
          onChange={(e) => void toggle(e.target.checked)}
        />
        Enable external API access
      </label>
      <p className="hint">
        Disabling access pauses all named keys; enabling it makes unrevoked,
        unexpired keys usable again. Changing administrator credentials or
        restoring a backup revokes keys.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void change({
            action: "create",
            name,
            scope,
            expires_in_days: Number(days),
            current_password: password,
          });
        }}
      >
        <label>
          Key name
          <input
            maxLength={64}
            required
            value={name}
            disabled={busy}
            placeholder="Monitoring or download automation"
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <div className="settings-columns">
          <label>
            API permissions
            <select
              value={scope}
              disabled={busy}
              onChange={(e) => setScope(e.target.value as Key["scope"])}
            >
              <option value="read_only">Read only</option>
              <option value="control">Read and control</option>
            </select>
          </label>
          <label>
            Key expiry
            <select
              value={days}
              disabled={busy}
              onChange={(e) => setDays(e.target.value)}
            >
              <option value="30">30 days</option>
              <option value="90">90 days</option>
              <option value="365">1 year</option>
              <option value="0">No expiry</option>
            </select>
          </label>
        </div>
        <p className="hint">
          Read only includes status, cached results, profiles, chat history and
          downloaded files. Control also allows searches, downloads/uploads,
          chat and application settings. Administrator credentials, API-key
          management, backups and database maintenance require browser
          administrator sign-in.
        </p>
        <button
          className="button"
          disabled={
            busy ||
            !view?.enabled ||
            !password ||
            !name.trim() ||
            view.keys.length >= view.maximum_keys
          }
        >
          Create API key
        </button>
      </form>
      {created && (
        <div className="backup-review">
          <h3>Copy your new API key</h3>
          <p>This key is shown once. Save it before leaving this page.</p>
          <label>
            New API key
            <input
              readOnly
              value={created.token}
              onFocus={(e) => e.target.select()}
            />
          </label>
          <div className="button-row">
            <button className="button" onClick={() => void copy()}>
              {copied ? "Key copied" : "Copy API key"}
            </button>
            <button
              className="button secondary"
              onClick={() => setCreated(null)}
            >
              I saved this key
            </button>
          </div>
        </div>
      )}
      <h3>Integration keys</h3>
      {!view?.keys.length && <p className="hint">No keys have been created.</p>}
      <ul className="api-key-list">
        {view?.keys.map((key) => (
          <li key={key.id}>
            <strong>{key.name}</strong>
            <p>
              {key.scope === "read_only" ? "Read only" : "Read and control"} ·{" "}
              {key.expires_at && key.expires_at * 1000 <= Date.now()
                ? "Expired"
                : view.enabled
                  ? "Enabled"
                  : "Disabled"}
            </p>
            <dl className="privileges-details">
              <dt>Created</dt>
              <dd>{when(key.created_at)}</dd>
              <dt>Expires</dt>
              <dd>{when(key.expires_at)}</dd>
              <dt>Last used</dt>
              <dd>
                {key.last_used_at ? when(key.last_used_at) : "Not used yet"}
              </dd>
            </dl>
            <button
              className="button small secondary"
              disabled={busy}
              onClick={() => void change({ action: "revoke", id: key.id })}
              aria-label={`Revoke key ${key.name}`}
            >
              Revoke key
            </button>
          </li>
        ))}
      </ul>
      {!!view?.keys.length && (
        <button
          className="button secondary"
          disabled={busy}
          onClick={() => void change({ action: "revoke_all" })}
        >
          Revoke all API keys
        </button>
      )}
      <h3>Connect an integration</h3>
      <p>
        API base URL: <code>{window.location.origin}/api/v1</code>
      </p>
      <p className="hint">
        Send the key in an Authorization header. Last-used times may lag by up
        to a minute. Keys are stored as hashes in a private installation
        database and are excluded from portable backups.
      </p>
      <pre className="api-example">{`curl -H "Authorization: Bearer YOUR_API_KEY" "${window.location.origin}/api/v1/status"`}</pre>
    </section>
  );
}
