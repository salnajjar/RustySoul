import { useEffect, useState, type FormEvent } from "react";
import type { Snapshot } from "./types";
import { api } from "./api";
import FolderBrowser from "./FolderBrowser";
import LibraryProgress from "./LibraryProgress";
import SystemMessage from "./SystemMessage";
import StorageReserve from "./StorageReserve";
export default function Locations({
  state,
  token,
}: {
  state: Snapshot;
  token: string;
}) {
  const [downloads, setDownloads] = useState(state.locations.downloads);
  const [shares, setShares] = useState(state.locations.shares.join("\n"));
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [indexStartup, setIndexStartup] = useState(
    state.library_settings.index_on_startup,
  );
  useEffect(
    () => setIndexStartup(state.library_settings.index_on_startup),
    [state.library_settings.index_on_startup],
  );
  const [browsing, setBrowsing] = useState<"downloads" | "shares" | null>(null);
  useEffect(() => {
    setDownloads(state.locations.downloads);
    setShares(state.locations.shares.join("\n"));
    setPending(false);
  }, [state.locations.downloads, state.locations.shares.join("\n")]);
  useEffect(() => {
    if (state.error) setPending(false);
  }, [state.error]);
  async function save(e: FormEvent) {
    e.preventDefault();
    setError("");
    setPending(true);
    try {
      await api(token, "commands", {
        type: "update_locations",
        downloads,
        shares: shares
          .split("\n")
          .map((p) => p.trim())
          .filter(Boolean),
      });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  }
  return (
    <section className="panel wide">
      {browsing && (
        <FolderBrowser
          token={token}
          title={
            browsing === "shares"
              ? "Add a shared library folder"
              : "Choose download folder"
          }
          onClose={() => setBrowsing(null)}
          onChoose={(path) => {
            if (browsing === "downloads") {
              setDownloads(path);
              return;
            }
            const paths = shares
              .split("\n")
              .filter(Boolean)
              .filter(
                (p) =>
                  p !== path &&
                  !path.startsWith(p + "/") &&
                  !p.startsWith(path + "/"),
              );
            setShares([...paths, path].join("\n"));
          }}
        />
      )}
      <h2>File locations & local library</h2>
      <form className="credentials-form" onSubmit={save}>
        <label>
          Download folder
          <input
            value={downloads}
            onChange={(e) => setDownloads(e.target.value)}
            required
            maxLength={4096}
          />
        </label>
        <button
          type="button"
          className="button small secondary"
          onClick={() => setBrowsing("downloads")}
        >
          Browse download folder
        </button>
        <label>
          Shared library folders — one per line
          <textarea
            aria-label="Shared library folders — one per line"
            value={shares}
            onChange={(e) => setShares(e.target.value)}
            rows={3}
            maxLength={8192}
          />
        </label>
        <button
          type="button"
          className="button small secondary"
          onClick={() => setBrowsing("shares")}
        >
          Browse and add library folder
        </button>
        <p className="hint">
          Installation mounts:{" "}
          {state.location_roots.length
            ? state.location_roots.join(", ")
            : "server paths configured at installation"}
          . Select these folders or subfolders within them. Downloads must be
          writable; shared folders can be mounted read-only. Only folders listed
          here are shared. Changing a destination does not move existing
          downloads.
        </p>
        <div className="button-row">
          <button className="button" disabled={pending}>
            {pending ? "Saving…" : "Save file locations"}
          </button>
          <button
            type="button"
            className="button secondary"
            disabled={state.library.scanning}
            onClick={() =>
              api(token, "commands", { type: "rescan_shares" }).catch((e) =>
                setError(e.message),
              )
            }
          >
            {state.library.scanning ? "Indexing…" : "Re-index local library"}
          </button>
        </div>
        {error && (
          <SystemMessage error onDismiss={() => setError("")}>
            {error}
          </SystemMessage>
        )}
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={indexStartup}
            onChange={(e) => {
              const value = e.target.checked;
              setIndexStartup(value);
              void api(token, "commands", {
                type: "update_library_settings",
                settings: { index_on_startup: value },
              }).catch((e) => {
                setIndexStartup(state.library_settings.index_on_startup);
                setError(e.message);
              });
            }}
          />
          Index the library when RustySoul starts
        </label>
        <p className="hint">
          Disabled by default. Startup loads the saved index; filesystem events
          and periodic reconciliation continue to detect changes. Manual
          re-indexing is always available.
        </p>
        <LibraryProgress status={state.library} />
        <p className="hint">
          {state.library.watcher || "Filesystem hooks are starting."} New and
          modified files are indexed automatically. Manual re-indexing checks
          metadata and skips unchanged files.
        </p>
      </form>
      <StorageReserve state={state} token={token} />
    </section>
  );
}
