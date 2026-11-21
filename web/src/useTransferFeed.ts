import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import type { Transfer } from "./types";

export interface TransferPage {
  users?: Record<
    string,
    {
      downloaded: number;
      uploaded: number;
      first_at: number;
      last_at: number;
      files: number;
    }
  >;
  users_since?: number;
  transfers: Transfer[];
  total: number;
  offset: number;
  next_offset?: number;
  summary: {
    total: number;
    active: number;
    queued?: number;
    speed: number;
    clearable: Record<string, number>;
  };
}

// Append while scrolling; coalesce live changes into a replay of the loaded
// window. Replays commit together, preserving old rows and scroll while busy.
export function useTransferFeed(
  token: string,
  kind: string,
  query: string,
  active: boolean,
  revision?: number,
  total?: number,
) {
  const key = `${kind}?${query}`;
  const latest = useRef("");
  latest.current = `${revision}:${total}`;
  const load = useRef(() => {}),
    retry = useRef(() => {});
  const [state, setState] = useState<{
    key: string;
    data?: TransferPage;
    loading: boolean;
    error: string;
  }>({ key, loading: active, error: "" });
  useEffect(() => {
    if (!active) return;
    const abort = new AbortController();
    let cache: TransferPage | undefined,
      next = 0,
      stamp = "",
      busy = false,
      failed = false;
    setState({ key, loading: true, error: "" });
    async function run(append: boolean, target: number) {
      if (busy || abort.signal.aborted) return;
      busy = true;
      failed = false;
      setState((s) => ({ ...s, loading: true, error: "" }));
      const requestedStamp = latest.current;
      const incremental = append && cache && stamp === requestedStamp;
      let offset = incremental ? next : 0;
      const files = new Map<string, Transfer>(
        incremental
          ? cache!.transfers.map((t) => [
              JSON.stringify([t.username, t.path]),
              t,
            ])
          : [],
      );
      const users: NonNullable<TransferPage["users"]> = incremental
        ? { ...cache!.users }
        : {};
      let page = cache;
      try {
        do {
          const q = new URLSearchParams(query);
          q.set("offset", String(offset));
          q.set("limit", String(Math.min(1000, Math.max(1, target - offset))));
          page = await api<TransferPage>(
            token,
            `transfers/${kind}?${q}`,
            undefined,
            abort.signal,
          );
          for (const t of page.transfers)
            files.set(JSON.stringify([t.username, t.path]), t);
          Object.assign(users, page.users);
          const following = page.next_offset ?? offset + page.transfers.length;
          if (following <= offset || !page.transfers.length) {
            offset = following;
            break;
          }
          offset = following;
        } while (offset < target && offset < page.total);
        if (abort.signal.aborted) return;
        next = offset;
        stamp = requestedStamp;
        cache = {
          ...page!,
          offset: 0,
          next_offset: next,
          transfers: [...files.values()],
          users,
        };
        setState({ key, data: cache, loading: false, error: "" });
      } catch (e) {
        if (!abort.signal.aborted) {
          failed = true;
          setState((s) => ({
            ...s,
            loading: false,
            error: (e as Error).message,
          }));
        }
      } finally {
        busy = false;
      }
    }
    load.current = () => {
      if (!failed) void run(true, Math.max(100, next + 100));
    };
    retry.current = () => {
      void run(false, Math.max(100, next));
    };
    void run(false, 100);
    const timer = setInterval(() => {
      if (!failed && cache && stamp !== latest.current)
        void run(false, Math.max(100, next));
    }, 2000);
    return () => {
      abort.abort();
      clearInterval(timer);
      load.current = () => {};
      retry.current = () => {};
    };
  }, [token, key, active]);
  return {
    data: state.key === key ? state.data : undefined,
    loading: state.key !== key || state.loading,
    error: state.key === key ? state.error : "",
    load: () => load.current(),
    retry: () => retry.current(),
  };
}
