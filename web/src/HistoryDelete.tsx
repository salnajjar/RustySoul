import { useState, type FormEvent } from "react";

export default function HistoryDelete({
  target,
  privateChat,
  send,
  close,
}: {
  target: string;
  privateChat: boolean;
  send: (command: object) => unknown;
  close: () => void;
}) {
  const [mode, setMode] = useState("all");
  const [days, setDays] = useState("30");
  const [before, setBefore] = useState("");
  const [from, setFrom] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function remove(event: FormEvent) {
    event.preventDefault();
    setError("");
    let range: object | undefined;
    const time = (value: string) =>
      Math.floor(new Date(value).getTime() / 1000);
    if (mode === "older_than") {
      if (
        !Number.isInteger(Number(days)) ||
        Number(days) < 1 ||
        Number(days) > 36500
      ) {
        setError("Choose 1 to 36500 days.");
        return;
      }
      range = { mode, days: Number(days) };
    } else if (mode === "before" || mode === "between") {
      const end = time(before),
        start = time(from);
      if (
        !Number.isFinite(end) ||
        end <= 0 ||
        (mode === "between" &&
          (!Number.isFinite(start) || start < 0 || start >= end))
      ) {
        setError("Choose valid dates with the end after the start.");
        return;
      }
      range =
        mode === "before"
          ? { mode, timestamp: end }
          : { mode, from: start, to: end };
    }
    setBusy(true);
    try {
      if (
        (await send({
          type: "clear_message_history",
          target,
          private: privateChat,
          ...(range ? { range } : {}),
        })) !== false
      )
        close();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <form
      className="chat-history-confirmation"
      role="alertdialog"
      aria-label="Delete conversation history"
      onSubmit={remove}
    >
      <p>
        Permanently delete {mode === "all" ? "all saved" : "the selected"}{" "}
        {privateChat ? "private chat" : "room"} messages{" "}
        {privateChat ? "with" : "in"} {target}
        {mode === "all" ? ", including older history" : ""}? New messages will
        still be saved.
      </p>
      <fieldset disabled={busy}>
        <label>
          Messages to delete
          <select
            aria-label="Messages to delete"
            value={mode}
            onChange={(e) => {
              setMode(e.target.value);
              setError("");
            }}
          >
            <option value="all">All messages in this conversation</option>
            <option value="older_than">Older than a number of days</option>
            <option value="before">Before a date and time</option>
            <option value="between">Within a date range</option>
          </select>
        </label>
        {mode === "older_than" && (
          <label>
            Older than (days)
            <input
              aria-label="Older than (days)"
              type="number"
              required
              min="1"
              max="36500"
              step="1"
              value={days}
              onChange={(e) => setDays(e.target.value)}
            />
          </label>
        )}
        {mode === "between" && (
          <label>
            From (including)
            <input
              aria-label="Delete messages from"
              type="datetime-local"
              required
              value={from}
              onChange={(e) => setFrom(e.target.value)}
            />
          </label>
        )}
        {(mode === "before" || mode === "between") && (
          <label>
            {mode === "before" ? "Before" : "Until (not including)"}
            <input
              aria-label="Delete messages before"
              type="datetime-local"
              required
              value={before}
              onChange={(e) => setBefore(e.target.value)}
            />
          </label>
        )}
        <p className="hint">
          Only this conversation is affected. Dates use your browser's time
          zone. This deletes saved messages, including ones hidden by ignore
          rules.
        </p>
        {error && <p role="alert">{error}</p>}
        <div className="button-row">
          <button className="button small" type="submit">
            Delete history permanently
          </button>
          <button
            className="button small secondary"
            type="button"
            onClick={close}
          >
            Cancel
          </button>
        </div>
      </fieldset>
    </form>
  );
}
