import { useEffect, useState, type FormEvent } from "react";
import type { Snapshot } from "./types";
import { api, bytes } from "./api";
import SystemMessage from "./SystemMessage";
export default function StorageReserve({
  state,
  token,
}: {
  state: Snapshot;
  token: string;
}) {
  const saved = state.network.minimum_free_space_mib ?? 1024;
  const [gib, setGib] = useState(String(saved / 1024)),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  useEffect(() => setGib(String(saved / 1024)), [saved]);
  const n = Number(gib),
    valid = gib.trim() !== "" && Number.isFinite(n) && n >= 0 && n <= 1_000_000;
  const available =
    state.system?.downloads.path === state.locations.downloads
      ? state.system.downloads.available_bytes
      : null;
  async function save(e: FormEvent) {
    e.preventDefault();
    if (!valid) return;
    setBusy(true);
    setError("");
    try {
      await api(token, "commands", {
        type: "update_storage_reserve",
        minimum_free_space_mib: Math.round(n * 1024),
      });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <form className="storage-reserve" onSubmit={save}>
      <h3>Download storage protection</h3>
      <p>
        Keep a free-space reserve on download filesystems. Downloads wait when
        storage is low, keep their partial files and resume automatically when
        space is available. Storage holds do not use network retry attempts.
      </p>
      <label>
        Keep free (GiB)
        <input
          aria-label="Download free-space reserve (GiB)"
          type="number"
          min={0}
          max={1000000}
          step="any"
          value={gib}
          onChange={(e) => setGib(e.target.value)}
          disabled={busy}
        />
      </label>
      <p className="hint">
        Default: 1 GiB. Set 0 to disable. This is a free-space floor, rather
        than a reservation for the entire queue; other applications can also
        consume disk space.
      </p>
      {available != null && (
        <p>
          Last measured at the current destination:{" "}
          <strong>{bytes(available)}</strong> available.
        </p>
      )}
      {valid && available != null && n * 1024 ** 3 > available && (
        <p role="status">
          This reserve exceeds the last measured free space. Downloads will wait
          until more space is available or the reserve is lowered.
        </p>
      )}
      <button className="button secondary" disabled={busy || !valid}>
        {busy ? "Saving…" : "Save storage protection"}
      </button>
      {saved === 0 && (
        <p role="status">Download storage protection is disabled.</p>
      )}
      {error && (
        <SystemMessage error onDismiss={() => setError("")}>
          {error}
        </SystemMessage>
      )}
    </form>
  );
}
