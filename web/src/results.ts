import type { FileResult } from "./types";
export type AudioFormat = "all" | "lossless" | "lossy";
export type UserSort = "matches" | "speed" | "name" | "available";
export type FileSort =
  | "name"
  | "name_desc"
  | "smallest"
  | "largest"
  | "bitrate"
  | "duration"
  | "sample_rate"
  | "bit_depth";
export function compareFiles(a: FileResult, b: FileResult, sort: FileSort) {
  let result = 0;
  switch (sort) {
    case "name_desc":
      return b.path.localeCompare(a.path);
    case "smallest":
      result = a.size - b.size;
      break;
    case "largest":
      result = b.size - a.size;
      break;
    case "bitrate":
      result = (b.attributes["0"] || 0) - (a.attributes["0"] || 0);
      break;
    case "duration":
      result =
        (a.attributes["1"] ?? Infinity) - (b.attributes["1"] ?? Infinity);
      break;
    case "sample_rate":
      result = (b.attributes["4"] || 0) - (a.attributes["4"] || 0);
      break;
    case "bit_depth":
      result = (b.attributes["5"] || 0) - (a.attributes["5"] || 0);
      break;
  }
  return result || a.path.localeCompare(b.path);
}
const lossless = new Set([
  "flac",
  "wav",
  "wave",
  "aiff",
  "aif",
  "alac",
  "ape",
  "wv",
  "tta",
  "tak",
  "dsf",
  "dff",
]);
const lossy = new Set([
  "mp3",
  "aac",
  "opus",
  "ogg",
  "oga",
  "wma",
  "mp2",
  "ac3",
]);
export function audioFormat(path: string): AudioFormat | "unknown" {
  const ext = path.split(".").at(-1)?.toLowerCase() || "";
  return lossless.has(ext) ? "lossless" : lossy.has(ext) ? "lossy" : "unknown";
}
export function parentFolder(path: string): string {
  const normalized = path.replaceAll("\\", "/");
  const offset = normalized.lastIndexOf("/");
  return offset < 0 ? "Root folder" : normalized.slice(0, offset);
}
export function groupResults(
  files: FileResult[],
  filter: string,
  format: AudioFormat,
  userSort: UserSort = "matches",
  fileSort: FileSort = "name",
) {
  const users = new Map<string, Map<string, FileResult[]>>();
  const seen = new Set<string>();
  const query = filter.toLowerCase();
  for (const file of files) {
    if (
      !`${file.path} ${file.username}`.toLowerCase().includes(query) ||
      (format !== "all" && audioFormat(file.path) !== format)
    )
      continue;
    const id = JSON.stringify([file.username, file.path]);
    if (seen.has(id)) continue;
    seen.add(id);
    let folders = users.get(file.username);
    if (!folders) {
      folders = new Map();
      users.set(file.username, folders);
    }
    const parent = parentFolder(file.path);
    let entries = folders.get(parent);
    if (!entries) {
      entries = [];
      folders.set(parent, entries);
    }
    entries.push(file);
  }
  return [...users]
    .map(([username, folders]) => ({
      username,
      speed: Math.max(
        0,
        ...[...folders.values()].flatMap((files) => files.map((f) => f.speed)),
      ),
      available: [...folders.values()].some((files) =>
        files.some((f) => f.free_slots > 0),
      ),
      count: [...folders.values()].reduce((n, f) => n + f.length, 0),
      folders: [...folders]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([path, files]) => ({
          path,
          files: files.sort((a, b) => compareFiles(a, b, fileSort)),
        })),
    }))
    .sort(
      (a, b) =>
        (userSort === "name"
          ? a.username.localeCompare(b.username)
          : userSort === "speed"
            ? b.speed - a.speed
            : userSort === "available"
              ? Number(b.available) - Number(a.available)
              : b.count - a.count) ||
        b.count - a.count ||
        a.username.localeCompare(b.username),
    );
}
