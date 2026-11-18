import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import TransferSpeed from "./TransferSpeed";

const kb = 1024;
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
function advance(ms: number) {
  act(() => {
    vi.advanceTimersByTime(ms);
  });
}
const value = () =>
  screen.getByTitle("Average transfer speed over the last 5 seconds");

it("weights irregular speed changes by time and expires old measurements after five seconds", () => {
  const { rerender } = render(<TransferSpeed speed={100 * kb} />);
  expect(value()).toHaveTextContent("100 KiB/s");
  advance(4000);
  rerender(<TransferSpeed speed={500 * kb} />);
  expect(value()).toHaveTextContent("100 KiB/s");
  advance(1000);
  expect(value()).toHaveTextContent("180 KiB/s");
  rerender(<TransferSpeed speed={0} />);
  advance(1000);
  expect(value()).toHaveTextContent("160 KiB/s");
  advance(4000);
  expect(value()).toHaveTextContent("0 B/s");
});

it("does not bias averages with repeated responses or parent renders", () => {
  const { rerender } = render(<TransferSpeed speed={100 * kb} />);
  advance(3000);
  for (let i = 0; i < 20; i++) rerender(<TransferSpeed speed={100 * kb} />);
  rerender(<TransferSpeed speed={300 * kb} />);
  advance(2000);
  expect(value()).toHaveTextContent("180 KiB/s");
  expect(vi.getTimerCount()).toBe(1);
});

it("waits for complete data, retains the average during loading, and cleans up on unmount", () => {
  const { rerender, unmount } = render(<TransferSpeed />);
  advance(10000);
  expect(value()).toHaveTextContent("0 B/s");
  rerender(<TransferSpeed speed={200 * kb} />);
  expect(value()).toHaveTextContent("200 KiB/s");
  advance(2000);
  rerender(<TransferSpeed />);
  advance(1000);
  expect(value()).toHaveTextContent("200 KiB/s");
  unmount();
  expect(vi.getTimerCount()).toBe(0);
  render(<TransferSpeed speed={50 * kb} />);
  expect(value()).toHaveTextContent("50 KiB/s");
});

it("handles invalid rates as stopped and uses a monotonic clock", () => {
  const { rerender } = render(<TransferSpeed speed={100 * kb} />);
  advance(5000);
  vi.setSystemTime(new Date("2000-01-01"));
  rerender(<TransferSpeed speed={NaN} />);
  advance(5000);
  expect(value()).toHaveTextContent("0 B/s");
  rerender(<TransferSpeed speed={-100} />);
  expect(value()).toHaveTextContent("0 B/s");
});
