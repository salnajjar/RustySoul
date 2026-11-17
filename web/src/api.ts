import type { Snapshot } from "./types";
export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

export async function api<T>(
  token: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(`/api/v1/${path}`, {
    method: body === undefined ? "GET" : "POST",
    signal,
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(body === undefined ? {} : { "X-RustySoul-Request": "1" }),
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    throw new ApiError(
      data.error || `Request failed (${response.status})`,
      response.status,
    );
  }
  return response.json();
}

// Browser sessions use an HttpOnly cookie; external API clients may supply a bearer token.
export async function subscribe(
  token: string,
  onState: (state: Snapshot) => void,
  signal: AbortSignal,
): Promise<void> {
  const response = await fetch("/api/v1/events", {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    signal,
    credentials: "same-origin",
  });
  if (!response.ok || !response.body)
    throw new ApiError(
      `Live updates unavailable (${response.status})`,
      response.status,
    );
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let pending = "";
  try {
    while (!signal.aborted) {
      const { value, done } = await reader.read();
      if (done) throw new Error("Connection to daemon closed");
      pending += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
      let split: number;
      while ((split = pending.indexOf("\n\n")) >= 0) {
        const event = pending.slice(0, split);
        pending = pending.slice(split + 2);
        const data = event
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart())
          .join("\n");
        if (data) onState(JSON.parse(data));
      }
    }
  } finally {
    reader.releaseLock();
  }
}

export function bytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), 4);
  return `${(n / 1024 ** i).toLocaleString(undefined, { maximumFractionDigits: i ? 1 : 0 })} ${["B", "KiB", "MiB", "GiB", "TiB"][i]}`;
}
export function basename(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}
