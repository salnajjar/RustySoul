import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useTransferFeed } from "./useTransferFeed";
import type { Transfer } from "./types";
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});
const rows = (user: string, count: number): Transfer[] =>
  Array.from({ length: count }, (_, i) => ({
    username: user,
    path: `${user}-${i}.flac`,
    size: 100,
    transferred: 100,
    speed: 0,
    status: "completed",
    queue_position: null,
  }));
const response = (transfers: Transfer[], total: number, offset = 0) =>
  new Response(
    JSON.stringify({
      transfers,
      total,
      offset,
      next_offset: offset + transfers.length,
      summary: { total, active: 0, speed: 0, clearable: { all: total } },
    }),
  );

it("appends past the complete user boundary without refetching or duplicate admissions", async () => {
  const urls: URL[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (url) => {
    const q = new URL(String(url), "http://localhost");
    urls.push(q);
    return q.searchParams.get("offset") === "0"
      ? response(rows("alice", 150), 230)
      : response(rows("bob", 80), 230, 150);
  });
  const { result } = renderHook(() =>
    useTransferFeed("", "uploads", "group_users=true", true, 1, 230),
  );
  await waitFor(() => expect(result.current.data?.transfers).toHaveLength(150));
  act(() => {
    result.current.load();
    result.current.load();
  });
  await waitFor(() => expect(result.current.data?.transfers).toHaveLength(230));
  expect(urls.map((u) => u.searchParams.get("offset"))).toEqual(["0", "150"]);
  expect(new Set(result.current.data!.transfers.map((t) => t.path)).size).toBe(
    230,
  );
});

it("refreshes the loaded window after live insertions and removals, retaining data on retryable failures", async () => {
  vi.useFakeTimers();
  let files = rows("alice", 300),
    broken = false;
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (url) => {
      if (broken) throw new Error("Temporarily unavailable");
      const q = new URL(String(url), "http://localhost").searchParams;
      const offset = Number(q.get("offset")),
        limit = Number(q.get("limit"));
      return response(
        files.slice(offset, offset + limit),
        files.length,
        offset,
      );
    });
  const { result, rerender } = renderHook(
    ({ revision }) =>
      useTransferFeed(
        "",
        "downloads",
        "group_users=false",
        true,
        revision,
        300,
      ),
    { initialProps: { revision: 1 } },
  );
  await act(async () => {});
  await act(async () => {
    result.current.load();
  });
  expect(result.current.data?.transfers).toHaveLength(200);
  files = [
    ...rows("bob", 1),
    ...files.filter((t) => t.path !== "alice-0.flac"),
  ];
  rerender({ revision: 2 });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(result.current.data?.transfers).toHaveLength(200);
  expect(result.current.data?.transfers[0].username).toBe("bob");
  expect(
    result.current.data?.transfers.some((t) => t.path === "alice-0.flac"),
  ).toBe(false);
  broken = true;
  rerender({ revision: 3 });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(result.current.error).toBe("Temporarily unavailable");
  expect(result.current.data?.transfers).toHaveLength(200);
  const attempts = fetcher.mock.calls.length;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(10000);
  });
  expect(fetcher).toHaveBeenCalledTimes(attempts);
  broken = false;
  await act(async () => {
    result.current.retry();
  });
  expect(result.current.error).toBe("");
  expect(result.current.data?.transfers).toHaveLength(200);
});

it("aborts stale criteria and ignores late responses rather than mixing filtered users", async () => {
  let resolveOld!: (response: Response) => void;
  let oldSignal: AbortSignal | undefined;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (url, options) => {
    if (String(url).includes("alice")) {
      oldSignal = options?.signal as AbortSignal;
      return new Promise<Response>((resolve) => {
        resolveOld = resolve;
      });
    }
    return response(rows("bob", 2), 2);
  });
  const { result, rerender } = renderHook(
    ({ filter, active }) =>
      useTransferFeed("", "uploads", `filter=${filter}`, active, 1, 2),
    { initialProps: { filter: "alice", active: true } },
  );
  rerender({ filter: "bob", active: true });
  await waitFor(() =>
    expect(result.current.data?.transfers[0].username).toBe("bob"),
  );
  expect(oldSignal?.aborted).toBe(true);
  await act(async () => {
    resolveOld(response(rows("alice", 2), 2));
  });
  expect(
    result.current.data?.transfers.every((t) => t.username === "bob"),
  ).toBe(true);
});
