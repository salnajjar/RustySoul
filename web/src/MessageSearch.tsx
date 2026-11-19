import { useEffect, useRef, useState, type FormEvent } from "react";
import { api } from "./api";
import type { Message } from "./types";
import ConversationMessages from "./ConversationMessages";
type Page = { messages: Message[]; has_more: boolean };
export default function MessageSearch({
  token,
  target,
  privateChat,
  visibility,
  close,
}: {
  token: string;
  target: string;
  privateChat: boolean;
  visibility: string;
  close: () => void;
}) {
  const [text, setText] = useState("");
  const [user, setUser] = useState("");
  const [query, setQuery] = useState<{
    filter: string;
    username: string;
  } | null>(null);
  const [page, setPage] = useState<Page>({ messages: [], has_more: false });
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const request = useRef<AbortController | null>(null);
  useEffect(() => () => request.current?.abort(), []);
  useEffect(() => {
    request.current?.abort();
    setPage({ messages: [], has_more: false });
    setQuery(null);
    setError("");
    setBusy(false);
  }, [visibility, target, privateChat]);
  async function load(
    next: { filter: string; username: string },
    older = false,
  ) {
    request.current?.abort();
    const abort = new AbortController();
    request.current = abort;
    setBusy(true);
    setError("");
    if (!older) setPage({ messages: [], has_more: false });
    setQuery(next);
    try {
      const params = new URLSearchParams({
        target,
        private: String(privateChat),
        ...next,
      });
      if (older && page.messages[0]?.id)
        params.set("before_id", String(page.messages[0].id));
      const result = await api<Page>(
        token,
        `messages?${params}`,
        undefined,
        abort.signal,
      );
      if (!abort.signal.aborted)
        setPage({
          messages: older
            ? [...result.messages, ...page.messages]
            : result.messages,
          has_more: result.has_more,
        });
    } catch (e) {
      if (!abort.signal.aborted) setError((e as Error).message);
    } finally {
      if (!abort.signal.aborted) setBusy(false);
    }
  }
  function search(e: FormEvent) {
    e.preventDefault();
    void load({ filter: text.trim(), username: user.trim() });
  }
  return (
    <section className="message-search" aria-label="Search saved messages">
      <form className="search-history-form" onSubmit={search}>
        <label>
          Message contains
          <input
            aria-label="Search message text"
            placeholder="Words or a phrase…"
            maxLength={256}
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
        </label>
        <label>
          From username
          <input
            aria-label="Search messages from user"
            placeholder="Any user, or an exact username…"
            maxLength={30}
            value={user}
            onChange={(e) => setUser(e.target.value)}
          />
        </label>
        <div className="button-row">
          <button
            className="button small"
            disabled={busy || (!text.trim() && !user.trim())}
          >
            Search saved messages
          </button>
          <button
            className="button small secondary"
            type="button"
            onClick={close}
          >
            Back to conversation
          </button>
        </div>
      </form>
      <p className="hint">
        Searches all saved messages in this {privateChat ? "chat" : "room"},
        including older history. Username matching ignores letter case. Use
        either field or both. Ignored users stay hidden. Search again to include
        new arrivals; searching does not mark unseen messages as read.
      </p>
      {error && (
        <p role="alert">
          {error}{" "}
          <button
            className="text-link"
            onClick={() => query && void load(query)}
          >
            Retry message search
          </button>
        </p>
      )}
      {busy && <p role="status">Searching saved messages…</p>}
      {query && !busy && !error && (
        <p role="status">
          {page.messages.length.toLocaleString()} matching messages loaded
          {page.has_more ? " · more available" : ""}.
        </p>
      )}
      <div
        className="message-search-results"
        aria-label="Message search results"
      >
        {page.has_more && (
          <button
            className="button small secondary"
            disabled={busy}
            onClick={() => query && void load(query, true)}
          >
            Load older matches
          </button>
        )}
        <ConversationMessages messages={page.messages} />
      </div>
    </section>
  );
}
