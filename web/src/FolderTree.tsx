import { useEffect, useState, useRef } from "react";
import { ChevronRight, ChevronDown, FolderOpen } from "lucide-react";
import { api } from "./api";
import type { FileResult } from "./types";
import LoadMore from "./LoadMore";
type Folder = {
  path: string;
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
    setAllExpanded(false);
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
    <div className="folder-tree">
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
          toggle={toggle}
          pendingSelection={pendingSelection}
          chooseFiles={chooseFiles}
        />
      </ul>
    </div>
  );
}
function Children(
  props: TreeProps & {
    parent: string;
    isExpanded: (path: string) => boolean;
    allExpanded: boolean;
    pendingSelection: Map<string, boolean>;
    chooseFiles: (path: string, checked: boolean) => Promise<void>;
    toggle: (path: string) => void;
  },
) {
  const marker = useRef<HTMLLIElement>(null);
  const [ready, setReady] = useState(!props.allExpanded || !props.parent);
  useEffect(() => {
    if (ready) return;
    if (!props.allExpanded || typeof IntersectionObserver === "undefined") {
      setReady(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setReady(true);
          observer.disconnect();
        }
      },
      { rootMargin: "300px" },
    );
    if (marker.current) observer.observe(marker.current);
    return () => observer.disconnect();
  }, [ready, props.allExpanded]);
  const [data, setData] = useState<Page>({ folders: [], offset: 0, total: 0 });
  const [offset, setOffset] = useState(0);
  const [retry, setRetry] = useState(0);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    setOffset(0);
    setData({ folders: [], total: 0, offset: 0 });
  }, [props.username, props.parent, props.filter, props.revision]);
  useEffect(() => {
    if (!ready) return;
    const abort = new AbortController();
    setLoading(true);
    setError("");
    const q = new URLSearchParams({
      username: props.username,
      parent: props.parent,
      filter: props.filter,
      offset: String(offset),
    });
    api<Page>(props.token, `browse/tree?${q}`, undefined, abort.signal)
      .then((page) => {
        if (!abort.signal.aborted)
          setData((old) => ({
            ...page,
            folders: offset ? [...old.folders, ...page.folders] : page.folders,
          }));
      })
      .catch((e) => {
        if (!abort.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!abort.signal.aborted) setLoading(false);
      });
    return () => abort.abort();
  }, [
    ready,
    props.token,
    props.username,
    props.parent,
    props.filter,
    props.revision,
    offset,
    retry,
  ]);
  if (!ready)
    return (
      <li role="none" ref={marker}>
        <span className="hint">Folders load as you scroll…</span>
      </li>
    );
  return (
    <>
      {data.folders.map((folder) => (
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
      {loading && (
        <li role="none">
          <span role="status" className="hint">
            Loading folders…
          </span>
        </li>
      )}
      {error && (
        <li role="none">
          <p role="alert">{error}</p>
          <button className="text-link" onClick={() => setRetry((n) => n + 1)}>
            Retry
          </button>
        </li>
      )}
      {!loading && data.folders.length < data.total && (
        <li role="none">
          {props.allExpanded ? (
            <LoadMore
              loading={loading}
              trigger={data.folders.length}
              load={() => setOffset((n) => n + 100)}
            >
              Load more folders
            </LoadMore>
          ) : (
            <button
              className="button small secondary"
              onClick={() => setOffset((n) => n + 100)}
            >
              Load more folders ({data.folders.length}/
              {data.total.toLocaleString()})
            </button>
          )}
        </li>
      )}
    </>
  );
}
