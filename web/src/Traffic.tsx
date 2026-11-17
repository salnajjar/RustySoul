import { useEffect, useMemo, useState } from "react";
import { api, bytes } from "./api";
type Totals = { downloaded: number; uploaded: number; since: number };
type Point = { start: number; downloaded: number; uploaded: number };
type History = {
  totals: Totals;
  from: number;
  to: number;
  interval: number;
  points: Point[];
};
const date = (n: number) => new Date(n * 1000).toISOString().slice(0, 10);
export default function Traffic({
  token,
  totals,
}: {
  token: string;
  totals: Totals;
}) {
  const [showDownloads, setShowDownloads] = useState(true);
  const [showUploads, setShowUploads] = useState(true);
  const [range, setRange] = useState("week");
  const [custom, setCustom] = useState({
    from: date(totals.since),
    to: date(Date.now() / 1000),
  });
  const [shift, setShift] = useState(0);
  const [data, setData] = useState<History>();
  const [error, setError] = useState("");
  const [selected, setSelected] = useState(0);
  // Refresh on five-second persisted accounting updates, with bounded graph responses.
  useEffect(() => {
    const abort = new AbortController();
    const now = Math.floor(Date.now() / 1000) + 1;
    const duration = (
      {
        day: 86400,
        week: 7 * 86400,
        month: 30 * 86400,
        year: 365 * 86400,
      } as Record<string, number>
    )[range];
    const to =
      range === "custom"
        ? Date.parse(custom.to + "T00:00:00Z") / 1000 + 86400
        : duration
          ? now - shift * duration
          : now;
    const from =
      range === "custom"
        ? Date.parse(custom.from + "T00:00:00Z") / 1000
        : duration
          ? Math.max(totals.since, to - duration)
          : totals.since;
    if (!Number.isFinite(from) || !Number.isFinite(to) || from >= to) {
      setError("Choose an end date on or after the start date.");
      return;
    }
    setError("");
    api<History>(
      token,
      `traffic?${new URLSearchParams({ from: String(Math.floor(from)), to: String(Math.floor(to)) })}`,
      undefined,
      abort.signal,
    )
      .then((h) => {
        setData(h);
      })
      .catch((e) => {
        if (!abort.signal.aborted) setError(e.message);
      });
    return () => abort.abort();
  }, [
    token,
    range,
    shift,
    custom.from,
    custom.to,
    totals.downloaded,
    totals.uploaded,
    totals.since,
  ]);
  const points = useMemo(() => {
    if (!data) return [];
    const map = new Map(data.points.map((p) => [p.start, p]));
    const result: Point[] = [];
    for (
      let start = Math.floor(data.from / data.interval) * data.interval;
      start < data.to && result.length < 722;
      start += data.interval
    )
      result.push(map.get(start) ?? { start, downloaded: 0, uploaded: 0 });
    return result;
  }, [data]);
  const maximum = Math.max(
    1,
    ...points.flatMap((p) => [
      ...(showUploads ? [p.uploaded] : []),
      ...(showDownloads ? [p.downloaded] : []),
    ]),
  );
  const path = (key: "uploaded" | "downloaded") =>
    points
      .map(
        (p, i) =>
          `${i ? "L" : "M"}${40 + (i / Math.max(1, points.length - 1)) * 900},${220 - (p[key] / maximum) * 180}`,
      )
      .join(" ");
  const point = points[Math.min(selected, points.length - 1)];
  return (
    <section
      className="traffic-panel results-panel"
      aria-label="Bandwidth statistics"
    >
      <h2>Bandwidth & sharing ratio</h2>
      <div className="traffic-totals">
        <div>
          <small>All-time uploaded</small>
          <strong>{bytes(totals.uploaded)}</strong>
        </div>
        <div>
          <small>All-time downloaded</small>
          <strong>{bytes(totals.downloaded)}</strong>
        </div>
        <div>
          <small>Upload / download ratio</small>
          <strong>
            {totals.downloaded
              ? (totals.uploaded / totals.downloaded).toFixed(2)
              : totals.uploaded
                ? "∞"
                : "—"}
          </strong>
        </div>
      </div>
      <p className="hint">
        File data recorded since{" "}
        {new Date(totals.since * 1000).toLocaleString()}. Includes retries;
        excludes protocol overhead. Clearing transfer history keeps these
        totals.
      </p>
      <div className="traffic-controls">
        <label>
          Graph range{" "}
          <select
            aria-label="Bandwidth graph range"
            value={range}
            onChange={(e) => {
              setRange(e.target.value);
              setShift(0);
            }}
          >
            {[
              ["day", "24 hours"],
              ["week", "7 days"],
              ["month", "30 days"],
              ["year", "1 year"],
              ["all", "All recorded time"],
              ["custom", "Custom dates"],
            ].map(([v, l]) => (
              <option value={v} key={v}>
                {l}
              </option>
            ))}
          </select>
        </label>
        {!["all", "custom"].includes(range) && (
          <>
            <button
              className="button small secondary"
              disabled={!data || data.from <= totals.since}
              onClick={() => setShift((n) => n + 1)}
            >
              Earlier period
            </button>
            <button
              className="button small secondary"
              disabled={!shift}
              onClick={() => setShift((n) => Math.max(0, n - 1))}
            >
              Later period
            </button>
          </>
        )}
        {range === "custom" && (
          <>
            <label>
              From{" "}
              <input
                aria-label="Graph start date"
                type="date"
                min={date(totals.since)}
                max={date(Date.now() / 1000)}
                value={custom.from}
                onChange={(e) => setCustom({ ...custom, from: e.target.value })}
              />
            </label>
            <label>
              Through{" "}
              <input
                aria-label="Graph end date"
                type="date"
                min={custom.from}
                max={date(Date.now() / 1000)}
                value={custom.to}
                onChange={(e) => setCustom({ ...custom, to: e.target.value })}
              />
            </label>
          </>
        )}
      </div>
      {error && <p role="alert">{error}</p>}
      {data && (
        <>
          <div className="traffic-legend">
            <label className="download-series">
              <input
                type="checkbox"
                checked={showDownloads}
                onChange={(e) => setShowDownloads(e.target.checked)}
              />{" "}
              Downloads
            </label>
            <label className="upload-series">
              <input
                type="checkbox"
                checked={showUploads}
                onChange={(e) => setShowUploads(e.target.checked)}
              />{" "}
              Uploads
            </label>
            <small>
              Volume per{" "}
              {data.interval < 3600
                ? `${data.interval / 60} min`
                : data.interval < 86400
                  ? `${data.interval / 3600} hour(s)`
                  : `${data.interval / 86400} day(s)`}{" "}
              · UTC
            </small>
          </div>
          {!showDownloads && !showUploads && (
            <p className="hint">Select uploads or downloads to show a graph.</p>
          )}
          <svg
            viewBox="0 0 960 255"
            role="img"
            aria-label={
              showDownloads && showUploads
                ? "Upload and download volume over time"
                : showDownloads
                  ? "Download volume over time"
                  : showUploads
                    ? "Upload volume over time"
                    : "Transfer volume graph"
            }
            className="traffic-chart"
          >
            <title>
              File volumes, {new Date(data.from * 1000).toLocaleDateString()} to{" "}
              {new Date(data.to * 1000).toLocaleDateString()}
            </title>
            <line x1="40" x2="940" y1="220" y2="220" className="graph-axis" />
            <text x="40" y="20">
              {bytes(maximum)}
            </text>
            <text x="40" y="245">
              {date(data.from)}
            </text>
            <text x="940" y="245" textAnchor="end">
              {date(data.to)}
            </text>
            {showDownloads && (
              <path d={path("downloaded")} className="download-series" />
            )}
            {showUploads && (
              <path d={path("uploaded")} className="upload-series" />
            )}
            {point && (
              <line
                x1={40 + (selected / Math.max(1, points.length - 1)) * 900}
                x2={40 + (selected / Math.max(1, points.length - 1)) * 900}
                y1="35"
                y2="220"
                className="graph-cursor"
              />
            )}
          </svg>
          {point && (
            <>
              <input
                className="graph-scrubber"
                aria-label="Inspect graph time"
                type="range"
                min={0}
                max={Math.max(0, points.length - 1)}
                value={Math.min(selected, points.length - 1)}
                onChange={(e) => setSelected(Number(e.target.value))}
              />
              <output className="graph-readout">
                {new Date(point.start * 1000).toLocaleString("en-GB", {
                  timeZone: "UTC",
                })}{" "}
                UTC · Downloaded {bytes(point.downloaded)} · Uploaded{" "}
                {bytes(point.uploaded)}
              </output>
            </>
          )}
        </>
      )}
      <p className="hint">
        Minute detail for 7 days, hourly for 90 days, then daily totals kept
        indefinitely. Older ranges use the retained resolution; boundary buckets
        are included in full.
      </p>
    </section>
  );
}
