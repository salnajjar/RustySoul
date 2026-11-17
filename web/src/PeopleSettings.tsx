import { useState } from "react";
import type { Snapshot } from "./types";
import User from "./User";
export default function PeopleSettings({
  state,
  send,
}: {
  state: Snapshot;
  send: (command: object) => void;
}) {
  const [filter, setFilter] = useState("");
  const [limit, setLimit] = useState(100);
  const matching = state.user_rules.filter((r) =>
    r.username.toLowerCase().includes(filter.toLowerCase()),
  );
  return (
    <section className="panel wide">
      <h2>Message & sharing controls</h2>
      <p>
        Ignore a user's messages or prevent them from accessing your shares.
        These settings are saved across restarts.
      </p>
      <input
        aria-label="Filter user controls"
        placeholder="Find a username…"
        value={filter}
        onChange={(e) => {
          setFilter(e.target.value);
          setLimit(100);
        }}
      />
      {matching.slice(0, limit).map((rule) => (
        <div className="user-rule" key={rule.username}>
          <User username={rule.username} />
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={rule.ignored}
              onChange={(e) =>
                send({
                  type: "set_user_rule",
                  ...rule,
                  ignored: e.target.checked,
                })
              }
            />
            Ignore messages
          </label>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={rule.banned}
              onChange={(e) =>
                send({
                  type: "set_user_rule",
                  ...rule,
                  banned: e.target.checked,
                })
              }
            />
            Ban sharing
          </label>
        </div>
      ))}
      {matching.length > limit && (
        <button
          className="button secondary"
          onClick={() => setLimit((n) => n + 100)}
        >
          Show more controls
        </button>
      )}
      <p className="hint">
        Click any username to add controls. Removing both options removes the
        entry. Banning cancels current uploads to that username; it does not
        delete files or historical messages.
      </p>
    </section>
  );
}
