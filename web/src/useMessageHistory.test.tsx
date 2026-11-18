import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "./api";
import { useMessageHistory } from "./useMessageHistory";
import type { Message } from "./types";
vi.mock("./api", () => ({ api: vi.fn() }));
const request = vi.mocked(api);
const message = (id: number): Message => ({
  id,
  target: "peer",
  username: "self",
  private: true,
  text: `saved ${id}`,
  timestamp: 1,
});
const flush = async () => {
  await act(async () => {});
};
beforeEach(() => {
  vi.useFakeTimers();
  request.mockReset();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("loads older archives and catches up across gaps larger than the live preview", async () => {
  let end = 750;
  request.mockImplementation(async (_, path) => {
    const q = new URL(path, "http://fixture").searchParams;
    const after = q.get("after_id"),
      before = q.get("before_id");
    let ids = Array.from({ length: end }, (_, i) => i + 1).filter((id) =>
      after ? id > Number(after) : !before || id < Number(before),
    );
    const more = ids.length > 100;
    ids = after ? ids.slice(0, 100) : ids.slice(-100);
    return { messages: ids.map(message), has_more: more };
  });
  const { result, rerender } = renderHook(
    ({ revision }) => useMessageHistory("", "peer", true, revision, 0, []),
    { initialProps: { revision: 1 } },
  );
  await flush();
  expect(result.current.messages[0].id).toBe(651);
  act(() => result.current.loadOlder());
  await flush();
  expect(result.current.messages[0].id).toBe(551);
  expect(result.current.messages).toHaveLength(200);
  end = 1350;
  rerender({ revision: 2 });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(result.current.messages).toHaveLength(800);
  expect(result.current.messages.at(-1)?.id).toBe(1350);
  expect(new Set(result.current.messages.map((m) => m.id)).size).toBe(800);
});

it("does not resurrect deleted history from an in-flight older-page response", async () => {
  let resolve!: (value: unknown) => void;
  request.mockResolvedValueOnce({ messages: [message(10)], has_more: true });
  request.mockImplementationOnce(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  request.mockResolvedValue({ messages: [], has_more: false });
  const { result, rerender } = renderHook(
    ({ reset }) => useMessageHistory("", "peer", true, 1, reset, []),
    { initialProps: { reset: 0 } },
  );
  await flush();
  act(() => result.current.loadOlder());
  const signal = request.mock.calls[1][3];
  rerender({ reset: 1 });
  await flush();
  expect(signal?.aborted).toBe(true);
  await act(async () => {
    resolve({ messages: [message(1)], has_more: false });
  });
  expect(result.current.messages).toEqual([]);
});

it("retains visible history during an error and retries without duplicates", async () => {
  request.mockResolvedValueOnce({ messages: [message(10)], has_more: true });
  request.mockRejectedValueOnce(Error("temporary failure"));
  request.mockResolvedValueOnce({ messages: [message(1)], has_more: false });
  const { result } = renderHook(() =>
    useMessageHistory("", "peer", true, 1, 0, []),
  );
  await flush();
  act(() => result.current.loadOlder());
  await flush();
  expect(result.current.error).toBe("temporary failure");
  expect(result.current.messages).toHaveLength(1);
  act(() => result.current.retry());
  await flush();
  expect(result.current.messages.map((m) => m.id)).toEqual([1, 10]);
  expect(result.current.older).toBe(false);
});
