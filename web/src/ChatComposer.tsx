import { useEffect, useId, useRef, useState } from "react";
import { ArrowUp, Smile, X } from "lucide-react";
import { emojis, insertEmoji, messageBytes } from "./emoji";

export default function ChatComposer({
  draft,
  setDraft,
  disabled,
  send,
  target,
  privateChat,
}: {
  draft: string;
  setDraft: (s: string) => void;
  disabled: boolean;
  send: (c: object) => unknown;
  target: string;
  privateChat: boolean;
}) {
  const input = useRef<HTMLInputElement>(null),
    search = useRef<HTMLInputElement>(null),
    picker = useRef<HTMLDivElement>(null);
  const selection = useRef({ start: draft.length, end: draft.length });
  const [open, setOpen] = useState(false),
    [filter, setFilter] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const id = useId();
  const count = messageBytes(draft);
  const found = emojis.filter(
    (e) =>
      e.keywords.includes(filter.toLowerCase().replaceAll(":", "").trim()) ||
      e.character === filter.trim(),
  );
  useEffect(() => {
    if (!open) return;
    search.current?.focus();
    const outside = (e: PointerEvent) => {
      if (!picker.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  useEffect(() => {
    setOpen(false);
    setError("");
    selection.current = { start: draft.length, end: draft.length };
  }, [target, privateChat]);
  const remember = () => {
    if (input.current)
      selection.current = {
        start: input.current.selectionStart ?? draft.length,
        end: input.current.selectionEnd ?? draft.length,
      };
  };
  const close = () => {
    setOpen(false);
    input.current?.focus();
  };
  function choose(character: string) {
    const next = insertEmoji(
      draft,
      selection.current.start,
      selection.current.end,
      character,
    );
    if (messageBytes(next.text) > 2000) {
      setError(
        "This message is too long. Remove some text before adding an emoji.",
      );
      return;
    }
    setDraft(next.text);
    setError("");
    setOpen(false);
    requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.setSelectionRange(next.cursor, next.cursor);
      selection.current = { start: next.cursor, end: next.cursor };
    });
  }
  return (
    <form
      className="message-form chat-composer"
      onSubmit={async (e) => {
        e.preventDefault();
        if (disabled || busy || !draft.trim() || count > 2000) return;
        setBusy(true);
        setError("");
        try {
          if (
            (await send({
              type: "send_message",
              target,
              text: draft,
              private: privateChat,
            })) !== false
          ) {
            setDraft("");
            selection.current = { start: 0, end: 0 };
          }
        } catch (e) {
          setError((e as Error).message);
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="composer-row">
        <input
          ref={input}
          aria-label="Message"
          placeholder="Write a message…"
          value={draft}
          onSelect={remember}
          onKeyUp={remember}
          onClick={remember}
          onChange={(e) => {
            setDraft(e.target.value);
            selection.current = {
              start: e.target.selectionStart ?? e.target.value.length,
              end: e.target.selectionEnd ?? e.target.value.length,
            };
            setError("");
          }}
          maxLength={2000}
          disabled={busy}
        />
        <div
          className="emoji-control"
          ref={picker}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              close();
            }
          }}
        >
          <button
            type="button"
            className="icon-button emoji-trigger"
            aria-label="Insert emoji"
            aria-expanded={open}
            aria-controls={id}
            disabled={busy}
            title="Insert emoji"
            onMouseDown={(e) => {
              e.preventDefault();
              remember();
            }}
            onClick={() => {
              setOpen(!open);
              setFilter("");
            }}
          >
            <Smile size={20} />
          </button>
          {open && (
            <div
              className="emoji-picker"
              role="dialog"
              aria-label="Choose an emoji"
              id={id}
            >
              <div className="emoji-heading">
                <strong>Emoji</strong>
                <button
                  type="button"
                  className="icon-button"
                  aria-label="Close emoji picker"
                  onClick={close}
                >
                  <X size={18} />
                </button>
              </div>
              <input
                ref={search}
                type="search"
                aria-label="Search emoji"
                placeholder="Search smiles, music, thanks…"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
              />
              <div
                className="emoji-grid"
                role="group"
                aria-label="Emoji choices"
                onKeyDown={(e) => {
                  const buttons = Array.from(
                    e.currentTarget.querySelectorAll<HTMLButtonElement>(
                      "button",
                    ),
                  );
                  const i = buttons.indexOf(
                    document.activeElement as HTMLButtonElement,
                  );
                  const columns = getComputedStyle(
                    e.currentTarget,
                  ).gridTemplateColumns.split(" ").length;
                  const step =
                    e.key === "ArrowRight"
                      ? 1
                      : e.key === "ArrowLeft"
                        ? -1
                        : e.key === "ArrowDown"
                          ? columns
                          : e.key === "ArrowUp"
                            ? -columns
                            : 0;
                  if (step && i >= 0) {
                    e.preventDefault();
                    buttons[
                      (i + step + buttons.length) % buttons.length
                    ]?.focus();
                  }
                }}
              >
                {found.map((e) => (
                  <button
                    key={e.character}
                    type="button"
                    aria-label={`${e.name} ${e.character}`}
                    title={e.keywords}
                    onClick={() => choose(e.character)}
                  >
                    {e.character}
                  </button>
                ))}
              </div>
              {!found.length && <p className="hint">No matching emoji.</p>}
              <p className="hint">
                Sent as standard text, just like pasted emoji.
              </p>
            </div>
          )}
        </div>
        <button
          className="button"
          disabled={disabled || busy || !draft.trim() || count > 2000}
        >
          Send
          <ArrowUp size={15} />
        </button>
      </div>
      {(error || count > 2000) && (
        <p role="alert">
          {error ||
            "This message is too long. Please shorten it before sending."}
        </p>
      )}
      {count >= 1800 && (
        <small title="The limit is 2,000 UTF-8 bytes; an emoji may use several bytes.">
          {count.toLocaleString()} / 2,000 bytes
        </small>
      )}
    </form>
  );
}
