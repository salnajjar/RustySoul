import { useEffect, useId, useRef, useState } from "react";
import { Check, ChevronDown, LogOut, Plug } from "lucide-react";
import type { Snapshot } from "./types";

export default function NetworkStatus({
  state,
  busy,
  send,
  openAccounts,
}: {
  state: Snapshot;
  busy: boolean;
  send: (command: object) => Promise<unknown>;
  openAccounts: () => void;
}) {
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  const connected = state.connection === "connected";
  const connecting = ["connecting", "verifying"].includes(state.connection);
  const label = connected
    ? state.away
      ? state.away_automatic
        ? "Away (idle)"
        : "Away"
      : "Online"
    : connecting
      ? "Connecting…"
      : "Offline";
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!container.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        trigger.current?.focus();
      }
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);
  const command = (value: object) => {
    setOpen(false);
    void send(value);
  };
  return (
    <div
      className="network-status"
      ref={container}
      onBlur={(event) => {
        if (
          event.relatedTarget &&
          !event.currentTarget.contains(event.relatedTarget)
        )
          setOpen(false);
      }}
    >
      <button
        ref={trigger}
        className={`connection-label status-trigger ${connected ? (state.away ? "away" : "online") : ""}`}
        aria-label={`Soulseek status: ${label}`}
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
      >
        <span
          className={`dot ${connected ? (state.away ? "away" : "online") : ""}`}
        />
        {label}
        <ChevronDown size={12} aria-hidden="true" />
      </button>
      {open && (
        <div
          id={id}
          className="network-status-menu"
          role="region"
          aria-label="Soulseek status options"
        >
          <strong>{state.username || "Soulseek network"}</strong>
          <p>
            {connected
              ? state.away_automatic
                ? "Automatically Away. Activity returns you Online."
                : state.away
                  ? "Manually Away. Select Online when you return."
                  : "Available on Soulseek."
              : connecting
                ? "Connecting to Soulseek…"
                : "Disconnected from Soulseek."}
          </p>
          {connected && (
            <>
              <button
                disabled={busy}
                aria-pressed={!state.away}
                onClick={() => command({ type: "set_away", away: false })}
              >
                <span className="dot online" /> Online
                {!state.away && <Check size={15} aria-hidden="true" />}
              </button>
              <button
                disabled={busy}
                aria-pressed={state.away}
                onClick={() => command({ type: "set_away", away: true })}
              >
                <span className="dot away" /> Away
                {state.away && <Check size={15} aria-hidden="true" />}
              </button>
            </>
          )}
          {connected || connecting ? (
            <button
              className="network-disconnect"
              disabled={busy}
              onClick={() => command({ type: "disconnect" })}
            >
              <LogOut size={15} aria-hidden="true" /> Disconnect from Soulseek
            </button>
          ) : state.credentials_configured ? (
            <button
              disabled={busy}
              onClick={() => command({ type: "connect" })}
            >
              <Plug size={15} aria-hidden="true" /> Connect to Soulseek
            </button>
          ) : (
            <button
              onClick={() => {
                setOpen(false);
                openAccounts();
              }}
            >
              <Plug size={15} aria-hidden="true" /> Sign in to Soulseek
            </button>
          )}
        </div>
      )}
    </div>
  );
}
