import { useState } from "react";
import { Star, Play, Trash2 } from "lucide-react";
import type { Snapshot } from "./types";
export default function Wishlist({
  state,
  send,
  open,
}: {
  state: Snapshot;
  send: (command: object) => void;
  open: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const running = state.searches.some((s) => s.wishlist_id && s.collecting);
  const ready =
    state.connection === "connected" &&
    !running &&
    Date.now() / 1000 >= state.wishlist_next_at;
  return (
    <details className="panel wishlist-panel">
      <summary>
        <Star size={16} /> Wishlist ·{" "}
        {state.wishlist.filter((w) => w.enabled).length} enabled
      </summary>
      <p className="hint">
        RustySoul checks enabled wishes in rotation, at the interval allowed by
        Soulseek. Matching files are saved for you to review; downloads are
        always your choice.
      </p>
      <form
        className="wishlist-form"
        onSubmit={(e) => {
          e.preventDefault();
          send({ type: "add_wish", query: query.trim() });
          setQuery("");
        }}
      >
        <input
          aria-label="Wishlist query"
          value={query}
          maxLength={256}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="An album or artist to keep looking for…"
        />
        <button
          className="button small"
          disabled={!query.trim() || state.wishlist.length >= 128}
        >
          Add wish
        </button>
      </form>
      {state.wishlist.map((w) => (
        <div className="wishlist-item" key={w.id}>
          <label className="checkbox-label">
            <input
              type="checkbox"
              aria-label={`Enable wish ${w.query}`}
              checked={w.enabled}
              onChange={(e) =>
                send({
                  type: "update_wish",
                  id: w.id,
                  enabled: e.target.checked,
                })
              }
            />
            <span>
              {w.query}
              <small>
                {w.last_run
                  ? `${w.last_count.toLocaleString()} cached files · checked ${new Date(w.last_run * 1000).toLocaleString()}`
                  : "Waiting for its first check"}
              </small>
            </span>
          </label>
          <div className="button-row">
            <button
              className="button small secondary"
              disabled={!w.search_id}
              onClick={() => {
                send({ type: "open_wish", id: w.id });
                if (w.search_id) open(w.search_id);
              }}
            >
              View results
            </button>
            <button
              className="icon-button"
              aria-label={`Check wish ${w.query} now`}
              disabled={!ready}
              title={
                ready
                  ? "Check now"
                  : "Waiting for the server's wishlist interval or a connection"
              }
              onClick={() => send({ type: "run_wish", id: w.id })}
            >
              <Play size={15} />
            </button>
            <button className="button small secondary" disabled={!w.search_id} onClick={()=>send({type:"clear_wish_results",id:w.id})}>Clear matches</button>
            <button
              className="icon-button"
              aria-label={`Remove wish ${w.query}`}
              onClick={() => send({ type: "remove_wish", id: w.id })}
            >
              <Trash2 size={15} />
            </button>
          </div>
        </div>
      ))}
      <p className="hint">
        {running
          ? "Collecting wishlist results…"
          : state.connection !== "connected"
            ? "Your wishes are saved. Connect to Soulseek to check them."
            : state.wishlist_next_at > Date.now() / 1000
              ? `Next search slot: ${new Date(state.wishlist_next_at * 1000).toLocaleTimeString()}`
              : "Ready for the next search slot."}
      </p>
    </details>
  );
}
