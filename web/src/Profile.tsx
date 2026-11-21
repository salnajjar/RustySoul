import { useEffect, useState, type FormEvent } from "react";
import { api, bytes } from "./api";
import type { ProfileSettings, Snapshot } from "./types";
const empty: ProfileSettings = {
  description: "",
  picture: null,
  likes: [],
  dislikes: [],
  away_on_startup: false,
  away_message: "",
};
export default function Profile({
  state,
  token,
  openUser,
}: {
  state: Snapshot;
  token: string;
  openUser: (u: string) => void;
}) {
  const [profile, setProfile] = useState<ProfileSettings>(empty);
  const [idleEnabled, setIdleEnabled] = useState(false);
  const [idleMinutes, setIdleMinutes] = useState("10");
  const [likes, setLikes] = useState("");
  const [dislikes, setDislikes] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [requested, setRequested] = useState(false);
  useEffect(() => {
    const abort = new AbortController();
    api<ProfileSettings>(token, "profile", undefined, abort.signal)
      .then((p) => {
        setProfile(p);
        setIdleEnabled((p.auto_away_minutes ?? 0) > 0);
        setIdleMinutes(String(p.auto_away_minutes || 10));
        setLikes(p.likes.join("\n"));
        setDislikes(p.dislikes.join("\n"));
        setLoaded(true);
        setRequested(false);
      })
      .catch((e) => {
        if (!abort.signal.aborted) setError(e.message);
      });
    return () => abort.abort();
  }, [token, state.profile?.revision]);
  async function save(e: FormEvent) {
    e.preventDefault();
    if (
      idleEnabled &&
      (!Number.isInteger(Number(idleMinutes)) ||
        Number(idleMinutes) < 1 ||
        Number(idleMinutes) > 1440)
    ) {
      setError("Choose an idle timeout from 1 to 1440 minutes.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      await api(token, "profile", {
        ...profile,
        auto_away_minutes: idleEnabled ? Number(idleMinutes) : 0,
        likes: likes
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean),
        dislikes: dislikes
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean),
      });
      setRequested(true);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  async function photo(file: File | undefined) {
    if (!file) return;
    setError("");
    if (
      file.size > 10 * 1024 * 1024 ||
      !["image/jpeg", "image/png"].includes(file.type)
    ) {
      setError("Choose a JPEG or PNG of up to 10 MB.");
      return;
    }
    const reader = new FileReader();
    reader.onload = () =>
      setProfile((p) => ({
        ...p,
        picture: String(reader.result).split(",")[1],
      }));
    reader.onerror = () => setError("Could not read this picture.");
    reader.readAsDataURL(file);
  }
  return (
    <section className="panel wide profile-settings">
      <h2>Your Soulseek profile</h2>
      <p className="hint">
        Your description and picture are shared when another user views your
        profile. Likes and dislikes are published to the network while
        connected. Account credentials are managed under Accounts.
      </p>
      {error && <p role="alert">{error}</p>}
      <form onSubmit={save}>
        <fieldset disabled={busy || !loaded}>
          <label>
            About me
            <textarea
              aria-label="Profile description"
              rows={7}
              maxLength={8192}
              value={profile.description}
              onChange={(e) =>
                setProfile({ ...profile, description: e.target.value })
              }
              placeholder="Tell other listeners about your collection and sharing preferences…"
            />
          </label>
          <div className="profile-picture-controls">
            {profile.picture && (
              <img
                alt="Your profile picture preview"
                className="profile-picture"
                src={`data:${profile.picture.startsWith("iVBOR") ? "image/png" : "image/jpeg"};base64,${profile.picture}`}
              />
            )}
            <label>
              Profile picture
              <input
                aria-label="Profile picture"
                type="file"
                accept="image/jpeg,image/png"
                onChange={(e) => void photo(e.target.files?.[0])}
              />
            </label>
            {profile.picture && (
              <button
                type="button"
                className="button small secondary"
                onClick={() => setProfile({ ...profile, picture: null })}
              >
                Remove picture
              </button>
            )}
            <small>
              JPEG or PNG, up to 10 MB and 24 million pixels. Pictures fit
              within 512×512 without enlargement, retaining their aspect ratio.
              Saved as a losslessly compressed PNG with metadata removed.
            </small>
          </div>
          <div className="profile-interest-fields">
            <label>
              Things I like
              <textarea
                aria-label="Things I like"
                rows={5}
                value={likes}
                onChange={(e) => setLikes(e.target.value)}
                placeholder="One artist, genre or interest per line"
              />
            </label>
            <label>
              Things I dislike
              <textarea
                aria-label="Things I dislike"
                rows={5}
                value={dislikes}
                onChange={(e) => setDislikes(e.target.value)}
                placeholder="One topic per line"
              />
            </label>
          </div>
          <p className="hint">
            Up to 100 entries in each list, 80 bytes each. Duplicates are
            removed.
          </p>
          <h3>Upload speed reporting</h3>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={profile.report_upload_speed ?? true}
              onChange={(e) =>
                setProfile({
                  ...profile,
                  report_upload_speed: e.target.checked,
                })
              }
            />
            Report my upload speed to other users
          </label>
          <label>
            Custom reported upload speed (KiB/s)
            <input
              aria-label="Custom reported upload speed (KiB/s)"
              type="number"
              min="1"
              max="1000000"
              step="1"
              placeholder="Automatic"
              disabled={profile.report_upload_speed === false}
              value={profile.reported_upload_speed_kib ?? ""}
              onChange={(e) =>
                setProfile({
                  ...profile,
                  reported_upload_speed_kib:
                    e.target.value === "" ? null : Number(e.target.value),
                })
              }
            />
          </label>
          <p className="hint">
            Leave custom speed blank to use your total upload limit when
            configured, or the measured upload rate otherwise. Automatic speed
            averages the fastest 10% of your most recent 200 distinct upload
            recipients, using each recipient's all-time average. Simultaneous
            files to one person count together. Queueing, handshakes and idle
            gaps between files are excluded. Measurements survive restarts and
            clearing transfer history. At least one recipient is used, rounding
            the 10% up. Until measurements are available, the starting estimate
            is 1 MiB/s. Hiding speed also hides it in search replies; other
            clients may retain older server statistics.
          </p>
          {state.profile?.upload_speed_measured && (
            <p className="hint">
              Based on the fastest{" "}
              {state.profile.upload_speed_selected_recipients ?? 0} of{" "}
              {state.profile.upload_speed_recipients ?? 0} measured recipients.
              Each person has equal weight, regardless of how many files they
              download.
            </p>
          )}
          <p>
            Local{" "}
            {state.profile?.upload_speed_measured
              ? "measured rate"
              : "starting estimate"}
            :{" "}
            <strong>
              {bytes(state.profile?.measured_upload_speed || 1048576)}/s
            </strong>
          </p>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={profile.away_on_startup}
              onChange={(e) =>
                setProfile({ ...profile, away_on_startup: e.target.checked })
              }
            />
            Start in Away mode
          </label>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={idleEnabled}
              onChange={(e) => setIdleEnabled(e.target.checked)}
            />
            Automatically set Away when idle
          </label>
          <label>
            Idle timeout (minutes)
            <input
              type="number"
              min={1}
              max={1440}
              step={1}
              required={idleEnabled}
              disabled={!idleEnabled}
              value={idleMinutes}
              onChange={(e) => setIdleMinutes(e.target.value)}
            />
          </label>
          <p className="hint">
            Clicks, typing, pointer movement and scrolling in a signed-in UI
            keep you active. Activity restores Online after automatic Away.
            Manually selected Away stays in effect until you change it. Applies
            across your signed-in tabs; no input content is sent.
          </p>
          <label>
            Automatic Away reply
            <textarea
              aria-label="Automatic Away reply"
              rows={3}
              maxLength={1024}
              value={profile.away_message}
              onChange={(e) =>
                setProfile({ ...profile, away_message: e.target.value })
              }
              placeholder="Leave blank to disable automatic replies"
            />
          </label>
          <p className="hint">
            Sent in response to a private message while Away, at most once per
            sender every 15 minutes.
          </p>
          <div className="button-row">
            <button className="button primary" disabled={busy || !loaded}>
              {busy ? "Processing and saving…" : "Save profile"}
            </button>
            <button
              type="button"
              className="button secondary"
              disabled={!state.username}
              onClick={() => openUser(state.username)}
            >
              View my profile
            </button>
          </div>
          {requested && (
            <p role="status">
              Profile save requested. The daemon will confirm when saved.
            </p>
          )}
        </fieldset>
      </form>
    </section>
  );
}
