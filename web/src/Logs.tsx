import { useEffect, useState } from "react";
import { api } from "./api";
type Entry = {
  id: number;
  at: number;
  level: string;
  source: string;
  message: string;
};
type Page = { entries: Entry[]; more: boolean };
export default function Logs({ token }: { token: string }) {
  const [level, setLevel] = useState("all"),
    [source, setSource] = useState("all"),
    [filter, setFilter] = useState(""),
    [before, setBefore] = useState(0),
    [paused, setPaused] = useState(false),
    [revision, setRevision] = useState(0);
  const [data, setData] = useState<Page>({ entries: [], more: false }),
    [error, setError] = useState("");
  const [cursors, setCursors] = useState<number[]>([]);
  useEffect(() => {
    const abort = new AbortController();
    const load = () =>
      api<Page>(
        token,
        `logs?${new URLSearchParams({ level, source, filter, before: String(before) })}`,
        undefined,
        abort.signal,
      )
        .then(setData)
        .catch((e) => {
          if (!abort.signal.aborted) setError(e.message);
        });
    setError("");
    void load();
    const timer =
      !paused && !before ? setInterval(() => void load(), 5000) : undefined;
    return () => {
      abort.abort();
      clearInterval(timer);
    };
  }, [token, level, source, filter, before, paused, revision]);
  function reset() {
    setBefore(0);
    setCursors([]);
  }
  return (
    <section className="panel wide logs-panel">
      <h2>Logs & activity</h2>
      <p>
        Daemon and network logs, system messages and workspace activity. New
        entries refresh every five seconds. History keeps up to 10,000 entries
        for seven days; passwords and command contents are excluded.
      </p>
      <div className="logs-controls">
        <label>
          Level
          <select
            aria-label="Log level"
            value={level}
            onChange={(e) => {
              setLevel(e.target.value);
              reset();
            }}
          >
            {["all", "error", "warn", "info", "debug", "trace"].map((v) => (
              <option key={v} value={v}>
                {v === "all" ? "All levels" : v.toUpperCase()}
              </option>
            ))}
          </select>
        </label>
        <label>
          Source
          <select
            aria-label="Log source"
            value={source}
            onChange={(e) => {
              setSource(e.target.value);
              reset();
            }}
          >
            {[
              "all",
              "daemon",
              "protocol",
              "system",
              "activity",
              "deployment",
            ].map((v) => (
              <option key={v} value={v}>
                {v === "all" ? "All sources" : v}
              </option>
            ))}
          </select>
        </label>
        <label>
          Find in logs
          <input
            aria-label="Filter logs"
            maxLength={256}
            value={filter}
            onChange={(e) => {
              setFilter(e.target.value);
              reset();
            }}
          />
        </label>
        <button
          className="button small secondary"
          onClick={() => setPaused((p) => !p)}
        >
          {paused ? "Resume live logs" : "Pause live logs"}
        </button>
        <button
          className="button small secondary"
          onClick={() => {
            reset();
            setRevision((r) => r + 1);
          }}
        >
          Latest logs
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
      <div
        className="operational-log"
        role="log"
        aria-label="Operational history"
        aria-live={paused ? "off" : "polite"}
      >
        {data.entries.length ? (
          data.entries.map((e) => (
            <article className={`log-entry log-${e.level}`} key={e.id}>
              <div>
                <time dateTime={new Date(e.at * 1000).toISOString()}>
                  {new Date(e.at * 1000).toLocaleString()}
                </time>
                <strong>{e.level.toUpperCase()}</strong>
                <span>{e.source}</span>
              </div>
              <pre>{e.message}</pre>
            </article>
          ))
        ) : (
          <p className="hint">No entries match these filters.</p>
        )}
      </div>
      <div className="button-row">
        <button
          className="button small secondary"
          disabled={!before}
          onClick={() => {
            setBefore(cursors.at(-1) || 0);
            setCursors((c) => c.slice(0, -1));
          }}
        >
          Newer logs
        </button>
        <button
          className="button small secondary"
          disabled={!data.more}
          onClick={() => {
            setCursors((c) => [...c, before]);
            setBefore(data.entries.at(-1)?.id || 0);
          }}
        >
          Older logs
        </button>
      </div>
    </section>
  );
}
