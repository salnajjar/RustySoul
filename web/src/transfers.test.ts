import { expect, it } from "vitest";
import type { Transfer } from "./types";
import { sortTransfers, progress } from "./transfers";
const file = (path: string, extra: Partial<Transfer> = {}): Transfer => ({
  username: "alice",
  path,
  size: 100,
  transferred: 0,
  speed: 0,
  status: "waiting",
  queue_position: null,
  ...extra,
});
it("sorts live transfers by speed, progress, size, names and status without mutating the queue", () => {
  const files = [
    file("z.flac", { status: "completed", transferred: 100 }),
    file("a.flac", { speed: 10, status: "downloading", transferred: 50 }),
    file("b.flac", { size: 200, username: "bob" }),
  ];
  expect(sortTransfers(files, "activity")[0].path).toBe("a.flac");
  expect(sortTransfers(files, "speed")[0].path).toBe("a.flac");
  expect(sortTransfers(files, "progress")[0].path).toBe("z.flac");
  expect(sortTransfers(files, "size")[0].path).toBe("b.flac");
  expect(sortTransfers(files, "name").map((t) => t.path)).toEqual([
    "a.flac",
    "b.flac",
    "z.flac",
  ]);
  expect(sortTransfers(files, "user", false, "bob")).toHaveLength(1);
  expect(files[0].path).toBe("z.flac");
});
it("keeps unknown times and queue positions last and uses stable ties", () => {
  const files = [
    file("unknown"),
    file("fast", { speed: 20, queue_position: 2 }),
    file("slow", { speed: 10, queue_position: 4 }),
  ];
  expect(sortTransfers(files, "eta").map((t) => t.path)).toEqual([
    "fast",
    "slow",
    "unknown",
  ]);
  expect(sortTransfers(files, "eta", true).map((t) => t.path)).toEqual([
    "slow",
    "fast",
    "unknown",
  ]);
  expect(sortTransfers(files, "queue", true).map((t) => t.path)).toEqual([
    "slow",
    "fast",
    "unknown",
  ]);
  expect(sortTransfers(files, "added", true).map((t) => t.path)).toEqual([
    "slow",
    "fast",
    "unknown",
  ]);
});

it("completed empty files have full progress while queued empty files do not", () => {
  const empty = file("empty.txt", { size: 0, status: "completed" });
  expect(progress(empty)).toBe(1);
  expect(progress({ ...empty, status: "queued" })).toBe(0);
  expect(
    sortTransfers(
      [file("partial.flac", { transferred: 50 }), empty],
      "progress",
    )[0].path,
  ).toBe("empty.txt");
});
