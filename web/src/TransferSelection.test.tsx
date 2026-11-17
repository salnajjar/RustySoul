import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import {
  cancelTransferBatches,
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
