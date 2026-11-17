import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import SystemStatus, { duration } from "./SystemStatus";
import {
  emptySnapshot,
  type Snapshot,
  type SystemStatus as Status,
} from "./types";

afterEach(cleanup);
export const systemFixture = (): Status => ({
  sampled_at: Math.floor(Date.now() / 1000),
  uptime_seconds: 93784,
  process: {
    resident_bytes: 128 * 1024 ** 2,
    threads: 74,
    cpu_seconds: 125,
    cpu_percent: 12.5,
  },
  limits: {
    memory_bytes: 512 * 1024 ** 2,
    memory_used_bytes: 480 * 1024 ** 2,
    memory_scope: "self",
    tasks: 128,
    tasks_used: 77,
    tasks_scope: "self",
    cpu_cores: 2,
  },
  data: {
    path: "/var/lib/rustysoul",
    total_bytes: 100 * 1024 ** 3,
    used_bytes: 50 * 1024 ** 3,
    available_bytes: 45 * 1024 ** 3,
    error: null,
  },
  downloads: {
    path: "/downloads",
    total_bytes: 1000 * 1024 ** 3,
    used_bytes: 960 * 1024 ** 3,
    available_bytes: 30 * 1024 ** 3,
    error: null,
  },
  database_bytes: 32 * 1024 ** 2,
  wal_bytes: 4 * 1024 ** 2,
  download_transfers: {
    total: 316,
    states: {
      completed: 300,
      downloading: 5,
      waiting: 5,
      failed: 5,
      paused: 1,
    },
    speed: 615,
  },
  upload_transfers: {
    total: 12,
    states: { completed: 10, uploading: 1, blocked: 1 },
    speed: 8192,
  },
  process_error: null,
  database_error: null,
});
const props = () => ({
  state: {
    ...emptySnapshot,
    system: systemFixture(),
    connection: "connected",
    shares: { files: 94064, folders: 3000 },
    effective_peer_limit: 16,
    download_capacity_used: 11,
    transfer_capacity: 4096,
  } as Snapshot,
  live: true,
  openLogs: vi.fn(),
  openNetwork: vi.fn(),
  openLibrary: vi.fn(),
});
it("shows process, cgroup, disk, uptime and whole-history totals with clear limits", () => {
  render(<SystemStatus {...props()} />);
  expect(screen.getByText(/Up 1d 2h 3m/)).toBeVisible();
  expect(screen.getByText("12.5%")).toBeVisible();
  expect(screen.getByText("128 MiB")).toBeVisible();
  expect(
    screen.getByRole("meter", { name: "Cgroup memory usage" }),
  ).toHaveAttribute("value", "93.75");
  expect(screen.getByText(/Near the limit/)).toBeVisible();
  expect(screen.getByText(/Low free space/)).toBeVisible();
  const downloads = screen.getByRole("heading", {
    name: "Downloads",
  }).parentElement!;
  expect(within(downloads).getByText("316")).toBeVisible();
  expect(within(downloads).getByText("300")).toBeVisible();
  expect(screen.getByText("/downloads")).toBeVisible();
});
it("leaves unavailable data explicit instead of inventing zero measurements", () => {
  const p = props();
  delete p.state.system;
  render(<SystemStatus {...p} />);
  expect(screen.getByText("Waiting for the first sample")).toBeVisible();
  expect(
    screen.queryByRole("meter", { name: "Cgroup memory usage" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getAllByText("Transfer statistics are not available yet."),
  ).toHaveLength(2);
  expect(screen.queryByText("0 B")).not.toBeInTheDocument();
});
it("distinguishes stale samples and interrupted browser updates", () => {
  const p = props();
  p.state.system!.sampled_at -= 40;
  const { rerender } = render(<SystemStatus {...p} />);
  expect(screen.getByText("System readings delayed")).toBeVisible();
  rerender(<SystemStatus {...p} live={false} />);
  expect(screen.getByText("Live updates interrupted")).toBeVisible();
});
it("renders probe errors literally and links to existing configuration and logs", () => {
  const p = props();
  p.state.system!.data.error = "<script>window.bad=true</script>";
  p.state.system!.limits.memory_scope = "ancestor";
  const { container } = render(<SystemStatus {...p} />);
  expect(screen.getByText(/Capacity unavailable: <script>/)).toBeVisible();
  expect(container.querySelector("script")).toBeNull();
  expect(screen.getByText(/Shared parent budget/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "View logs & activity" }));
  expect(p.openLogs).toHaveBeenCalledOnce();
  fireEvent.click(
    screen.getByRole("button", { name: "Open network settings" }),
  );
  expect(p.openNetwork).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Open file locations" }));
  expect(p.openLibrary).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByText("External monitoring"));
  expect(screen.getByRole("link", { name: "View metrics" })).toHaveAttribute(
    "href",
    "/api/v1/metrics",
  );
});
it("formats uptime without treating a daemon restart as an all-time counter", () => {
  expect(duration(0)).toBe("0s");
  expect(duration(61)).toBe("1m 1s");
  expect(duration(3601)).toBe("1h 0m");
  expect(duration(-1)).toBe("0s");
});
