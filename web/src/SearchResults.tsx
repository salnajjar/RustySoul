import { useEffect, useState, useRef } from "react";
import { api } from "./api";
import type { FileResult, Search } from "./types";
import GroupedResults from "./GroupedResults";
import LoadMore from "./LoadMore";
import { useChoice, fileIdentity } from "./WorkspaceState";
import type { AudioFormat, FileSort, UserSort } from "./results";
type Page = {
  files: FileResult[];
  total: number;
  offset: number;
  users: Record<
    string,
    { count: number; folders: number; speed: number; slots: number }
  >;
};
export default function SearchResults({
  search,
  token,
  filter,
  format,
  send,
  online,
  capacity,
}: {
  search: Search;
  token: string;
  filter: string;
  format: AudioFormat;
  send: (command: object) => void;
  online: boolean;
  capacity: number;
}) {
  const [userSort, setUserSort] = useChoice<UserSort>(
    "search.userOrder",
    "matches",
    ["matches", "speed", "name", "available"],
  );
  const [fileSort, setFileSort] = useChoice<FileSort>(
    "search.fileOrder",
    "name",
    ["name", "name_desc", "smallest", "largest", "bitrate", "duration"],
  );
  const [exclude, setExclude] = useState("");
  const key = JSON.stringify([
    search.id,
    search.query,
    filter,
    format,
    exclude,
    userSort,
    fileSort,
  ]);
  const [window, setWindow] = useState({ key, count: 100 });
  const [data, setData] = useState<Page>({
    files: [],
    total: 0,
    offset: 0,
    users: {},
  });
  const [loadedKey, setLoadedKey] = useState(key),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(false);
  const cache = useRef<{
    key: string;
    stamp: string;
    next: number;
    page: Page;
  } | null>(null);
  const [retry, setRetry] = useState(0);
  const paged = search.total !== undefined;
  useEffect(() => {
    setWindow({ key, count: 100 });
  }, [key]);
  useEffect(() => {
    if (!paged || window.key !== key) return;
    const abort = new AbortController();
    setLoading(true);
    setError("");
    const timer = setTimeout(() => {
      void (async () => {
        const stamp = `${search.revision}:${search.total}`;
        const previous = cache.current;
        const incremental = previous?.key === key && previous.stamp === stamp;
        let latest: Page = incremental
          ? previous.page
          : { files: [], total: 0, offset: 0, users: {} };
        const files = new Map<string, FileResult>(
          incremental
            ? previous.page.files.map((f) => [fileIdentity(f), f])
            : [],
        );
        let next = incremental ? previous.next : 0;
        // Append on scroll; refresh the loaded window only after live reordering.
        for (let offset = next; offset < window.count; offset += 100) {
          const q = new URLSearchParams({
            offset: String(offset),
            filter,
            exclude,
            format,
            user_sort: userSort,
            file_sort: fileSort,
          });
          latest = await api<Page>(
            token,
            `searches/${encodeURIComponent(search.id)}/results?${q}`,
            undefined,
            abort.signal,
          );
          next = offset + 100;
          for (const file of latest.files) files.set(fileIdentity(file), file);
          if (offset + 100 >= latest.total || !latest.files.length) break;
        }
        if (!abort.signal.aborted) {
          const page = { ...latest, offset: 0, files: [...files.values()] };
          cache.current = { key, stamp, next, page };
          setData(page);
          setLoadedKey(key);
        }
      })()
        .catch((e) => {
          if (!abort.signal.aborted) setError(e.message);
        })
        .finally(() => {
          if (!abort.signal.aborted) setLoading(false);
        });
    }, 120);
    return () => {
      clearTimeout(timer);
      abort.abort();
    };
  }, [
    paged,
    retry,
    key,
    window.count,
    window.key,
    search.id,
    search.revision,
    search.total,
    token,
  ]);
  const preview = search.results.filter(
    (f) =>
      !exclude
        .toLowerCase()
        .split(/\s+/)
        .filter(Boolean)
        .some((term) => `${f.username} ${f.path}`.toLowerCase().includes(term)),
  );
  const current = loadedKey === key ? data.files : [];
  return (
    <>
      <label className="exclude-filter">
        Exclude terms
        <input
          aria-label="Exclude result terms"
          placeholder="Words to leave out…"
          value={exclude}
          maxLength={256}
          onChange={(e) => setExclude(e.target.value)}
        />
      </label>
      {error && (
        <p role="alert">
          {error}{" "}
          <button className="text-link" onClick={() => setRetry((r) => r + 1)}>
            Retry loading results
          </button>
        </p>
      )}
      {search.truncated && (
        <p className="hint">
          Collection stopped at the configured resource limit. Narrow your query
          or increase limits.search_results in the configuration.
        </p>
      )}
      {paged && (
        <div className="result-pagination">
          <span>
            {data.total.toLocaleString()} matching files ·{" "}
            {current.length.toLocaleString()} loaded
            {loading ? " · Updating…" : ""}
          </span>
          <small>
            More results load as you scroll. User order updates with incoming
            results.
          </small>
        </div>
      )}
      <GroupedResults
        selectionScope={`search:${search.id}`}
        files={paged ? current : preview}
        filter={paged ? "" : filter}
        format={paged ? "all" : format}
        send={send}
        online={online}
        capacity={capacity}
        ordering={{
          user: userSort,
          file: fileSort,
          setUser: setUserSort,
          setFile: setFileSort,
        }}
        statistics={paged ? data.users : undefined}
        pageSelection={paged}
        infinite={paged}
      />
      {paged && current.length < data.total && (
        <LoadMore
          loading={loading || !!error}
          trigger={`${key}:${current.length}`}
          load={() =>
            setWindow((w) => ({
              key,
              count: w.key === key ? w.count + 100 : 100,
            }))
          }
        >
          Load more results
        </LoadMore>
      )}
    </>
  );
}
