import { describe, it, expect } from "vitest";
import {
  audioFormat,
  groupResults,
  parentFolder,
  compareFiles,
} from "./results";
import { sortRooms } from "./rooms";
import type { FileResult } from "./types";
function file(username: string, path: string): FileResult {
  return {
    username,
    path,
    size: 100,
    free_slots: 1,
    speed: 10,
    attributes: {},
  };
}
describe("live search grouping", () => {
  it("sorts peer speed, availability, name and file metadata without mutating input", () => {
    const slow = {
      ...file("alice", "Music/z.flac"),
      speed: 10,
      free_slots: 0,
      size: 100,
      attributes: { "0": 128, "1": 200 },
    };
    const fast = {
      ...file("bob", "Music/a.flac"),
      speed: 100,
      size: 200,
      attributes: { "0": 320, "1": 100 },
    };
    expect(
      groupResults([slow, fast], "", "all", "speed").map((g) => g.username),
    ).toEqual(["bob", "alice"]);
    expect(
      groupResults([slow, fast], "", "all", "available").map((g) => g.username),
    ).toEqual(["bob", "alice"]);
    expect(
      groupResults([fast, slow], "", "all", "name").map((g) => g.username),
    ).toEqual(["alice", "bob"]);
    for (const sort of ["name", "largest", "bitrate", "duration"] as const)
      expect(compareFiles(fast, slow, sort)).toBeLessThan(0);
    for (const sort of ["name_desc", "smallest"] as const)
      expect(compareFiles(fast, slow, sort)).toBeGreaterThan(0);
    expect(compareFiles(file("x", "z"), fast, "duration")).toBeGreaterThan(0);
  });
  it("sorts rooms by users, name and observed activity with unknown activity last", () => {
    const rooms = [
      { name: "Zulu", users: 100, last_activity: null },
      { name: "Alpha", users: 10, last_activity: 50 },
      { name: "Beta", users: 20, last_activity: 100 },
    ];
    expect(sortRooms(rooms, "users").map((r) => r.name)).toEqual([
      "Zulu",
      "Beta",
      "Alpha",
    ]);
    expect(sortRooms(rooms, "name").map((r) => r.name)).toEqual([
      "Alpha",
      "Beta",
      "Zulu",
    ]);
    expect(sortRooms(rooms, "activity").map((r) => r.name)).toEqual([
      "Beta",
      "Alpha",
      "Zulu",
    ]);
    expect(rooms[0].name).toBe("Zulu");
  });
  it("orders users by result count and regroups new results without mutating input", () => {
    const initial = [
      file("alice", "Music\\A\\one.flac"),
      file("bob", "Music/B/two.mp3"),
    ];
    expect(groupResults(initial, "", "all").map((g) => g.username)).toEqual([
      "alice",
      "bob",
    ]);
    const updated = [
      ...initial,
      file("bob", "Music/B/three.mp3"),
      file("bob", "Music/C/four.flac"),
    ];
    const groups = groupResults(updated, "", "all");
    expect(groups.map((g) => g.username)).toEqual(["bob", "alice"]);
    expect(groups[0].folders.map((f) => [f.path, f.files.length])).toEqual([
      ["Music/B", 2],
      ["Music/C", 1],
    ]);
    expect(initial).toHaveLength(2);
  });
  it("filters lossless/lossy without assuming ambiguous containers are one or the other", () => {
    const files = [file("a", "x.FLAC"), file("a", "x.mp3"), file("b", "x.m4a")];
    expect(groupResults(files, "", "lossless")[0].count).toBe(1);
    expect(groupResults(files, "", "lossy")[0].count).toBe(1);
    expect(
      groupResults(files, "", "all").reduce((n, g) => n + g.count, 0),
    ).toBe(3);
    expect(audioFormat("x.m4a")).toBe("unknown");
  });
  it("deduplicates the same peer/path while preserving files from distinct peers", () => {
    const f = file("a", "Folder/x.flac");
    const groups = groupResults([f, f, file("b", f.path)], "folder", "all");
    expect(groups.map((g) => g.count)).toEqual([1, 1]);
    expect(parentFolder("Folder\\album\\track.flac")).toBe("Folder/album");
  });
});
