import type { Snapshot } from "./types";

// Match Soulseek/SQLite's ASCII-insensitive username comparisons.
export const userKey = (username: string) =>
  username.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
export const isIgnored = (state: Snapshot, username: string) =>
  state.user_rules.some(
    (rule) => rule.ignored && userKey(rule.username) === userKey(username),
  );
export const unreadTotal = (counts?: Record<string, number>) =>
  Object.values(counts || {}).reduce((sum, count) => sum + count, 0);
