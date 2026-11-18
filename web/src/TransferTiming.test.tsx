import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import TransferTiming, { duration } from "./TransferTiming";
import type { Transfer } from "./types";

const start = 1795000000;
const transfer = (change: Partial<Transfer> = {}): Transfer => ({
  username: "peer",
  path: "album/track.flac",
  size: 100,
  transferred: 50,
  speed: 10,
  status: "downloading",
  queue_position: null,
  started_at: start,
  finished_at: null,
  ...change,
});
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime((start + 65) * 1000);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("uses one shared live clock for uploads and downloads and freezes terminal totals", () => {
  const { rerender } = render(
    <>
      <TransferTiming transfer={transfer()} />
      <TransferTiming transfer={transfer({ status: "uploading" })} />
    </>,
  );
  expect(screen.getAllByText("Elapsed 00:01:05")).toHaveLength(2);
  expect(vi.getTimerCount()).toBe(1);
  act(() => {
    vi.advanceTimersByTime(3000);
  });
  expect(screen.getAllByText("Elapsed 00:01:08")).toHaveLength(2);
  rerender(
    <>
      <TransferTiming
        transfer={transfer({ status: "completed", finished_at: start + 68 })}
      />
      <TransferTiming
        transfer={transfer({ status: "timed_out", finished_at: start + 67 })}
      />
    </>,
  );
  expect(vi.getTimerCount()).toBe(0);
  act(() => {
    vi.advanceTimersByTime(120000);
  });
  expect(screen.getByText("Total time 00:01:08")).toBeInTheDocument();
  expect(screen.getByText("Total time 00:01:07")).toBeInTheDocument();
  expect(
    screen.getAllByText((_, e) => e?.tagName === "TIME")[0],
  ).toHaveAttribute("dateTime", new Date(start * 1000).toISOString());
});

it("preserves the same start across pause/wait and starts fresh on a manual retry", () => {
  const { rerender } = render(
    <TransferTiming transfer={transfer({ status: "paused" })} />,
  );
  act(() => {
    vi.advanceTimersByTime(2000);
  });
  expect(screen.getByText("Elapsed 00:01:07")).toBeInTheDocument();
  rerender(<TransferTiming transfer={transfer({ status: "retry_waiting" })} />);
  expect(screen.getByText("Elapsed 00:01:07")).toBeInTheDocument();
  rerender(
    <TransferTiming
      transfer={transfer({ status: "cancelled", finished_at: start + 67 })}
    />,
  );
  expect(vi.getTimerCount()).toBe(0);
  rerender(
    <TransferTiming
      transfer={transfer({ status: "queued", started_at: null })}
    />,
  );
  expect(screen.getByText("Not started")).toBeInTheDocument();
  rerender(<TransferTiming transfer={transfer({ started_at: start + 67 })} />);
  expect(screen.getByText("Elapsed 00:00:00")).toBeInTheDocument();
});

it("does not invent historical start times or keep incomplete terminal metadata ticking", () => {
  const { rerender } = render(
    <TransferTiming
      transfer={transfer({
        status: "completed",
        started_at: null,
        added_at: start - 100,
      })}
    />,
  );
  expect(screen.getByText("Timing unavailable")).toBeInTheDocument();
  expect(document.querySelector("time")).toBeNull();
  rerender(<TransferTiming transfer={transfer({ status: "failed" })} />);
  expect(screen.getByText("Duration unavailable")).toBeInTheDocument();
  expect(vi.getTimerCount()).toBe(0);
  rerender(
    <TransferTiming transfer={transfer({ started_at: Number.MAX_VALUE })} />,
  );
  expect(screen.getByText("Timing unavailable")).toBeInTheDocument();
});

it("formats zero, days and clock corrections without negative counters", () => {
  expect(duration(0)).toBe("00:00:00");
  expect(duration(-10)).toBe("00:00:00");
  expect(duration(90061)).toBe("1d 01:01:01");
});
