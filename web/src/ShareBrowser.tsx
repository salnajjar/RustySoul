import { useEffect, useState } from "react";
import { FolderOpen, RefreshCw, X } from "lucide-react";
import type { FileResult, Snapshot } from "./types";
import { api } from "./api";
import GroupedResults from "./GroupedResults";
import User, { UserStatus } from "./User";
import FolderTree from "./FolderTree";
import type { FileSort } from "./results";
import { useSelection, fileIdentity, accepted } from "./WorkspaceState";
type Page = {
  files: FileResult[];
  folders: { path: string; files: number }[];
  total: number;
  offset: number;
};
const empty: Page = { files: [], folders: [], total: 0, offset: 0 };
export default function ShareBrowser({
  state,
  token,
  send,
  selected,
  onSelect,
  openUser,
}: {
  state: Snapshot;
  token: string;
  send: (command: object) => void;
  selected: string;
  onSelect: (username: string) => void;
  openUser: (username: string) => void;
}) {
  const [name, setName] = useState("");
  const [filter, setFilter] = useState("");
  const [folderFilter, setFolderFilter] = useState("");
  const [folder, setFolder] = useState<string | null>(null);
  const [offset, setOffset] = useState(0);
  const [recursive, setRecursive] = useState(true);
  const [fileSort, setFileSort] = useState<FileSort>("name");
  const [files, setFiles] = useState<Page>(empty);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [selection, setSelection] = useSelection("browse");
  const [selectionBusy, setSelectionBusy] = useState(false);
  const [selectionError, setSelectionError] = useState("");
  async function folderFiles(
    username: string,
    path: string,
    checked: boolean,
    includeChildren = false,
  ) {
    if (!checked)
      return [...selection.values()].filter(
        (f) =>
          f.username === username &&
          (f.path
            .replaceAll("/", "\\")
            .substring(0, f.path.replaceAll("/", "\\").lastIndexOf("\\")) ===
            path ||
            (includeChildren &&
              f.path.replaceAll("/", "\\").startsWith(path + "\\"))),
      );
    const chosen: FileResult[] = [];
    for (let offset = 0; ; offset += 100) {
      const q = new URLSearchParams({
        username,
        folder: path,
        recursive: String(includeChildren),
        filter: includeChildren ? "" : filter,
        offset: String(offset),
      });
      const page = await api<Page>(token, `browse/files?${q}`);
      if (page.total > 256)
        throw Error(
          `This folder has ${page.total.toLocaleString()} matching files. Select smaller branches or individual files; each download batch supports up to 256 files.`,
        );
      chosen.push(...page.files);
      if (offset + 100 >= page.total) break;
    }
    return chosen;
  }
  async function selectTreeFolder(path: string, checked: boolean) {
    setSelectionBusy(true);
    setSelectionError("");
    try {
      const files = await folderFiles(selected, path, checked, true);
      setSelection((old) => {
        const next = new Map(old);
        for (const file of files) {
          if (checked) next.set(fileIdentity(file), file);
          else next.delete(fileIdentity(file));
        }
        return next;
      });
    } catch (e) {
      setSelectionError((e as Error).message);
    } finally {
      setSelectionBusy(false);
    }
  }
  async function downloadCurrentFolder() {
    if (!folder) return;
    setSelectionBusy(true);
    setSelectionError("");
    try {
      const files = await folderFiles(selected, folder, true, recursive);
      if (
        await accepted(send, {
          type: "download_batch",
          files: files.map((f) => ({
            username: f.username,
            path: f.path,
            size: f.size,
          })),
        })
      )
        setSelection((old) => {
          const next = new Map(old);
          for (const file of files) next.delete(fileIdentity(file));
          return next;
        });
    } catch (e) {
      setSelectionError((e as Error).message);
    } finally {
      setSelectionBusy(false);
    }
  }
  const listing = state.browse[selected];
  const online = state.connection === "connected";
  const self = selected === state.username;
  useEffect(() => {
    setFolder(null);
    setOffset(0);
    setFilter("");
    setFolderFilter("");
    setFiles(empty);
  }, [selected]);
  useEffect(() => {
    if (!listing || listing.pending || !selected) return;
    const abort = new AbortController();
    setLoading(true);
    setError("");
    const params = new URLSearchParams({
      username: selected,
      filter,
      offset: String(offset),
      recursive: String(recursive),
      order: fileSort,
    });
    if (folder !== null) params.set("folder", folder);
    api<Page>(token, `browse/files?${params}`, undefined, abort.signal)
      .then(setFiles)
      .catch((e) => {
        if (!abort.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!abort.signal.aborted) setLoading(false);
      });
    return () => abort.abort();
  }, [
    selected,
    listing?.pending,
    listing?.revision,
    filter,
    folder,
    offset,
    token,
    recursive,
    fileSort,
  ]);
  const browse = (username: string) => {
    onSelect(username);
    send({ type: "browse", username });
  };
  const pager = (page: Page, change: (offset: number) => void) =>
    page.total > 100 && (
      <div className="button-row">
        <button
          className="button small secondary"
          disabled={page.offset === 0}
          onClick={() => change(Math.max(0, page.offset - 100))}
        >
          Previous
        </button>
        <span>
          {page.offset + 1}–{Math.min(page.total, page.offset + 100)} of{" "}
          {page.total.toLocaleString()}
        </span>
        <button
          className="button small secondary"
          disabled={page.offset + 100 >= page.total}
          onClick={() => change(page.offset + 100)}
        >
          Next
        </button>
      </div>
    );
  return (
    <>
      <div className="page-header">
        <div>
          <span className="eyebrow">PEOPLE & COLLECTIONS</span>
          <h1>Users & shared libraries</h1>
          <p>
            Look up an exact Soulseek username, view their profile, or explore
            shared files.
          </p>
        </div>
        {state.username && (
          <button
            className="button secondary"
            onClick={() => browse(state.username)}
          >
            <FolderOpen size={16} />
            Browse my shares
          </button>
        )}
      </div>
      <form
        className="search-form compact"
        onSubmit={(e) => {
          e.preventDefault();
          openUser(name.trim());
        }}
      >
        <input
          aria-label="Username to browse"
          placeholder="Exact Soulseek username"
          maxLength={30}
          required
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <button
          className="button"
          disabled={!name.trim() || (!online && name.trim() !== state.username)}
        >
          Find user
        </button>
        <button
          type="button"
          className="button secondary"
          disabled={!name.trim() || (!online && name.trim() !== state.username)}
          onClick={() => browse(name.trim())}
        >
          Browse library
        </button>
      </form>
      {!online && (
        <p className="hint">
          Connect to Soulseek to look up other users. Your cached shares remain
          available.
        </p>
      )}
      <section className="panel buddy-panel">
        <h2>
          Buddies <small>{state.buddies.length}/128</small>
        </h2>
        {state.buddies.length ? (
          <div className="buddy-list">
            {state.buddies.map((u) => (
              <User key={u} username={u} />
            ))}
          </div>
        ) : (
          <p className="hint">
            Click any username and choose Add buddy to keep them here across
            restarts.
          </p>
        )}
      </section>
      {selectionError && <p role="alert">{selectionError}</p>}
      {selectionBusy && <p role="status">Selecting shared folder…</p>}
      <div className="browse-tabs">
        {Object.keys(state.browse).map((u) => (
          <div className="browse-tab" key={u}>
            <button
              className={`button small ${selected === u ? "" : "secondary"}`}
              onClick={() => onSelect(u)}
            >
              {u}
              <UserStatus username={u} />
            </button>
            <button
              className="icon-button"
              aria-label={`Close library for ${u}`}
              onClick={() => {
                send({ type: "close_browse", username: u });
                if (selected === u) onSelect("");
              }}
            >
              <X size={15} />
            </button>
          </div>
        ))}
      </div>
      {listing && (
        <section className="panel wide">
          <div className="results-toolbar">
            <div>
              <User username={selected} />
              <p>
                {listing.pending
                  ? "Requesting shares; trying direct and reverse connections…"
                  : `${(listing.total ?? listing.files.length).toLocaleString()} shared files${listing.truncated ? " · listing reached safety limit" : ""}`}
              </p>
            </div>
            <button
              className="button secondary small"
              disabled={listing.pending || (!online && !self)}
              onClick={() => browse(selected)}
            >
              <RefreshCw size={15} />
              Refresh library
            </button>
          </div>
          {listing.error && <p role="alert">{listing.error}</p>}
          {error && <p role="alert">{error}</p>}
          {!listing.pending && (
            <div className="share-layout">
              <aside className="share-folders">
                <h3>Folders</h3>
                <input
                  aria-label="Filter share folders"
                  placeholder="Find a folder…"
                  value={folderFilter}
                  onChange={(e) => {
                    setFolderFilter(e.target.value);
                  }}
                />
                <FolderTree
                  selectedFiles={[...selection.values()].filter(
                    (f) => f.username === selected,
                  )}
                  selectFiles={self ? undefined : selectTreeFolder}
                  token={token}
                  username={selected}
                  revision={listing.revision || 0}
                  filter={folderFilter}
                  selected={folder}
                  select={(path) => {
                    setFolder(path);
                    setOffset(0);
                  }}
                />
              </aside>
              <div className="share-files">
                <label>
                  Filter files
                  <input
                    aria-label="Filter library"
                    placeholder="Find a file…"
                    value={filter}
                    onChange={(e) => {
                      setFilter(e.target.value);
                      setOffset(0);
                    }}
                  />
                </label>
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    checked={recursive}
                    onChange={(e) => {
                      setRecursive(e.target.checked);
                      setOffset(0);
                    }}
                  />
                  Include subfolders
                </label>
                {folder && (
                  <button
                    className="button secondary small"
                    disabled={
                      !online || self || listing.truncated || listing.pending
                    }
                    onClick={() => void downloadCurrentFolder()}
                  >
                    Download {recursive ? "folder & subfolders" : "this folder"}
                  </button>
                )}
                <p className="hint">
                  {folder || "All folders"} · {files.total.toLocaleString()}{" "}
                  matching files{self ? " · Your own shared files" : ""}
                </p>
                {loading && <p role="status">Loading files…</p>}
                {
                  <GroupedResults
                    selectionScope={self ? "own-browse" : "browse"}
                    selectable={!self}
                    showUserOrder={false}
                    selectFolder={folderFiles}
                    key={`${selected}:${listing.revision}`}
                    files={loading ? [] : files.files}
                    pageSelection
                    ordering={{
                      user: "name",
                      file: fileSort,
                      setUser: () => {},
                      setFile: (s) => {
                        setFileSort(s);
                        setOffset(0);
                      },
                    }}
                    send={send}
                    online={online && !self}
                    capacity={Math.max(
                      0,
                      state.transfer_capacity -
                        (state.download_capacity_used ??
                          state.downloads.length),
                    )}
                  />
                }
                {pager(files, setOffset)}
              </div>
            </div>
          )}
        </section>
      )}
    </>
  );
}
