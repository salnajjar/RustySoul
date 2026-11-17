import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import SystemMessage from "./SystemMessage";
interface Listing {
  path: string | null;
  parent: string | null;
  roots: string[];
  folders: { name: string; path: string }[];
  truncated: boolean;
}
export default function FolderBrowser({
  token,
  title,
  onChoose,
  onClose,
}: {
  token: string;
  title: string;
  onChoose: (path: string) => void;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function load(path?: string) {
    setBusy(true);
    setError("");
    try {
      setListing(
        await api<Listing>(
          token,
          "folders" + (path ? "?path=" + encodeURIComponent(path) : ""),
        ),
      );
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    dialog.current?.showModal();
    void load();
  }, []);
  return (
    <dialog
      ref={dialog}
      className="folder-dialog"
      aria-label={title}
      onCancel={onClose}
    >
      <h2>{title}</h2>
      <p className="hint">
        Browse the folders mounted into RustySoul. Additional host folders must
        be mapped when installing the container.
      </p>
      <div className="button-row">
        <button
          type="button"
          className="button small secondary"
          onClick={() => load()}
        >
          Storage mounts
        </button>
        {listing?.parent && (
          <button
            type="button"
            className="button small secondary"
            onClick={() => load(listing.parent!)}
          >
            Up one folder
          </button>
        )}
      </div>
      <p className="folder-current">
        {listing?.path || "Choose a mounted folder"}
      </p>
      {error && (
        <SystemMessage error onDismiss={() => setError("")}>
          {error}
        </SystemMessage>
      )}
      <div className="folder-choices" aria-busy={busy}>
        {busy ? (
          <p>Loading folders…</p>
        ) : (
          listing?.folders.map((f) => (
            <button type="button" key={f.path} onClick={() => load(f.path)}>
              <span>▸</span>
              {f.name}
            </button>
          ))
        )}
        {!busy && listing?.folders.length === 0 && (
          <p>No subfolders. You can select this folder.</p>
        )}
      </div>
      {listing?.truncated && (
        <p className="hint">Showing the first 1,000 subfolders.</p>
      )}
      <div className="button-row">
        <button
          type="button"
          className="button"
          disabled={!listing?.path || busy}
          onClick={() => {
            onChoose(listing!.path!);
            onClose();
          }}
        >
          Use this folder
        </button>
        <button type="button" className="button secondary" onClick={onClose}>
          Cancel
        </button>
      </div>
    </dialog>
  );
}
