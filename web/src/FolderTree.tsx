import { useEffect, useState, useMemo } from "react";
import { ChevronRight, ChevronDown, FolderOpen } from "lucide-react";
import { api } from "./api";
import type { FileResult } from "./types";
type Folder = {
  path: string;
  parent?: string;
  name: string;
  files: number;
  direct_files: number;
  has_children: boolean;
  subfolders?: number;
  direct_folders?: number;
};
type Page = { folders: Folder[]; total: number; offset: number };
type TreeProps = {
  token: string;
  username: string;
  revision: number;
  filter: string;
  selected: string | null;
  select: (path: string | null) => void;
  selectedFiles?: FileResult[];
  selectFiles?: (path: string, checked: boolean) => void;
};
export default function FolderTree(props: TreeProps) {
  const key = JSON.stringify([props.username, props.revision]);
  const [retry, setRetry] = useState(0);
  const [data, setData] = useState<{
    key: string;
    folders: Folder[];
    loading: boolean;
    error: string;
  }>({ key: "", folders: [], loading: true, error: "" });
  useEffect(() => {
    const abort = new AbortController();
    setData({ key, folders: [], loading: true, error: "" });
    const q = new URLSearchParams({ username: props.username, all: "true" });
    api<Page>(props.token, `browse/tree?${q}`, undefined, abort.signal)
      .then((page) => {
        if (abort.signal.aborted) return;
        if (page.folders.length !== page.total)
          throw Error(
            "The complete folder tree was not returned. Refresh the library and retry.",
          );
        setData({ key, folders: page.folders, loading: false, error: "" });
      })
      .catch((e) => {
        if (!abort.signal.aborted)
          setData({ key, folders: [], loading: false, error: e.message });
      });
    return () => abort.abort();
  }, [key, props.username, props.token, retry]);
  const foldersByParent = useMemo(() => {
    const folders = data.key === key ? data.folders : [];
    const parent = (folder: Folder) =>
      folder.parent ??
      (folder.path.includes("\\")
        ? folder.path.slice(0, folder.path.lastIndexOf("\\"))
        : "");
    const byPath = new Map(folders.map((folder) => [folder.path, folder]));
    const query = props.filter.trim().toLowerCase();
    const visible = new Set<string>();
    if (query)
      for (const folder of folders) {
        if (!folder.path.toLowerCase().includes(query)) continue;
        let current: Folder | undefined = folder;
        while (current && !visible.has(current.path)) {
          visible.add(current.path);
          current = byPath.get(parent(current));
        }
      }
    const result = new Map<string, Folder[]>();
    for (const folder of folders) {
      if (query && !visible.has(folder.path)) continue;
      const name = parent(folder);
      const siblings = result.get(name) ?? [];
      siblings.push(folder);
      result.set(name, siblings);
    }
    return result;
  }, [data.key, data.folders, key, props.filter]);

  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [allExpanded, setAllExpanded] = useState(false);
  const [pendingSelection, setPendingSelection] = useState<
    Map<string, boolean>
  >(new Map());
  async function chooseFiles(path: string, checked: boolean) {
    setPendingSelection((old) => new Map(old).set(path, checked));
    try {
      await props.selectFiles?.(path, checked);
    } finally {
      setPendingSelection((old) => {
        const next = new Map(old);
        next.delete(path);
        return next;
      });
    }
  }
  useEffect(() => {
    setExpanded(new Set());
    setAllExpanded(!!props.filter.trim());
  }, [props.username, props.filter]);
  const isExpanded = (path: string) =>
    allExpanded ? !expanded.has(path) : expanded.has(path);
  const toggle = (path: string) =>
    setExpanded((old) => {
      const next = new Set(old);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  return (
    <div className="folder-tree" aria-busy={data.key !== key || data.loading}>
      <div className="button-row">
        <button
          className="button small secondary"
          onClick={() => {
            setAllExpanded(!(allExpanded || expanded.size > 0));
            setExpanded(new Set());
          }}
        >
          {allExpanded || expanded.size > 0
            ? "Collapse all folders"
            : "Expand all folders"}
        </button>
      </div>
      <button
        className={`tree-all ${props.selected === null ? "chosen" : ""}`}
        onClick={() => props.select(null)}
      >
        All shared files
      </button>
      <ul
        role="tree"
        aria-label="Shared folder tree"
        onKeyDown={(e) => {
          const target = e.target as HTMLElement;
          if (!target.dataset.treePath) return;
          const nodes = Array.from(
            e.currentTarget.querySelectorAll<HTMLButtonElement>(
              "button[data-tree-path]",
            ),
          );
          const index = nodes.indexOf(target as HTMLButtonElement);
          let next: HTMLButtonElement | undefined;
          if (e.key === "ArrowDown") next = nodes[index + 1];
          else if (e.key === "ArrowUp") next = nodes[index - 1];
          else if (e.key === "Home") next = nodes[0];
          else if (e.key === "End") next = nodes.at(-1);
          else if (e.key === "ArrowRight") {
            if (
              target.dataset.children === "true" &&
              !isExpanded(target.dataset.treePath)
            )
              toggle(target.dataset.treePath);
            else next = nodes[index + 1];
          } else if (e.key === "ArrowLeft") {
            if (isExpanded(target.dataset.treePath))
              toggle(target.dataset.treePath);
            else
              next = nodes.find(
                (n) => n.dataset.treePath === target.dataset.parent,
              );
          } else return;
          e.preventDefault();
          next?.focus();
        }}
      >
        <Children
          {...props}
          parent=""
          isExpanded={isExpanded}
          allExpanded={allExpanded}
          foldersByParent={foldersByParent}
          toggle={toggle}
          pendingSelection={pendingSelection}
          chooseFiles={chooseFiles}
        />
      </ul>
      {(data.key !== key || data.loading) && (
        <p role="status" className="hint">
          Loading all folders…
        </p>
      )}
      {data.key === key && data.error && (
        <>
          <p role="alert">{data.error}</p>
          <button className="text-link" onClick={() => setRetry((n) => n + 1)}>
            Retry
          </button>
        </>
      )}
      {data.key === key && !data.loading && !data.error && (
        <p className="hint">
          {data.folders.length.toLocaleString()} folders loaded
        </p>
      )}
    </div>
  );
}
function Children(
  props: TreeProps & {
    parent: string;
    foldersByParent: Map<string, Folder[]>;
    isExpanded: (path: string) => boolean;
    allExpanded: boolean;
    pendingSelection: Map<string, boolean>;
    chooseFiles: (path: string, checked: boolean) => Promise<void>;
    toggle: (path: string) => void;
  },
) {
  return (
    <>
      {(props.foldersByParent.get(props.parent) ?? []).map((folder) => (
        <li
          role="treeitem"
          key={folder.path}
          aria-expanded={
            folder.has_children ? props.isExpanded(folder.path) : undefined
          }
          aria-selected={props.selected === folder.path}
        >
          <div
            className={`tree-row ${props.selected === folder.path ? "chosen" : ""}`}
          >
            {folder.has_children ? (
              <button
                type="button"
                className="tree-toggle"
                aria-label={`${props.isExpanded(folder.path) ? "Collapse" : "Expand"} folder ${folder.path}`}
                onClick={() => props.toggle(folder.path)}
              >
                {props.isExpanded(folder.path) ? (
                  <ChevronDown size={14} />
                ) : (
                  <ChevronRight size={14} />
                )}
              </button>
            ) : (
              <span className="tree-toggle-space" />
            )}
            {props.selectFiles && (
              <input
                type="checkbox"
                aria-label={`Select shared folder ${folder.path}`}
                disabled={props.pendingSelection.has(folder.path)}
                checked={
                  props.pendingSelection.get(folder.path) ??
                  (folder.files > 0 &&
                    (props.selectedFiles || []).filter((f) =>
                      f.path
                        .replaceAll("/", "\\")
                        .startsWith(folder.path + "\\"),
                    ).length === folder.files)
                }
                onChange={(e) =>
                  void props.chooseFiles(folder.path, e.target.checked)
                }
              />
            )}
            <button
              type="button"
              className="tree-select"
              title={folder.path}
              data-tree-path={folder.path}
              data-parent={props.parent}
              data-children={folder.has_children}
              onClick={() => props.select(folder.path)}
            >
              <FolderOpen size={14} />
              <span>
                {folder.name}
                <small>
                  {folder.files.toLocaleString()} files ·{" "}
                  {folder.subfolders?.toLocaleString() ??
                    (folder.has_children ? "…" : "0")}{" "}
                  subfolders
                </small>
              </span>
            </button>
          </div>
          {folder.has_children && props.isExpanded(folder.path) && (
            <ul role="group">
              <Children {...props} parent={folder.path} />
            </ul>
          )}
        </li>
      ))}
    </>
  );
}
