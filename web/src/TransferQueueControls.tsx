import { useState } from "react";
import type { Transfer } from "./types";
import { api } from "./api";
export default function TransferQueueControls({
  token,
  file,
  uploads,
  changed,
}: {
  token: string;
  file: Transfer;
  uploads: boolean;
  changed: () => void;
}) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  async function update(fields: object) {
    setBusy(true);
    setError("");
    try {
      await api(token, "commands", {
        type: "update_transfer_queue",
        direction: uploads ? "uploads" : "downloads",
        username: file.username,
        path: file.path,
        ...fields,
      });
      changed();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <details className="transfer-queue-controls">
      <summary>Priority &amp; queue</summary>
      <p>
        Priority:{" "}
        {(file.priority ?? 0) > 0
          ? "High"
          : (file.priority ?? 0) < 0
            ? "Low"
            : "Normal"}
        . Changes apply to waiting work.
      </p>
      <div
        className="button-row"
        role="group"
        aria-label={`Priority for ${file.path} from ${file.username}`}
      >
        {[
          [1, "High"],
          [0, "Normal"],
          [-1, "Low"],
        ].map(([value, label]) => (
          <button
            key={value}
            className="button small secondary"
            disabled={busy || (file.priority ?? 0) === value}
            onClick={() => void update({ priority: value })}
            aria-label={`Set ${label} priority for ${file.path} from ${file.username}`}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="button-row">
        {[
          ["top", "Move to top"],
          ["up", "Move up"],
          ["down", "Move down"],
          ["bottom", "Move to bottom"],
        ].map(([movement, label]) => (
          <button
            key={movement}
            className="button small secondary"
            disabled={busy}
            aria-label={`${label}: ${file.path} from ${file.username}`}
            onClick={() => void update({ movement })}
          >
            {label}
          </button>
        ))}
      </div>
      {error && <p role="alert">{error}</p>}
      <small>
        Moves apply within the same priority.{" "}
        {uploads
          ? "Soulseek privileges still take precedence. Accepted offers and active files keep their slots."
          : "Remote users control their own upload queues. These preferences choose the next local requests."}
      </small>
    </details>
  );
}
