import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
  act,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import Maintenance from "./Maintenance";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});
const status = {
  can_restart: true,
  busy: false,
  pending: false,
  pending_id: null,
  stats: { database_bytes: 8388608, wal_bytes: 0, reusable_bytes: 4194304 },
  stats_error: null,
  last_result: null,
};
it("shows capacity and requests safe supervised compaction, then displays the matching completion", async () => {
  vi.useFakeTimers();
  let complete = false;
  const restarting = vi.fn();
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (_input, options) =>
      new Response(
        JSON.stringify(
          options?.method === "POST"
            ? { id: "job", status: "restart_requested" }
            : complete
              ? {
                  ...status,
                  last_result: {
                    id: "job",
                    status: "completed",
                    message: "Database compacted",
                    before_bytes: 8388608,
                    after_bytes: 4194304,
                    reclaimed_bytes: 4194304,
                    elapsed_ms: 1500,
                  },
                }
              : status,
        ),
      ),
  );
  render(<Maintenance token="" onRestarting={restarting} />);
  await act(async () => {});
  expect(screen.getByText("8 MiB")).toBeVisible();
  fireEvent.click(
    screen.getByRole("button", { name: "Compact database and restart" }),
  );
  await act(async () => {});
  expect(
    JSON.parse(
      vi.mocked(fetch).mock.calls.find((c) => c[1]?.method === "POST")![1]!
        .body as string,
    ),
  ).toEqual({ action: "compact" });
  expect(
    screen.getByRole("button", { name: "Optimize query performance" }),
  ).toBeDisabled();
  complete = true;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3000);
  });
  expect(screen.getByText("Database compacted")).toBeVisible();
  expect(restarting).toHaveBeenLastCalledWith(false);
  expect(
    screen.getByRole("button", { name: "Compact database and restart" }),
  ).not.toBeDisabled();
});
it("disables unsupported or busy jobs and keeps failures dismissible", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async () =>
      new Response(
        JSON.stringify({
          ...status,
          can_restart: false,
          last_result: {
            id: "failure",
            status: "failed",
            message: "Previous data recovered",
          },
        }),
      ),
  );
  render(<Maintenance token="" onRestarting={vi.fn()} />);
  await screen.findByText("Previous data recovered");
  expect(
    screen.getByRole("button", { name: "Compact database and restart" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
  await waitFor(() =>
    expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
  );
});
