import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import {
  cancelTransferBatches,
  retryDownloadBatches,
  canCancelTransfer,
  selectTransferRange,
  transferKey,
  useTransferSelection,
} from "./TransferSelection";
import type { Transfer } from "./types";
afterEach(cleanup);
const rows: Transfer[] = Array.from({ length: 350 }, (_, i) => ({
  username: "alice",
  path: `Music/${i}.flac`,
  size: 100,
  transferred: 0,
  speed: 0,
  status: "queued",
  queue_position: null,
}));
it("selects failed and timed-out downloads across filters and preserves them for retry", () => {
  const failed = rows
    .slice(0, 3)
    .map((t, i) => ({
      ...t,
      status: ["failed", "timed_out", "interrupted"][i],
    }));
  const { result, rerender } = renderHook(
    ({ all, order }) => useTransferSelection(all, order, "failed", false),
    { initialProps: { all: failed, order: failed } },
  );
  act(() => result.current.select(failed[0], {}));
  act(() => result.current.select(failed[2], { shiftKey: true }));
  expect(result.current.selected.size).toBe(3);
  expect(
    [...result.current.selected.values()].filter((t) =>
      canCancelTransfer(t, false),
    ),
  ).toHaveLength(1);
  rerender({ all: [failed[1]], order: [failed[1]] });
  expect(result.current.selected.size).toBe(3);
  rerender({ all: [{ ...failed[1], status: "completed" }], order: [] });
  expect(result.current.selected.size).toBe(2);
});
it("splits bulk retries into bounded requests without losing long UTF-8 paths", () => {
  const failed = rows.map((t) => ({
    ...t,
    status: "failed",
    path: t.path + "音楽🎶".repeat(30),
  }));
  const batches = retryDownloadBatches("alice", failed);
  expect(batches.flatMap((b) => b.paths)).toEqual(failed.map((t) => t.path));
  expect(
    batches.every(
      (b) =>
        b.type === "retry_downloads" &&
        b.paths.length <= 256 &&
        new TextEncoder().encode(JSON.stringify(b)).length <= 7500,
    ),
  ).toBe(true);
});
it("uses Ctrl toggles, reverse Shift ranges and additive Ctrl-Shift ranges across unloaded DOM rows", () => {
  let state = selectTransferRange(new Map(), rows, rows[0], null, false, false);
  state = selectTransferRange(
    state.selected,
    rows,
    rows[5],
    state.anchor,
    true,
    false,
  );
  expect(state.selected.size).toBe(2);
  state = selectTransferRange(
    state.selected,
    rows,
    rows[305],
    state.anchor,
    false,
    true,
  );
  expect(state.selected.size).toBe(301);
  expect(state.selected.has(transferKey(rows[0]))).toBe(false);
  expect(state.selected.has(transferKey(rows[190]))).toBe(true);
  state = selectTransferRange(
    state.selected,
    rows,
    rows[2],
    state.anchor,
    true,
    true,
  );
  expect(state.selected.size).toBe(304);
  state = selectTransferRange(
    state.selected,
    rows,
    rows[2],
    state.anchor,
    true,
    false,
  );
  expect(state.selected.has(transferKey(rows[2]))).toBe(false);
});
it("keeps hidden selections, resets range anchors after sorting/filtering and prunes newly completed files", () => {
  const { result, rerender } = renderHook(
    ({ all, order, criterion }) =>
      useTransferSelection(all, order, criterion, false),
    { initialProps: { all: rows, order: rows, criterion: "name" } },
  );
  act(() => result.current.select(rows[0], {}));
  rerender({ all: [rows[300]], order: [rows[300]], criterion: "filter" });
  act(() =>
    result.current.select(rows[300], { ctrlKey: true, shiftKey: true }),
  );
  expect(result.current.selected.size).toBe(2);
  rerender({
    all: [{ ...rows[300], status: "completed" }],
    order: [],
    criterion: "filter",
  });
  expect(result.current.selected.size).toBe(1);
  expect(result.current.selected.has(transferKey(rows[0]))).toBe(true);
});
it("splits large selections by both path count and UTF-8 request bytes without losing filenames", () => {
  const files = rows.map((t, i) => ({
    ...t,
    path: `Music/${i}-` + "音楽🎶".repeat(30) + ".flac",
  }));
  for (const uploads of [false, true]) {
    const batches = cancelTransferBatches(uploads, "alice", files);
    expect(batches.flatMap((b) => b.paths)).toEqual(files.map((t) => t.path));
    expect(
      batches.every(
        (b) =>
          b.paths.length <= 256 &&
          new TextEncoder().encode(JSON.stringify(b)).length <= 7500,
      ),
    ).toBe(true);
    expect(
      batches.every(
        (b) => b.type === (uploads ? "cancel_uploads" : "cancel_downloads"),
      ),
    ).toBe(true);
  }
  expect(() =>
    cancelTransferBatches(false, "alice", [
      { ...rows[0], path: '"'.repeat(4096) },
    ]),
  ).toThrow("request limit");
});
