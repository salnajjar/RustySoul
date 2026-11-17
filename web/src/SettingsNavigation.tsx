export const settingsTabs = [
  ["accounts", "Accounts"],
  ["profile", "Profile"],
  ["appearance", "Appearance"],
  ["people", "People & sharing"],
  ["library", "File locations & library"],
  ["network", "Network settings"],
  ["status", "Status & privacy"],
  ["logs", "Logs & activity"],
  ["api", "API access"],
  ["maintenance", "Database maintenance"],
  ["backups", "Backup & restore"],
] as const;
export type SettingsTab = (typeof settingsTabs)[number][0];
export default function SettingsNavigation({
  selected,
  onChange,
}: {
  selected: SettingsTab;
  onChange: (tab: SettingsTab) => void;
}) {
  return (
    <div
      role="tablist"
      aria-label="Settings sections"
      className="settings-tabs"
      onKeyDown={(e) => {
        const i = settingsTabs.findIndex((t) => t[0] === selected);
        const n =
          e.key === "ArrowRight"
            ? (i + 1) % settingsTabs.length
            : e.key === "ArrowLeft"
              ? (i + settingsTabs.length - 1) % settingsTabs.length
              : e.key === "Home"
                ? 0
                : e.key === "End"
                  ? settingsTabs.length - 1
                  : -1;
        if (n >= 0) {
          e.preventDefault();
          onChange(settingsTabs[n][0]);
          document
            .getElementById("settings-tab-" + settingsTabs[n][0])
            ?.focus();
        }
      }}
    >
      {settingsTabs.map(([id, label]) => (
        <button
          key={id}
          type="button"
          role="tab"
          id={"settings-tab-" + id}
          aria-controls="settings-content"
          aria-selected={selected === id}
          tabIndex={selected === id ? 0 : -1}
          onClick={() => onChange(id)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}
