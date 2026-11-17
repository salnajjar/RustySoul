import type { ReactNode } from "react";
import { X } from "lucide-react";
export default function SystemMessage({
  children,
  onDismiss,
  error = false,
}: {
  children: ReactNode;
  onDismiss: () => void;
  error?: boolean;
}) {
  return (
    <div
      className={`system-message ${error ? "message-error" : "message-info"}`}
      role={error ? "alert" : "status"}
    >
      <div>{children}</div>
      <button
        type="button"
        className="icon-button"
        aria-label={error ? "Dismiss error" : "Dismiss message"}
        onClick={onDismiss}
      >
        <X size={17} />
      </button>
    </div>
  );
}
