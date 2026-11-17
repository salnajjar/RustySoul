import { useEffect, useRef, useState } from "react";
import { Star } from "lucide-react";
import { api } from "./api";
import type { Snapshot } from "./types";
import SystemMessage from "./SystemMessage";
export function privilegeLabel(state: Snapshot) {
  if (state.connection !== "connected") return "Privileges not checked";
  const info = state.privileges;
  if (info?.remaining_seconds != null && info.remaining_seconds > 0)
    return info.error ? "Last known privileges active" : "Privileges active";
  if (info?.pending) return "Checking privileges…";
  if (info?.error) return "Privilege check unavailable";
  if (info?.remaining_seconds === 0)
    return info.expires_at
      ? "Privileges expired — checking again"
      : "No active privileges";
  return "Privileges not checked";
}
export function privilegeDuration(seconds: number) {
  if (seconds < 60) return "Less than a minute";
  const days = Math.floor(seconds / 86400),
    hours = Math.floor((seconds % 86400) / 3600),
    minutes = Math.floor((seconds % 3600) / 60);
  if (days)
    return `${days} day${days === 1 ? "" : "s"}${hours ? ` ${hours} hour${hours === 1 ? "" : "s"}` : ""}`;
  if (hours)
    return `${hours} hour${hours === 1 ? "" : "s"}${minutes ? ` ${minutes} minute${minutes === 1 ? "" : "s"}` : ""}`;
  return `${minutes} minute${minutes === 1 ? "" : "s"}`;
}
export function PrivilegeBadge({
  state,
  open,
}: {
  state: Snapshot;
  open: () => void;
}) {
  const active =
    state.connection === "connected" &&
    (state.privileges?.remaining_seconds ?? 0) > 0;
  const label = privilegeLabel(state);
  return (
    <button
      className={`privilege-badge ${active ? "active" : ""}`}
      onClick={open}
      aria-label={`Soulseek privileges: ${label}`}
      title={`${label} — view status and purchase options`}
    >
      <Star size={16} aria-hidden="true" />
      <span>{active ? "Privileges active" : "Soulseek privileges"}</span>
    </button>
  );
}
export default function Privileges({
  token,
  state,
}: {
  token: string;
  state: Snapshot;
}) {
  const info = state.privileges,
    online = state.connection === "connected";
  const [error, setError] = useState(""),
    [copied, setCopied] = useState(false),
    [dismissed, setDismissed] = useState(false);
  const returning = useRef(false);
  useEffect(() => {
    setDismissed(false);
  }, [info?.error]);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 4000);
    return () => clearTimeout(timer);
  }, [copied]);
  useEffect(() => {
    returning.current = false;
    const check = () => {
      if (
        !returning.current ||
        !online ||
        document.visibilityState !== "visible"
      )
        return;
      returning.current = false;
      void api(token, "commands", {
        type: "check_privileges",
        watch_for_activation: true,
      }).catch((e) => setError(e.message));
    };
    window.addEventListener("focus", check);
    document.addEventListener("visibilitychange", check);
    return () => {
      window.removeEventListener("focus", check);
      document.removeEventListener("visibilitychange", check);
    };
  }, [online, state.username, token]);
  async function refresh(watch = false) {
    setError("");
    try {
      await api(token, "commands", {
        type: "check_privileges",
        watch_for_activation: watch,
      });
    } catch (e) {
      setError((e as Error).message);
    }
  }
  async function copyUsername() {
    try {
      await navigator.clipboard.writeText(state.username);
      setCopied(true);
    } catch {
      setError(
        "Could not copy automatically. Select and copy your Soulseek username above.",
      );
    }
  }
  const active = online && (info?.remaining_seconds ?? 0) > 0;
  const donation = `https://www.slsknet.org/qtlogin.php?username=${encodeURIComponent(state.username)}`;
  return (
    <section className="privileges-panel" aria-labelledby="privileges-heading">
      <h3 id="privileges-heading">Soulseek privileges</h3>
      <p
        className={`privileges-status ${active ? "active" : ""}`}
        role="status"
      >
        <Star size={18} aria-hidden="true" />
        {privilegeLabel(state)}
      </p>
      <p className="hint">
        Privileges give your downloads priority in Soulseek queues. File
        availability and transfer speed depend on the uploader.
      </p>
      {online && info?.checked_at && (
        <dl className="privileges-details">
          <dt>Soulseek account</dt>
          <dd>{state.username}</dd>
          {active && (
            <>
              <dt>Time remaining</dt>
              <dd>{privilegeDuration(info.remaining_seconds!)}</dd>
              <dt>Estimated expiry</dt>
              <dd>
                {info.expires_at
                  ? new Date(info.expires_at * 1000).toLocaleString()
                  : "Not available"}
              </dd>
            </>
          )}
          <dt>Last checked</dt>
          <dd>{new Date(info.checked_at * 1000).toLocaleString()}</dd>
        </dl>
      )}
      <div className="button-row">
        <button
          className="button secondary"
          type="button"
          disabled={!online || info?.pending}
          onClick={() => void refresh()}
        >
          {info?.pending ? "Checking privileges…" : "Refresh privileges"}
        </button>
        {online && info?.official_server && (
          <>
            <a
              className="button"
              href={donation}
              target="_blank"
              rel="noopener noreferrer"
              onClick={() => {
                returning.current = true;
                void refresh(true);
              }}
            >
              {active
                ? "Extend Soulseek privileges"
                : "Get Soulseek privileges"}
            </a>
            <button
              className="button secondary"
              type="button"
              onClick={() => void copyUsername()}
            >
              {copied ? "Username copied" : "Copy Soulseek username"}
            </button>
          </>
        )}
        {!online &&
          state.credentials_configured &&
          state.connection === "disconnected" && (
            <button
              className="button"
              type="button"
              onClick={() =>
                void api(token, "commands", { type: "connect" }).catch((e) =>
                  setError(e.message),
                )
              }
            >
              Connect and check privileges
            </button>
          )}
      </div>
      {!online && (
        <p className="hint">
          Connect your Soulseek account to check its current privileges and open
          purchase options.
        </p>
      )}
      {online && info?.official_server && (
        <p className="hint">
          The official Soulseek page opens in a new tab with{" "}
          <strong>{state.username}</strong> filled in. Confirm that username
          before purchasing. We recheck when you return and briefly watch for
          activation.{" "}
          <a
            href="https://www.slsknet.org/donate.php"
            target="_blank"
            rel="noopener noreferrer"
          >
            Alternative donation page
          </a>{" "}
          ·{" "}
          <a
            href="https://www.slsknet.org/thankyou.html"
            target="_blank"
            rel="noopener noreferrer"
          >
            Donation help
          </a>
        </p>
      )}
      {online && info && !info.official_server && (
        <p className="hint">
          This installation uses a custom Soulseek server. Official Soulseek
          donation links apply to accounts on the official network.
        </p>
      )}
      {online && info?.watching && (
        <p role="status">
          Watching for updated privileges for up to five minutes. Activation can
          take a few minutes.
        </p>
      )}
      {online && info?.error && !dismissed && (
        <SystemMessage error onDismiss={() => setDismissed(true)}>
          {info.error}
          {info.checked_at
            ? " The balance shown is from the last successful check."
            : ""}
        </SystemMessage>
      )}
      {error && (
        <SystemMessage error onDismiss={() => setError("")}>
          {error}
        </SystemMessage>
      )}
    </section>
  );
}
