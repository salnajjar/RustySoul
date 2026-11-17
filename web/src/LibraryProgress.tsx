import type { Snapshot } from "./types";
import { bytes } from "./api";
const phases: Record<string, string> = {
  queued: "Library update queued",
  preparing: "Preparing library update",
  scanning: "Updating library",
  saving: "Saving library index",
  building: "Preparing shared library",
  idle: "Library up to date",
  failed: "Library update stopped",
};
export default function LibraryProgress({
  status,
  banner = false,
}: {
  status: Snapshot["library"];
  banner?: boolean;
}) {
  return (
    <div
      className={`library-progress ${banner ? "library-banner" : "library-detail"}`}
      role="status"
      aria-label={banner ? "Library update banner" : "Library update details"}
      aria-live="polite"
      aria-atomic="true"
    >
      <strong>
        {phases[status.phase] ||
          (status.scanning ? "Updating library" : "Library up to date")}
      </strong>
      {status.scanning && (
        <div className="scan-indicator" aria-hidden="true">
          <span />
        </div>
      )}
      <p>
        {(status.processed ?? 0).toLocaleString()} files checked ·{" "}
        {(status.folders ?? 0).toLocaleString()} folders ·{" "}
        {bytes(status.bytes ?? 0)}
      </p>
      <p>
        {status.changed.toLocaleString()} new or modified ·{" "}
        {status.skipped.toLocaleString()} unchanged ·{" "}
        {status.removed.toLocaleString()} removed
      </p>
      <small>
        {status.indexed.toLocaleString()} files in the{" "}
        {status.scanning ? "last completed" : "current"} index ·{" "}
        {(status.elapsed_ms / 1000).toFixed(1)} seconds
        {status.scanning
          ? " elapsed; final total is known when the scan finishes"
          : ""}
      </small>
    </div>
  );
}
