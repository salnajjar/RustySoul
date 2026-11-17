import { useMemo, useState } from "react";
import { Download, FolderOpen, Users } from "lucide-react";
import type { FileResult } from "./types";
import { basename, bytes } from "./api";
import User from "./User";
import LoadMore from "./LoadMore";
import VirtualTable from "./VirtualTable";
import {
  useChoice,
  useSelection,
  fileIdentity,
  accepted,
} from "./WorkspaceState";
import {
  groupResults,
  type AudioFormat,
  type UserSort,
  type FileSort,
} from "./results";
export default function GroupedResults({
  files,
  filter = "",
  format = "all",
  send,
  online,
  capacity = 256,
  ordering,
  statistics,
  pageSelection = false,
  selectionScope = "results",
  infinite = false,
  selectFolder,
  selectable = true,
  showUserOrder = true,
}: {
  files: FileResult[];
  filter?: string;
  format?: AudioFormat;
  send: (c: object) => void;
  online: boolean;
  capacity?: number;
  notice?: string | null;
  ordering?: {
    user: UserSort;
    file: FileSort;
    setUser: (sort: UserSort) => void;
    setFile: (sort: FileSort) => void;
  };
  statistics?: Record<
    string,
    { count: number; folders: number; speed: number; slots: number }
  >;
  pageSelection?: boolean;
  selectionScope?: string;
  selectable?: boolean;
  showUserOrder?: boolean;
  infinite?: boolean;
  selectFolder?: (
    username: string,
    path: string,
    checked: boolean,
  ) => Promise<FileResult[] | null>;
}) {
  const [limits, setLimits] = useState<Record<string, number>>({});
  const [localUser, setLocalUser] = useChoice<UserSort>(
    "search.userOrder",
    "matches",
    ["matches", "speed", "name", "available"],
  );
  const userSort = ordering?.user ?? localUser;
  const setUserSort = ordering?.setUser ?? setLocalUser;
  const [localFile, setLocalFile] = useState<FileSort>("name");
  const fileSort = ordering?.file ?? localFile;
  const setFileSort = ordering?.setFile ?? setLocalFile;
  const [selected, setSelected] = useSelection(selectionScope);
  const identity = fileIdentity;
  const [submitting, setSubmitting] = useState(false);
  const [selectionError, setSelectionError] = useState("");
  const [selecting, setSelecting] = useState(false);
  const [pendingFolders, setPendingFolders] = useState<Map<string, boolean>>(
    new Map(),
  );
  async function folderSelection(
    username: string,
    path: string,
    files: FileResult[],
    checked: boolean,
  ) {
    if (!selectFolder) {
      select(files, checked);
      return;
    }
    setSelecting(true);
    const key = JSON.stringify([username, path]);
    setPendingFolders((old) => new Map(old).set(key, checked));
    setSelectionError("");
    try {
      const all = await selectFolder(username, path, checked);
      if (all) select(all, checked);
    } catch (e) {
      setSelectionError((e as Error).message);
    } finally {
      setSelecting(false);
      setPendingFolders((old) => {
        const next = new Map(old);
        next.delete(key);
        return next;
      });
    }
  }
  async function submit(files: FileResult[], single = false) {
    if (submitting || !files.length) return;
    setSubmitting(true);
    setSelectionError("");
    try {
      const ok = await accepted(
        send,
        single
          ? {
              type: "download",
              username: files[0].username,
              path: files[0].path,
              size: files[0].size,
            }
          : {
              type: "download_batch",
              files: files.map((f) => ({
                username: f.username,
                path: f.path,
                size: f.size,
              })),
            },
      );
      if (ok) select(files, false);
    } catch (e) {
      setSelectionError((e as Error).message);
    } finally {
      setSubmitting(false);
    }
  }
  function userToolbar(username: string, where: string) {
    if (!selectable) return null;
    const files = [...selected.values()].filter((f) => f.username === username);
    return (
      <div className="user-download-toolbar">
        <span>
          {files.length} selected ·{" "}
          {bytes(files.reduce((sum, f) => sum + f.size, 0))}
        </span>
        <button
          className="button small"
          aria-label={`Download selected from ${username} (${where})`}
          disabled={
            !online ||
            submitting ||
            !files.length ||
            files.length > Math.min(256, capacity)
          }
          onClick={() => void submit(files)}
        >
          Download selected
        </button>
      </div>
    );
  }
  function select(files: FileResult[], checked: boolean) {
    if (!selectable) return;
    setSelected((old) => {
      const next = new Map(old);
      for (const f of files) {
        if (checked) next.set(identity(f), f);
        else next.delete(identity(f));
      }
      return next;
    });
  }
  const groups = useMemo(() => {
    const groups = groupResults(files, filter, format, userSort, fileSort);
    if (statistics)
      groups.sort((a, b) => {
        const x = statistics[a.username],
          y = statistics[b.username];
        return (
          (userSort === "name"
            ? a.username.localeCompare(b.username)
            : userSort === "speed"
              ? (y?.speed ?? b.speed) - (x?.speed ?? a.speed)
              : userSort === "available"
                ? (y?.slots ?? Number(b.available)) -
                  (x?.slots ?? Number(a.available))
                : (y?.count ?? b.count) - (x?.count ?? a.count)) ||
          a.username.localeCompare(b.username)
        );
      });
    return groups;
  }, [files, filter, format, userSort, fileSort, statistics]);
  const total = groups.reduce((n, g) => n + g.count, 0);
  const matching = groups.flatMap((g) => g.folders.flatMap((f) => f.files));
  const hidden =
    selected.size - matching.filter((f) => selected.has(identity(f))).length;
  const displayed = groups.reduce(
    (n, user) =>
      n +
      user.folders.reduce(
        (sum, folder) =>
          sum +
          Math.min(
            folder.files.length,
            limits[JSON.stringify([user.username, folder.path])] || 100,
          ),
        0,
      ),
    0,
  );
  if (!total && !selected.size)
    return (
      <div className="empty">
        <h2>{files.length ? "No matching files" : "Nothing here yet"}</h2>
        <p>
          {files.length
            ? "Try a different format or filter."
            : "Waiting for results from Soulseek peers."}
        </p>
      </div>
    );
  return (
    <>
      <div className="results-controls">
        {showUserOrder && (
          <label>
            Order users
            <select
              aria-label="Order users"
              value={userSort}
              onChange={(e) => setUserSort(e.target.value as UserSort)}
            >
              <option value="matches">Most matches</option>
              <option value="speed">Fastest upload speed</option>
              <option value="name">Username A–Z</option>
              <option value="available">Available slot first</option>
            </select>
          </label>
        )}
        <label>
          Order files
          <select
            aria-label="Order files"
            value={fileSort}
            onChange={(e) => setFileSort(e.target.value as FileSort)}
          >
            <option value="name">Name A–Z</option>
            <option value="name_desc">Name Z–A</option>
            <option value="smallest">Smallest first</option>
            <option value="largest">Largest first</option>
            <option value="bitrate">Highest bitrate</option>
            <option value="duration">Shortest duration</option>
          </select>
        </label>
      </div>
      {selectable && (
        <div className="results-selection">
          <button
            className="button small secondary"
            onClick={() => select(matching, true)}
          >
            {infinite
              ? "Select loaded results"
              : pageSelection
                ? "Select this page"
                : "Select all matching"}{" "}
            ({total})
          </button>
          <button
            className="button small secondary"
            disabled={!selected.size}
            onClick={() => setSelected(new Map())}
          >
            Deselect all
          </button>
          <span>
            {selected.size} selected ·{" "}
            {bytes([...selected.values()].reduce((sum, f) => sum + f.size, 0))}
            {hidden > 0
              ? ` (${hidden} ${pageSelection ? "not on this page" : "hidden by filter"})`
              : ""}
          </span>
          <button
            className="button small"
            disabled={
              !online ||
              submitting ||
              selecting ||
              selected.size === 0 ||
              selected.size > Math.min(256, capacity)
            }
            onClick={() => void submit([...selected.values()])}
          >
            Download selected
          </button>
          {selected.size > capacity && (
            <small>
              Queue space: {capacity}. Select fewer files or remove completed
              transfers.
            </small>
          )}
        </div>
      )}
      {selectionError && <p role="alert">{selectionError}</p>}
      {selecting && <p role="status">Selecting folder files…</p>}
      <div className="grouped-results">
        {groups.map((user) => {
          const visible = user.folders.map((folder) => {
            const key = JSON.stringify([user.username, folder.path]);
            const rows = folder.files.slice(0, limits[key] || 100);
            return { ...folder, rows, key };
          });
          return (
            <details className="user-results" key={user.username} open>
              <summary>
                <input
                  type="checkbox"
                  aria-label={`Select results from ${user.username}`}
                  disabled={!selectable}
                  checked={user.folders.every((folder) =>
                    folder.files.every((f) => selected.has(identity(f))),
                  )}
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) =>
                    select(
                      user.folders.flatMap((folder) => folder.files),
                      e.target.checked,
                    )
                  }
                />
                <Users size={17} />
                <User username={user.username} />
                <span>
                  {(
                    statistics?.[user.username]?.count ?? user.count
                  ).toLocaleString()}{" "}
                  files ·{" "}
                  {statistics?.[user.username]?.folders ?? user.folders.length}{" "}
                  folders ·{" "}
                  <span title="Peer-reported upload speed, not a measured download rate">
                    {bytes(statistics?.[user.username]?.speed ?? user.speed)}/s
                    reported
                  </span>
                </span>
              </summary>
              {userToolbar(user.username, "top")}
              {visible.map((folder) => (
                <details className="folder-results" key={folder.path} open>
                  <summary>
                    <input
                      type="checkbox"
                      aria-label={`Select folder ${folder.path} from ${user.username}`}
                      disabled={!selectable || pendingFolders.has(folder.key)}
                      checked={
                        pendingFolders.get(folder.key) ??
                        folder.files.every((f) => selected.has(identity(f)))
                      }
                      onClick={(e) => e.stopPropagation()}
                      onChange={(e) =>
                        void folderSelection(
                          user.username,
                          folder.path,
                          folder.files,
                          e.target.checked,
                        )
                      }
                    />
                    <FolderOpen size={15} />
                    <span>{folder.path}</span>
                    <small>{folder.files.length} files</small>
                  </summary>
                  {folder.rows.length ? (
                    <VirtualTable
                      files={folder.rows}
                      header={
                        <thead>
                          <tr>
                            <th>File</th>
                            <th>Quality</th>
                            <th>Size</th>
                            <th>Availability</th>
                            <th>
                              <span className="sr-only">Download</span>
                            </th>
                          </tr>
                        </thead>
                      }
                      render={(f) => (
                        <tr key={JSON.stringify([f.username, f.path])}>
                          <td className="file-name">
                            <input
                              type="checkbox"
                              aria-label={`Select ${f.path} from ${f.username}`}
                              disabled={!selectable}
                              checked={selected.has(identity(f))}
                              onChange={(e) => select([f], e.target.checked)}
                            />
                            <span title={f.path}>
                              <strong>{basename(f.path)}</strong>
                            </span>
                          </td>
                          <td>
                            <span className="format-tag">
                              {f.path
                                .split(".")
                                .at(-1)
                                ?.toUpperCase()
                                .slice(0, 6)}
                            </span>
                            {f.attributes["0"] ? (
                              <small>{f.attributes["0"]} kbps</small>
                            ) : null}
                          </td>
                          <td className="mono">{bytes(f.size)}</td>
                          <td>
                            <span
                              className={`availability ${f.free_slots ? "free" : ""}`}
                            >
                              {f.free_slots
                                ? "Slot available"
                                : "Queue / unknown"}
                            </span>
                          </td>
                          <td>
                            <button
                              className="icon-button"
                              aria-label={`Download ${basename(f.path)}`}
                              disabled={!online || submitting || selecting}
                              onClick={() => void submit([f], true)}
                            >
                              <Download size={17} />
                            </button>
                          </td>
                        </tr>
                      )}
                    />
                  ) : (
                    <p className="hint">
                      Use “Show more” below to display these files.
                    </p>
                  )}
                  {folder.rows.length < folder.files.length && (
                    <LoadMore
                      trigger={folder.rows.length}
                      load={() =>
                        setLimits((previous) => ({
                          ...previous,
                          [folder.key]: (previous[folder.key] || 100) + 100,
                        }))
                      }
                    >
                      Show 100 more in this folder
                    </LoadMore>
                  )}
                </details>
              ))}
              {userToolbar(user.username, "bottom")}
            </details>
          );
        })}
      </div>
      <div className="table-bottom">
        <span>
          {displayed} of {total} files · selected user order updates as results
          arrive
        </span>
      </div>
    </>
  );
}
