import { useEffect, useRef, useState } from "react";
import { basename } from "./api";
import type { Transfer } from "./types";
export default function AudioPlayer({
  file,
  close,
}: {
  file: Transfer;
  close: () => void;
}) {
  const ref = useRef<HTMLAudioElement>(null);
  const [error, setError] = useState("");
  const url = `/api/v1/downloads/file?${new URLSearchParams({ username: file.username, path: file.path })}`;
  useEffect(() => setError(""), [url]);
  async function failed() {
    try {
      const response = await fetch(url, {
        method: "HEAD",
        credentials: "same-origin",
      });
      setError(
        response.status === 404
          ? "The file is missing from its recorded location. It may have been moved or removed."
          : !response.ok
            ? "The file cannot be opened. Check your session and file access."
            : "Your browser cannot play this format. Use Save file to open it with a local player.",
      );
    } catch {
      setError("The server could not be reached. Try again when connected.");
    }
  }
  return (
    <section
      className="audio-player"
      aria-label={file.size === 0 ? "Empty file" : "Audio playback"}
    >
      <div>
        <strong>{basename(file.path)}</strong>
        <small>
          {file.username}
          {file.local_path ? ` · ${file.local_path}` : ""}
        </small>
      </div>
      {file.size === 0 ? (
        <p>This file is empty. You can save it to your device.</p>
      ) : (
        <audio
          key={url}
          ref={ref}
          controls
          autoPlay
          preload="metadata"
          src={url}
          onError={() => void failed()}
          aria-label={`Playing ${basename(file.path)}`}
        />
      )}
      <a className="button small secondary" href={url + "&download=true"}>
        Save file
      </a>
      <button
        className="icon-button"
        aria-label={file.size === 0 ? "Close file" : "Close audio player"}
        onClick={close}
      >
        ×
      </button>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
