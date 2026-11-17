import { useEffect, useState, type FormEvent } from "react";
import { api } from "./api";
import type { Snapshot } from "./types";
import SystemMessage from "./SystemMessage";
import Privileges from "./Privileges";
export default function Credentials({
  token,
  state,
}: {
  token: string;
  state: Snapshot;
}) {
  const [username, setUsername] = useState(state.username);
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!saved) return;
    const timer = setTimeout(() => setSaved(false), 6000);
    return () => clearTimeout(timer);
  }, [saved, state.credentials_revision]);
  const disconnected = state.connection === "disconnected";
  useEffect(() => {
    setUsername(state.username);
    setPassword("");
    setPending(false);
    setSaved(state.credentials_revision > 0);
  }, [state.username, state.credentials_revision]);
  useEffect(() => {
    if (state.error) setPending(false);
  }, [state.error]);
  useEffect(() => {
    if (state.credentials_configured && state.connection === "disconnected")
      setSaved(false);
  }, [
    state.connection,
    state.credentials_revision,
    state.credentials_configured,
  ]);
  async function save(e: FormEvent) {
    e.preventDefault();
    setError("");
    setSaved(false);
    setPending(true);
    try {
      await api(token, "commands", {
        type: "update_credentials",
        username,
        password: password || null,
      });
    } catch (e) {
      setError((e as Error).message);
      setPending(false);
    }
  }
  async function remove() {
    setPending(true);
    setError("");
    setSaved(false);
    try {
      await api(token, "commands", { type: "clear_credentials" });
    } catch (e) {
      setError((e as Error).message);
      setPending(false);
    }
  }
  return (
    <section className="panel wide" id="soulseek-account">
      <h2>Soulseek account</h2>
      <form className="credentials-form" onSubmit={save}>
        <label>
          Soulseek username
          <input
            name="username"
            autoComplete="username"
            maxLength={30}
            required
            value={username}
            disabled={
              (!disconnected && state.connection !== "verifying") || pending
            }
            onChange={(e) => {
              setUsername(e.target.value);
              setSaved(false);
            }}
          />
        </label>
        <label>
          Soulseek password
          <input
            name="password"
            type="password"
            autoComplete="new-password"
            maxLength={1024}
            value={password}
            disabled={
              (!disconnected && state.connection !== "verifying") || pending
            }
            placeholder={
              state.credentials_configured && username === state.username
                ? "Saved — leave blank to keep"
                : "Enter your password"
            }
            required={
              !state.credentials_configured || username !== state.username
            }
            onChange={(e) => {
              setPassword(e.target.value);
              setSaved(false);
            }}
          />
        </label>
        <div className="button-row">
          <button
            className="button"
            disabled={
              (!disconnected && state.connection !== "verifying") ||
              pending ||
              !username
            }
          >
            {pending ? "Verifying login…" : "Save account"}
          </button>
          <button
            className="button secondary"
            type="button"
            disabled={
              (!disconnected && state.connection !== "verifying") ||
              pending ||
              !state.credentials_configured
            }
            onClick={remove}
          >
            Remove saved account
          </button>
        </div>
        {saved && (
          <SystemMessage onDismiss={() => setSaved(false)}>
            {state.credentials_configured
              ? "Login verified. Account saved and connected to Soulseek."
              : "Saved account removed."}
          </SystemMessage>
        )}
        {error && (
          <SystemMessage error onDismiss={() => setError("")}>
            {error}
          </SystemMessage>
        )}
        <p className="hint">
          {!disconnected && state.connection !== "verifying"
            ? "Disconnect from Soulseek before changing your account. "
            : ""}
          Saving first verifies your Soulseek login, then saves and connects if
          successful. Your saved password is never returned to the browser.
          Leave it blank to keep it when updating the same account. Use HTTPS or
          an SSH tunnel.
        </p>
      </form>
      <Privileges token={token} state={state} />
    </section>
  );
}
