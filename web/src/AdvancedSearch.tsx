import { useState, type FormEvent } from "react";
import type { FileResult } from "./types";
export type FileCriteria = Partial<
  Record<
    | "min_bitrate"
    | "max_bitrate"
    | "min_duration"
    | "max_duration"
    | "min_sample_rate"
    | "max_sample_rate"
    | "min_bit_depth"
    | "max_bit_depth"
    | "min_size"
    | "max_size",
    number
  >
> & { vbr?: boolean; free_slots?: boolean };
const ranges = [
  ["bitrate", "Bitrate (kbps)"],
  ["duration", "Duration (seconds)"],
  ["sample_rate", "Sample rate (Hz)"],
  ["bit_depth", "Bit depth (bits)"],
  ["size", "File size (MiB)"],
] as const;
export function matchesCriteria(file: FileResult, c: FileCriteria): boolean {
  const a = file.attributes;
  for (const [name, actual] of [
    ["bitrate", a["0"]],
    ["duration", a["1"]],
    ["sample_rate", a["4"]],
    ["bit_depth", a["5"]],
    ["size", file.size],
  ] as const) {
    const min = c[`min_${name}`],
      max = c[`max_${name}`];
    if (
      (min !== undefined && (actual === undefined || actual < min)) ||
      (max !== undefined && (actual === undefined || actual > max))
    )
      return false;
  }
  return (
    (c.vbr === undefined || a["2"] === Number(c.vbr)) &&
    (!c.free_slots || file.free_slots > 0)
  );
}
export default function AdvancedSearch({
  value,
  change,
}: {
  value: FileCriteria;
  change: (value: FileCriteria) => void;
}) {
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [vbr, setVbr] = useState("all"),
    [free, setFree] = useState(false),
    [error, setError] = useState("");
  const count =
    Object.values(value).filter((v) => v !== undefined && v !== false).length +
    (value.vbr === false ? 1 : 0);
  function apply(e: FormEvent) {
    e.preventDefault();
    setError("");
    const next: FileCriteria = {};
    for (const [name] of ranges) {
      const values: (number | undefined)[] = [];
      for (const bound of ["min", "max"]) {
        const key = `${bound}_${name}` as keyof Omit<
          FileCriteria,
          "vbr" | "free_slots"
        >;
        const text = draft[key]?.trim() || "";
        if (!text) {
          values.push(undefined);
          continue;
        }
        const raw = Number(text),
          number = name === "size" ? Math.round(raw * 1048576) : raw;
        if (
          !Number.isFinite(raw) ||
          raw < 0 ||
          !Number.isSafeInteger(number) ||
          number > (name === "size" ? 16 * 1024 ** 4 : 4294967295)
        ) {
          setError(
            "Use nonnegative whole attribute values and file sizes up to 16 TiB.",
          );
          return;
        }
        next[key] = number;
        values.push(number);
      }
      if (
        values[0] !== undefined &&
        values[1] !== undefined &&
        values[0] > values[1]
      ) {
        setError("Each minimum must be less than or equal to its maximum.");
        return;
      }
    }
    if (vbr !== "all") next.vbr = vbr === "vbr";
    if (free) next.free_slots = true;
    change(next);
  }
  function clear() {
    setDraft({});
    setVbr("all");
    setFree(false);
    setError("");
    change({});
  }
  return (
    <details className="advanced-search">
      <summary>
        Advanced search filters{count ? ` · ${count} active` : ""}
      </summary>
      <form onSubmit={apply}>
        <div className="advanced-ranges">
          {ranges.map(([name, label]) => (
            <fieldset key={name}>
              <legend>{label}</legend>
              {["min", "max"].map((bound) => (
                <label key={bound}>
                  {bound === "min" ? "Minimum" : "Maximum"}
                  <input
                    aria-label={`${bound === "min" ? "Minimum" : "Maximum"} ${label}`}
                    type="number"
                    min="0"
                    step={name === "size" ? "any" : "1"}
                    value={draft[`${bound}_${name}`] || ""}
                    placeholder="Any"
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        [`${bound}_${name}`]: e.target.value,
                      })
                    }
                  />
                </label>
              ))}
            </fieldset>
          ))}
        </div>
        <label>
          Bitrate mode
          <select
            aria-label="Bitrate mode"
            value={vbr}
            onChange={(e) => setVbr(e.target.value)}
          >
            <option value="all">Any or unknown</option>
            <option value="cbr">Constant bitrate (CBR)</option>
            <option value="vbr">Variable bitrate (VBR)</option>
          </select>
        </label>
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={free}
            onChange={(e) => setFree(e.target.checked)}
          />
          Only users with a free upload slot
        </label>
        <p className="hint">
          Filters apply to all collected results as they arrive. Missing
          attributes do not match an active attribute filter. Duration uses
          seconds; sample rate uses Hz (CD quality is 44100). This filters
          replies locally; other Soulseek clients receive your text search
          terms.
        </p>
        {error && <p role="alert">{error}</p>}
        <div className="button-row">
          <button className="button small">Apply advanced filters</button>
          <button
            className="button small secondary"
            type="button"
            onClick={clear}
          >
            Clear advanced filters
          </button>
        </div>
      </form>
    </details>
  );
}
