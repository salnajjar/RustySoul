import { useEffect, useState } from "react";
export type Theme = "original" | "light" | "dark" | "system";
export interface Preferences {
  theme: Theme;
  libraryBanner: boolean;
}
const key = "rustysoul.preferences";
export function usePreferences() {
  const [preferences, setPreferences] = useState<Preferences>(() => {
    try {
      const p = JSON.parse(localStorage.getItem(key) || "{}");
      return {
        theme: ["original", "light", "dark", "system"].includes(p.theme)
          ? p.theme
          : "original",
        libraryBanner: p.libraryBanner !== false,
      };
    } catch {
      return { theme: "original", libraryBanner: true };
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(key, JSON.stringify(preferences));
    } catch {
      /* Storage may be unavailable in private browsing. */
    }
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    const apply = () => {
      const theme =
        preferences.theme === "system"
          ? media?.matches
            ? "dark"
            : "light"
          : preferences.theme;
      document.documentElement.dataset.theme = theme;
      document.documentElement.style.colorScheme =
        theme === "dark" ? "dark" : "light";
    };
    apply();
    media?.addEventListener("change", apply);
    return () => media?.removeEventListener("change", apply);
  }, [preferences]);
  return [preferences, setPreferences] as const;
}
export default function Appearance({
  preferences,
  onChange,
}: {
  preferences: Preferences;
  onChange: (p: Preferences) => void;
}) {
  return (
    <section className="panel wide">
      <h2>Appearance & notifications</h2>
      <p>These preferences are saved for this browser.</p>
      <div className="credentials-form">
        <label>
          Theme
          <select
            aria-label="Theme"
            value={preferences.theme}
            onChange={(e) =>
              onChange({ ...preferences, theme: e.target.value as Theme })
            }
          >
            <option value="original">Original — Garden</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
            <option value="system">Follow system</option>
          </select>
        </label>
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={preferences.libraryBanner}
            onChange={(e) =>
              onChange({ ...preferences, libraryBanner: e.target.checked })
            }
          />
          Show library update banner
        </label>
        <p className="hint">
          Library update progress always remains visible in File locations &
          local library.
        </p>
      </div>
    </section>
  );
}
