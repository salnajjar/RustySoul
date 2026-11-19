import { useEffect, useState } from "react";
import { Compass, RefreshCw, Search, ThumbsDown, ThumbsUp } from "lucide-react";
import type { DiscoveryQuery, Snapshot } from "./types";
import User from "./User";

export default function Discover({
  state,
  send,
  search,
  profile,
  signin,
}: {
  state: Snapshot;
  send: (c: object) => unknown;
  search: (query: string) => void;
  profile: () => void;
  signin: () => void;
}) {
  const data = state.discovery;
  const query = data?.query || { mode: "personal" };
  const [topic, setTopic] = useState(
    query.mode === "topic" || query.mode === "dislikes" ? query.item : "",
  );
  const [preference, setPreference] = useState("like");
  const [filter, setFilter] = useState("");
  const [limit, setLimit] = useState(50);
  const [peopleLimit, setPeopleLimit] = useState(50);
  const online = state.connection === "connected";
  const pending = Boolean(
    data?.recommendations_pending || data?.people_pending,
  );
  const key = JSON.stringify(query);
  useEffect(() => {
    setPreference(query.mode === "dislikes" ? "dislike" : "like");
    setFilter("");
    setLimit(50);
    setPeopleLimit(50);
    setTopic(
      query.mode === "topic" || query.mode === "dislikes" ? query.item : "",
    );
  }, [key]);
  const load = (q: DiscoveryQuery, refresh = false) =>
    void send({ type: "discover", query: q, refresh });
  const match = (text: string) =>
    text.toLocaleLowerCase().includes(filter.toLocaleLowerCase());
  const recommendations = (data?.recommendations || []).filter((r) =>
    match(r.item),
  );
  const against = (data?.against || []).filter((r) => match(r.item));
  const people = (data?.people || []).filter((p) => match(p.username));
  const when = (at?: number | null) =>
    at
      ? `Last updated ${new Date(at * 1000).toLocaleString()}`
      : "No cached results yet";
  const liked = (item: string) =>
    state.profile?.likes.includes(item.toLowerCase());
  const disliked = (item: string) =>
    state.profile?.dislikes.includes(item.toLowerCase());
  function interestButtons(item: string, labelled = false) {
    return (
      <>
        <button
          className={labelled ? "button small secondary" : "icon-button"}
          aria-pressed={Boolean(liked(item))}
          onClick={() =>
            void send({
              type: "set_interest",
              item,
              disposition: liked(item) ? "remove" : "like",
            })
          }
          aria-label={`${liked(item) ? "Remove like for" : "Like"} ${item}`}
          title="Add or remove a like"
        >
          <ThumbsUp size={16} aria-hidden="true" />
          {labelled && "Like"}
        </button>
        <button
          className={labelled ? "button small secondary" : "icon-button"}
          aria-pressed={Boolean(disliked(item))}
          onClick={() =>
            void send({
              type: "set_interest",
              item,
              disposition: disliked(item) ? "remove" : "dislike",
            })
          }
          aria-label={`${disliked(item) ? "Remove dislike for" : "Dislike"} ${item}`}
          title="Add or remove a dislike"
        >
          <ThumbsDown size={16} aria-hidden="true" />
          {labelled && "Dislike"}
        </button>
      </>
    );
  }
  function itemRow(
    r: { item: string; rating: number },
    dislikeContext = false,
  ) {
    return (
      <li className="discovery-item" key={r.item}>
        <div className="discovery-item-name">
          <button
            className="text-link"
            disabled={pending}
            onClick={() =>
              load({
                mode: dislikeContext ? "dislikes" : "topic",
                item: r.item,
              })
            }
            aria-label={`Explore ${r.item}`}
          >
            {r.item}
          </button>
          <span className="hint">
            {query.mode === "dislikes"
              ? `${Math.abs(r.rating).toLocaleString()} matching public profiles`
              : `Score ${r.rating.toLocaleString()}`}
          </span>
        </div>
        <div className="discovery-actions">
          <button
            className="icon-button"
            disabled={!online}
            onClick={() => search(r.item)}
            aria-label={`Search files for ${r.item}`}
            title="Search files"
          >
            <Search size={16} />
          </button>
          {interestButtons(r.item)}
        </div>
      </li>
    );
  }
  return (
    <>
      <header className="page-header">
        <div>
          <div className="eyebrow">FOLLOW YOUR INTERESTS</div>
          <h1>Discover</h1>
          <p>Find music and people through the Soulseek community.</p>
        </div>
        <Compass size={30} />
      </header>
      {!online && (
        <p className="inline-notice">
          Cached results are available offline.{" "}
          <button className="text-link" onClick={signin}>
            Sign in to Soulseek
          </button>{" "}
          to refresh or search files.
        </p>
      )}
      <section className="panel discovery-controls">
        <div
          className="discovery-modes"
          role="group"
          aria-label="Discovery source"
        >
          <button
            className={`button ${query.mode === "personal" ? "" : "secondary"}`}
            aria-pressed={query.mode === "personal"}
            disabled={pending}
            onClick={() => load({ mode: "personal" })}
          >
            For you
          </button>
          <button
            className={`button ${query.mode === "popular" ? "" : "secondary"}`}
            aria-pressed={query.mode === "popular"}
            disabled={pending}
            onClick={() => load({ mode: "popular" })}
          >
            Popular
          </button>
          <button
            className={`button ${query.mode === "dislikes" ? "" : "secondary"}`}
            disabled={pending || !state.profile?.dislikes.length}
            aria-pressed={query.mode === "dislikes"}
            onClick={() => {
              const item = state.profile?.dislikes[0];
              if (item) load({ mode: "dislikes", item });
            }}
          >
            Shared dislikes
          </button>
        </div>
        {!!state.profile?.dislikes.length && (
          <div className="interest-tags" aria-label="Explore things I dislike">
            {state.profile.dislikes.map((item) => (
              <button
                className="button small secondary"
                key={item}
                disabled={pending}
                onClick={() => load({ mode: "dislikes", item })}
              >
                People who dislike {item}
              </button>
            ))}
          </div>
        )}
        <form
          className="discovery-topic"
          onSubmit={(e) => {
            e.preventDefault();
            if (topic.trim())
              load({
                mode: preference === "dislike" ? "dislikes" : "topic",
                item: topic.trim(),
              });
          }}
        >
          <label>
            Match people who
            <select
              aria-label="Discovery preference"
              value={preference}
              onChange={(e) => setPreference(e.target.value)}
            >
              <option value="like">Like this topic</option>
              <option value="dislike">Dislike this topic</option>
            </select>
          </label>
          <label>
            Explore an artist, genre or topic
            <input
              aria-label="Discovery topic"
              value={topic}
              maxLength={80}
              onChange={(e) => setTopic(e.target.value)}
            />
          </label>
          <button
            className="button secondary"
            disabled={pending || !topic.trim()}
          >
            Explore topic
          </button>
        </form>
        <div className="button-row">
          <button
            className="button"
            disabled={!online || pending}
            onClick={() => load(query, true)}
          >
            <RefreshCw size={16} />
            {pending
              ? "Updating discovery…"
              : data?.recommendations_at
                ? "Refresh discovery"
                : "Get recommendations"}
          </button>
          <button className="text-link" onClick={profile}>
            Edit my interests
          </button>
        </div>
        <p className="hint">
          Personal recommendations use the likes and dislikes in your profile.
          Refresh after changing them. Scores reflect community preferences, not
          file availability.
        </p>
        {query.mode === "personal" &&
          !state.profile?.likes.length &&
          !state.profile?.dislikes.length && (
            <p>
              Add some likes in your profile to help Soulseek find
              recommendations and similar listeners.
            </p>
          )}
        {(query.mode === "topic" || query.mode === "dislikes") && (
          <div className="discovery-interest-heading">
            <h2>
              {query.mode === "topic" ? "Related to" : "Shared dislike:"}{" "}
              {query.item}
            </h2>
            <div
              className="discovery-actions"
              role="group"
              aria-label={`Your preference for ${query.item}`}
            >
              {interestButtons(query.item, true)}
            </div>
          </div>
        )}
        {query.mode === "dislikes" && (
          <>
            <p className="hint">
              People and topics are matched against published dislikes. We check
              a sample from server suggestions, your rooms, buddies and recently
              viewed profiles. Fresh public-profile data may be reused for up to
              15 minutes.
            </p>
            {data?.dislike_evidence && (
              <p role="status">
                {data.dislike_evidence.checked} public profiles checked ·{" "}
                {data.dislike_evidence.matched} confirmed matches ·{" "}
                {data.dislike_evidence.cached} cached profiles
                {data.dislike_evidence.timed_out
                  ? ` · ${data.dislike_evidence.timed_out} did not respond`
                  : ""}
                {data.dislike_evidence.limited
                  ? " · Sample limit reached; refresh to check more people"
                  : ""}
                {data.dislike_evidence.missing_sources
                  ? " · Some server suggestions were unavailable"
                  : ""}
              </p>
            )}
            <p className="hint">
              This is a sample of verified public profiles, not a network-wide
              list of everyone who dislikes the topic.
            </p>
          </>
        )}
        <label>
          Filter these results
          <input
            type="search"
            aria-label="Filter discovery results"
            value={filter}
            onChange={(e) => {
              setFilter(e.target.value);
              setLimit(50);
              setPeopleLimit(50);
            }}
          />
        </label>
      </section>
      <div className="discovery-grid">
        <section className="panel" aria-labelledby="discovery-recommendations">
          <h2 id="discovery-recommendations">
            {query.mode === "dislikes"
              ? "Things these people like"
              : "Recommendations"}{" "}
            <span className="hint">({recommendations.length})</span>
          </h2>
          <p className="hint">{when(data?.recommendations_at)}</p>
          {data?.recommendations_pending && (
            <p role="status">Updating recommendations…</p>
          )}
          {data?.recommendations_error && (
            <p role="alert">{data.recommendations_error}</p>
          )}
          <ul className="discovery-list">
            {recommendations.slice(0, limit).map((r) => itemRow(r))}
          </ul>
          {!recommendations.length && (
            <p className="hint">
              {data?.recommendations_at
                ? filter
                  ? "No recommendations match your filter."
                  : "The server returned no recommendations. Try another topic or add more interests."
                : "Choose a source and get recommendations to begin."}
            </p>
          )}
          {recommendations.length > limit && (
            <button
              className="button secondary"
              onClick={() => setLimit((n) => n + 50)}
            >
              Show more recommendations
            </button>
          )}
          {against.length > 0 && (
            <details className="discovery-against">
              <summary>
                {query.mode === "dislikes"
                  ? "Other things they dislike"
                  : "Less recommended"}{" "}
                ({against.length})
              </summary>
              <p className="hint">
                Topics this group tends to dislike. They are not blocked from
                searches or downloads.
              </p>
              <ul className="discovery-list">
                {against
                  .slice(0, limit)
                  .map((r) => itemRow(r, query.mode === "dislikes"))}
              </ul>
              {against.length > limit && (
                <button
                  className="button secondary"
                  onClick={() => setLimit((n) => n + 50)}
                >
                  Show more less recommended topics
                </button>
              )}
            </details>
          )}
        </section>
        {query.mode !== "popular" && (
          <section className="panel" aria-labelledby="discovery-people">
            <h2 id="discovery-people">
              {query.mode === "dislikes"
                ? "People who also dislike this"
                : query.mode === "topic"
                  ? "People who like this"
                  : "Similar listeners"}{" "}
              <span className="hint">({people.length})</span>
            </h2>
            <p className="hint">{when(data?.people_at)}</p>
            {data?.people_pending && <p role="status">Finding people…</p>}
            {data?.people_error && <p role="alert">{data.people_error}</p>}
            <ul className="discovery-list">
              {people.slice(0, peopleLimit).map((p) => (
                <li className="discovery-person" key={p.username}>
                  <User username={p.username} />
                  {p.weight !== null && query.mode !== "dislikes" && (
                    <span className="hint">
                      Similarity score {p.weight.toLocaleString()}
                    </span>
                  )}
                </li>
              ))}
            </ul>
            {!people.length && (
              <p className="hint">
                {query.mode === "dislikes" && pending
                  ? "Checking public dislikes…"
                  : data?.people_at
                    ? filter
                      ? "No people match your filter."
                      : query.mode === "dislikes"
                        ? "No verified shared-dislike matches in the checked public profiles. Refresh to check more people."
                        : "The server returned no people for these interests."
                    : "Get recommendations to find people with shared interests."}
              </p>
            )}
            {people.length > peopleLimit && (
              <button
                className="button secondary"
                onClick={() => setPeopleLimit((n) => n + 50)}
              >
                Show more people
              </button>
            )}
            <p className="hint">
              Select a username to browse shares, view their profile, message
              them or add a buddy. Unknown status does not mean offline.
            </p>
          </section>
        )}
      </div>
      <p className="hint">
        Recent results are saved for offline viewing, with up to 500 entries per
        list. Previous results stay visible during refresh.
      </p>
    </>
  );
}
