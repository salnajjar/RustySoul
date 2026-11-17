import { useEffect, useState, type FormEvent } from "react";
import { AudioLines, ChevronRight, ShieldCheck } from "lucide-react";
import { api } from "./api";
import SystemMessage from "./SystemMessage";
export interface Access {
  token: string;
  account: { username: string; must_change: boolean };
}
export function LoginScreen({
  onLogin,
}: {
  onLogin: (session: Access) => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const session = await api<Access>("", "auth/login", {
        username,
        password,
      });
      setPassword("");
      onLogin(session);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="login-screen">
      <div className="login-decoration" />
      <form className="login-panel" onSubmit={submit}>
        <span className="brand-icon">
          <AudioLines size={32} />
        </span>
        <span className="eyebrow">YOUR MUSIC. YOUR SERVER.</span>
        <h1>Welcome to RustySoul.</h1>
        <p>Sign in to your workspace.</p>
        <label htmlFor="admin-user">Username</label>
        <div className="input-icon">
          <input
            id="admin-user"
            autoComplete="username"
            maxLength={64}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            required
          />
        </div>
        <label htmlFor="admin-pass">Password</label>
        <div className="input-icon">
          <input
            id="admin-pass"
            type="password"
            autoComplete="current-password"
            maxLength={1024}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </div>
        {error && (
          <SystemMessage error onDismiss={() => setError("")}>
            {error}
          </SystemMessage>
        )}
        <button className="button" disabled={busy || !username || !password}>
          {busy ? "Signing in…" : "Sign in"}
          <ChevronRight size={17} />
        </button>
        <small>
          <ShieldCheck size={14} />
          First login requires a new username and password.
        </small>
      </form>
    </div>
  );
}
export function AccountForm({
  access,
  onChange,
  onCancel,
}: {
  access: Access;
  onChange: (session: Access) => void;
  onCancel?: () => void;
}) {
  const [username, setUsername] = useState(
    access.account.must_change ? "" : access.account.username,
  );
  const [current, setCurrent] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!saved) return;
    const timer = setTimeout(() => setSaved(false), 6000);
    return () => clearTimeout(timer);
  }, [saved]);
  async function submit(e: FormEvent) {
    e.preventDefault();
    setError("");
    setSaved(false);
    if (password !== confirm) {
      setError("The new passwords do not match");
      return;
    }
    setBusy(true);
    try {
      const next = await api<Access>(access.token, "auth/account", {
        current_password: current,
        username,
        password,
      });
      setCurrent("");
      setPassword("");
      setConfirm("");
      setSaved(true);
      onChange(next);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel wide">
      <h2>
        {access.account.must_change
          ? "Secure your administrator account"
          : "Administrator account"}
      </h2>
      <p>
        {access.account.must_change
          ? "Before using RustySoul, replace both the deployment username and password."
          : "Update the username and password used to sign in to RustySoul. This is separate from your Soulseek account."}
      </p>
      <form className="credentials-form" onSubmit={submit}>
        <label>
          Current password
          <input
            type="password"
            autoComplete="current-password"
            value={current}
            maxLength={1024}
            onChange={(e) => setCurrent(e.target.value)}
            required
          />
        </label>
        <label>
          New username
          <input
            autoComplete="username"
            value={username}
            minLength={3}
            maxLength={64}
            pattern={"[a-zA-Z0-9._\\-]+"}
            onChange={(e) => setUsername(e.target.value)}
            required
          />
        </label>
        <label>
          New password
          <input
            type="password"
            autoComplete="new-password"
            minLength={8}
            maxLength={1024}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </label>
        <label>
          Confirm new password
          <input
            type="password"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            required
          />
        </label>
        {error && (
          <SystemMessage error onDismiss={() => setError("")}>
            {error}
          </SystemMessage>
        )}
        {saved && (
          <SystemMessage onDismiss={() => setSaved(false)}>
            Administrator account updated. Other sessions have been signed out.
          </SystemMessage>
        )}
        <div className="button-row">
          <button className="button" disabled={busy}>
            {busy ? "Saving…" : "Save administrator account"}
          </button>
          {onCancel && (
            <button
              type="button"
              className="button secondary"
              onClick={onCancel}
            >
              Sign out
            </button>
          )}
        </div>
        <p className="hint">
          Use at least 8 characters. Updating this account signs out all other
          sessions.
        </p>
      </form>
    </section>
  );
}
