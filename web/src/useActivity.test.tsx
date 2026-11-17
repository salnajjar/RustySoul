import { renderHook, act, cleanup } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useActivity } from "./useActivity";
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});
it("reports real UI activity with leading/trailing throttling and stops when disabled", async () => {
  vi.useFakeTimers();
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async () => new Response(JSON.stringify({ status: "queued" })),
    );
  const { rerender, unmount } = renderHook(
    ({ enabled, away }) => useActivity("session", enabled, away),
    { initialProps: { enabled: true, away: false } },
  );
  await act(async () => {});
  expect(fetcher).toHaveBeenCalledTimes(1);
  await act(async () => {
    for (let i = 0; i < 50; i++)
      document.dispatchEvent(new Event("pointermove"));
  });
  expect(fetcher).toHaveBeenCalledTimes(1);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(10000);
  });
  expect(fetcher).toHaveBeenCalledTimes(2);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(120000);
  });
  expect(fetcher).toHaveBeenCalledTimes(2);
  rerender({ enabled: true, away: true });
  await act(async () => {
    document.dispatchEvent(new Event("pointerdown"));
  });
  expect(fetcher).toHaveBeenCalledTimes(3);
  expect(JSON.parse(fetcher.mock.calls[2][1]!.body as string)).toEqual({
    type: "ui_activity",
    idle_seconds: 0,
  });
  rerender({ enabled: false, away: false });
  await act(async () => {
    document.dispatchEvent(new Event("keydown"));
    await vi.advanceTimersByTimeAsync(20000);
  });
  expect(fetcher).toHaveBeenCalledTimes(3);
  unmount();
});
it("does not send input contents and does not clear a fresh error by throwing on connection failure", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockRejectedValue(new Error("Offline"));
  const { unmount } = renderHook(() => useActivity("", true, true));
  await act(async () => {});
  expect(fetcher).toHaveBeenCalledOnce();
  const payload = JSON.parse(fetcher.mock.calls[0][1]!.body as string);
  expect(Object.keys(payload).sort()).toEqual(["idle_seconds", "type"]);
  unmount();
});
