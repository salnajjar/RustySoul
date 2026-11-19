import { useEffect, useState, type FormEvent } from "react";
import { api } from "./api";
import type { Snapshot } from "./types";
import SystemMessage from "./SystemMessage";
type Numeric =
  | "port"
  | "upload_slots"
  | "download_slots"
  | "upload_slots_per_user"
  | "download_slots_per_user"
  | "upload_limit_kib"
  | "download_limit_kib"
  | "upload_per_user_limit_kib"
  | "download_per_user_limit_kib"
  | "max_peers";
export default function ConnectionSettings({
  state,
  token,
}: {
  state: Snapshot;
  token: string;
}) {
  const [settings, setSettings] = useState(state.network);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const fixed = state.port_binding.fixed_port;
  useEffect(() => setSettings(state.network), [JSON.stringify(state.network)]);
  async function save(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      await api(token, "commands", {
        type: "update_network",
        settings: {
          ...settings,
          user_limits: settings.user_limits.map((u) => ({
            ...u,
            username: u.username.trim(),
          })),
        },
      });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  function number(
    label: string,
    key: Numeric,
    min: number,
    max: number,
    disabled = false,
  ) {
    return (
      <label>
        {label}
        <input
          type="number"
          min={min}
          max={max}
          required
          value={settings[key]}
          disabled={disabled}
          onChange={(e) =>
            setSettings({ ...settings, [key]: Number(e.target.value) })
          }
        />
      </label>
    );
  }
  return (
    <section className="panel wide network-settings">
      <h2>Network settings</h2>
      <p>
        Control incoming connections, concurrent transfers and the bandwidth
        available to everyone or individual users.
      </p>
      <p className="hint">
        Peer connections adapt to the container thread budget, keeping capacity
        for handshakes and file transfers.
        {state.effective_peer_limit
          ? ` Current effective peer limit: ${state.effective_peer_limit}.`
          : ""}
      </p>
      <form className="credentials-form" onSubmit={save}>
        <fieldset>
          <legend>Web interface (HTTP)</legend>
          <p>
            Current binding: <strong>{state.http_bind}</strong>
          </p>
          <p className="hint">
            Change bind in the configuration file and restart RustySoul. Use
            127.0.0.1:8484 for this server only, an adapter IP such as
            192.168.1.1:8484, or 0.0.0.0:8484 (all IPv4 adapters). Non-loopback
            bindings also require allow_remote = true. Use HTTPS through a
            reverse proxy for remote sign-in.
          </p>
        </fieldset>
        <fieldset>
          <legend>Incoming connections</legend>
          {number(
            "Incoming transfer port (TCP)",
            "port",
            1024,
            65535,
            state.connection !== "disconnected" || fixed !== null,
          )}
          <p className="hint">
            {fixed !== null
              ? `This container publishes TCP ${fixed}. To change it, set RUSTYSOUL_PUBLISHED_TRANSFER_PORT and the matching published TCP port in the deployment and recreate the container. The port shown here always matches the published port.`
              : "With Linux host networking or a native installation, this port listens directly on the server while connected. It follows this setting without recreating the container. The host firewall and router must also allow this TCP port."}
          </p>
          {state.connection !== "disconnected" && (
            <p className="hint">
              Disconnect from Soulseek before changing the port. Transfer limits
              can be changed while connected.
            </p>
          )}
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={settings.upnp}
              disabled={fixed !== null}
              onChange={(e) =>
                setSettings({ ...settings, upnp: e.target.checked })
              }
            />
            Automatically forward the transfer port with UPnP
          </label>
          <p className="hint">
            {fixed !== null
              ? "Bridge networking needs manual router forwarding. Use host networking for automatic UPnP discovery and forwarding."
              : "RustySoul renews its own router rule while connected and removes it on disconnect. Other applications' rules are left untouched. Upstream NAT/CGNAT or disabled router UPnP may require manual forwarding."}
          </p>
        </fieldset>
        <fieldset>
          <legend>Transfers at a time</legend>
          <div className="network-fields">
            {number("Maximum simultaneous uploads", "upload_slots", 0, 16)}
            {number("Maximum simultaneous downloads", "download_slots", 0, 32)}
            {number("Maximum uploads per user", "upload_slots_per_user", 1, 16)}
            {number(
              "Maximum downloads per user",
              "download_slots_per_user",
              1,
              32,
              settings.automatic_download_parallelism ?? true,
            )}
          </div>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={settings.automatic_download_parallelism ?? true}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  automatic_download_parallelism: e.target.checked,
                })
              }
            />
            Automatically use simultaneous downloads offered by a user
          </label>
          <p className="hint">
            Uses additional slots when a peer accepts more files, up to your
            total download limit. Peers still control their own upload limits.
            Turn this off to set a manual per-user maximum.
          </p>
          <p className="hint">
            Zero total slots pauses new transfers. Lowering a limit lets
            existing reservations finish. Active downloads, paused connections
            and accepted offers reserve slots; remote queue waits do not. Other
            users can fill free slots within the per-user caps.
          </p>
        </fieldset>
        <fieldset>
          <legend>Speed limits</legend>
          <p>
            Speeds are KiB/s (1 KiB = 1,024 bytes). Zero means unlimited. Total
            and per-user budgets apply together, across all files.
          </p>
          <div className="network-fields">
            {number(
              "Total upload limit (KiB/s)",
              "upload_limit_kib",
              0,
              1000000,
            )}
            {number(
              "Total download limit (KiB/s)",
              "download_limit_kib",
              0,
              1000000,
            )}
            {number(
              "Upload limit for each user (KiB/s)",
              "upload_per_user_limit_kib",
              0,
              1000000,
            )}
            {number(
              "Download limit for each user (KiB/s)",
              "download_per_user_limit_kib",
              0,
              1000000,
            )}
          </div>
          <p className="hint">
            For example, a 1,000 KiB/s total upload cap and a 200 KiB/s per-user
            cap lets each user receive up to 200 KiB/s while everyone together
            stays within 1,000 KiB/s. Changes apply to active transfers;
            protocol messages are not throttled.
          </p>
          <details>
            <summary>Individual user speed overrides</summary>
            <p className="hint">
              Overrides replace the default per-user caps for that username.
              Total caps still apply. Zero removes that user's directional cap.
            </p>
            {settings.user_limits.map((u, i) => (
              <fieldset className="user-limit" key={i}>
                <legend>User {i + 1}</legend>
                <label>
                  Username
                  <input
                    aria-label={`User ${i + 1} username`}
                    value={u.username}
                    maxLength={30}
                    required
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        user_limits: settings.user_limits.map((v, n) =>
                          n === i ? { ...v, username: e.target.value } : v,
                        ),
                      })
                    }
                  />
                </label>
                <div className="network-fields">
                  {(["upload_limit_kib", "download_limit_kib"] as const).map(
                    (key) => (
                      <label key={key}>
                        {key.startsWith("upload")
                          ? "Upload cap"
                          : "Download cap"}{" "}
                        (KiB/s)
                        <input
                          aria-label={`User ${i + 1} ${key.startsWith("upload") ? "upload" : "download"} limit`}
                          type="number"
                          min={0}
                          max={1000000}
                          required
                          value={u[key]}
                          onChange={(e) =>
                            setSettings({
                              ...settings,
                              user_limits: settings.user_limits.map((v, n) =>
                                n === i
                                  ? { ...v, [key]: Number(e.target.value) }
                                  : v,
                              ),
                            })
                          }
                        />
                      </label>
                    ),
                  )}
                </div>
                <button
                  type="button"
                  className="button small secondary"
                  aria-label={`Remove user ${i + 1} override`}
                  onClick={() =>
                    setSettings({
                      ...settings,
                      user_limits: settings.user_limits.filter(
                        (_, n) => n !== i,
                      ),
                    })
                  }
                >
                  Remove user override
                </button>
              </fieldset>
            ))}
            <button
              type="button"
              className="button small secondary"
              disabled={settings.user_limits.length >= 32}
              onClick={() =>
                setSettings({
                  ...settings,
                  user_limits: [
                    ...settings.user_limits,
                    {
                      username: "",
                      upload_limit_kib: 0,
                      download_limit_kib: 0,
                    },
                  ],
                })
              }
            >
              Add user override
            </button>
          </details>
        </fieldset>
        <fieldset>
          <legend>Connection behaviour</legend>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={settings.auto_reconnect}
              onChange={(e) =>
                setSettings({ ...settings, auto_reconnect: e.target.checked })
              }
            />
            Reconnect automatically if the network drops
          </label>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={settings.connect_on_startup}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  connect_on_startup: e.target.checked,
                })
              }
            />
            Connect to Soulseek when RustySoul starts
          </label>
          <p className="hint">
            Startup connection uses the verified account saved in Accounts.
            Automatic reconnect backs off after failures and stops if Soulseek
            reports that the account signed in elsewhere.
          </p>
          {number("Maximum connected peers", "max_peers", 1, 256)}
          <p className="hint">
            Lower values use fewer sockets and resources. Allow enough peers for
            your transfer slots and searches.
          </p>
        </fieldset>
        <div className="button-row">
          <button className="button" disabled={busy}>
            {busy ? "Saving…" : "Save connection settings"}
          </button>
          <button
            type="button"
            className="button secondary"
            onClick={() =>
              api(token, "commands", { type: "check_network" }).catch((e) =>
                setError(e.message),
              )
            }
          >
            Check router & connection
          </button>
        </div>
        {error && (
          <SystemMessage error onDismiss={() => setError("")}>
            {error}
          </SystemMessage>
        )}
        <div className="connectivity-card" role="status">
          <strong>
            {state.connectivity.mapped
              ? "Router forwarding configured"
              : state.connectivity.state === "checking"
                ? "Checking connection…"
                : "Connection information"}
          </strong>
          <p>
            {state.connectivity.message ||
              "Use Check router & connection to discover your router. This does not enable forwarding unless UPnP is selected."}
          </p>
          <small>
            Local transfer listener:{" "}
            {state.connectivity.listener_ready
              ? "ready"
              : state.connection === "connected"
                ? "not reachable locally; check the port and listener"
                : "not listening while disconnected"}
          </small>
          {state.connectivity.external_ip && (
            <p>Router external IP: {state.connectivity.external_ip}</p>
          )}
          {state.connectivity.gateway && (
            <p>
              Gateway {state.connectivity.gateway} ·{" "}
              {state.connectivity.mapped
                ? state.connectivity.lease_seconds
                  ? `${state.connectivity.lease_seconds / 60}-minute renewable lease`
                  : "permanent rule; removed on disconnect"
                : "No rule created"}
            </p>
          )}
        </div>
      </form>
    </section>
  );
}
