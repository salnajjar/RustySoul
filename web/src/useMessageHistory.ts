import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import type { Message } from "./types";
type Page = { messages: Message[]; has_more: boolean };
export type Conversation = { target: string; private: boolean };

export function useConversations(
  token: string,
  enabled: boolean,
  reset: number,
  revision?: number,
) {
  const latest = useRef(revision);
  latest.current = revision;
  const [data, setData] = useState<Conversation[]>([]);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    const abort = new AbortController();
    let busy = false,
      stamp = latest.current;
    setData([]);
    async function load() {
      if (busy || abort.signal.aborted) return;
      busy = true;
      const requested = latest.current;
      setError("");
      try {
        const rows = await api<Conversation[]>(
          token,
          "messages/conversations",
          undefined,
          abort.signal,
        );
        if (!abort.signal.aborted) {
          setData(rows);
          stamp = requested;
        }
      } catch (e) {
        if (!abort.signal.aborted) setError((e as Error).message);
      } finally {
        busy = false;
      }
    }
    void load();
    const timer = setInterval(() => {
      if (stamp !== latest.current) void load();
    }, 5000);
    return () => {
      abort.abort();
      clearInterval(timer);
    };
  }, [token, enabled, reset, retry]);
  return { data, error, retry: () => setRetry((n) => n + 1) };
}

export function useMessageHistory(
  token: string,
  target: string,
  privateChat: boolean,
  revision: number | undefined,
  reset: number,
  live: Message[],
) {
  const enabled = revision !== undefined;
  const key = JSON.stringify([target, privateChat, reset]);
  const latest = useRef(revision);
  latest.current = revision;
  const [state, setState] = useState<{
    key: string;
    messages: Message[];
    older: boolean;
    loading: boolean;
    error: string;
  }>({ key: "", messages: [], older: false, loading: false, error: "" });
  const load = useRef<() => void>(() => {});
  const retry = useRef<() => void>(() => {});
  useEffect(() => {
    if (!enabled || !target) return;
    const abort = new AbortController();
    let rows: Message[] = [],
      older = false,
      busy = false,
      primed = false;
    let stamp = latest.current;
    let failed: "initial" | "older" | "new" | null = null;
    setState({ key, messages: [], older: false, loading: true, error: "" });
    async function run(mode: "initial" | "older" | "new") {
      if (busy || abort.signal.aborted) return;
      busy = true;
      failed = null;
      const requested = latest.current;
      setState((s) => ({ ...s, loading: true, error: "" }));
      try {
        let cursor =
          mode === "older"
            ? rows[0]?.id
            : mode === "new"
              ? rows.at(-1)?.id
              : undefined;
        do {
          const query = new URLSearchParams({
            target,
            private: String(privateChat),
          });
          if (cursor)
            query.set(
              mode === "older" ? "before_id" : "after_id",
              String(cursor),
            );
          const page = await api<Page>(
            token,
            `messages?${query}`,
            undefined,
            abort.signal,
          );
          if (abort.signal.aborted) return;
          const merged = new Map(rows.map((m) => [m.id, m]));
          for (const message of page.messages) merged.set(message.id, message);
          rows = [...merged.values()].sort((a, b) => (a.id || 0) - (b.id || 0));
          if (mode !== "new" || !primed) older = page.has_more;
          const next = rows.at(-1)?.id;
          if (mode !== "new" || !page.has_more || !next || next === cursor)
            break;
          cursor = next;
        } while (!abort.signal.aborted);
        primed = true;
        stamp = requested;
        setState({ key, messages: rows, older, loading: false, error: "" });
      } catch (e) {
        if (!abort.signal.aborted) {
          failed = mode;
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
      if (older) void run("older");
    };
    retry.current = () => {
      void run(failed || "initial");
    };
    void run("initial");
    const timer = setInterval(() => {
      if (primed && !failed && stamp !== latest.current) void run("new");
    }, 2000);
    return () => {
      abort.abort();
      clearInterval(timer);
      load.current = () => {};
      retry.current = () => {};
    };
  }, [token, key, enabled, target, privateChat]);
  const current = enabled && state.key === key ? state : undefined;
  const messages = new Map((current?.messages ?? []).map((m) => [m.id, m]));
  const oldest = current?.messages[0]?.id || 0;
  for (const message of live) {
    if (!current || (message.id || 0) >= oldest)
      messages.set(message.id, message);
  }
  return {
    messages: enabled
      ? [...messages.values()].sort((a, b) => (a.id || 0) - (b.id || 0))
      : live,
    loading: enabled && !!target && (!current || current.loading),
    older: current?.older ?? false,
    error: current?.error || "",
    loadOlder: () => load.current(),
    retry: () => retry.current(),
  };
}
