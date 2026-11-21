import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import ConnectionSettings from "./ConnectionSettings";
import { api } from "./api";
import { emptySnapshot } from "./types";

vi.mock("./api", () => ({
  api: vi.fn().mockResolvedValue({ status: "queued" }),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("edits and saves a manual per-user download maximum while automatic mode was enabled", async () => {
  render(<ConnectionSettings state={emptySnapshot} token="fixture" />);
  const maximum = screen.getByLabelText("Maximum downloads per user", {
    exact: true,
  });
  const automatic = screen.getByLabelText(
    "Automatically use simultaneous downloads offered by a user",
    { exact: true },
  );
  expect(maximum).toBeEnabled();
  expect(maximum).toHaveValue(emptySnapshot.network.download_slots);
  fireEvent.change(maximum, { target: { value: "2" } });
  expect(automatic).not.toBeChecked();
  expect(maximum).toHaveValue(2);
  fireEvent.click(
    screen.getByRole("button", {
      name: "Save connection settings",
    }),
  );
  await waitFor(() =>
    expect(api).toHaveBeenCalledWith(
      "fixture",
      "commands",
      expect.objectContaining({
        type: "update_network",
        settings: expect.objectContaining({
          download_slots_per_user: 2,
          automatic_download_parallelism: false,
        }),
      }),
    ),
  );
  fireEvent.click(automatic);
  expect(maximum).toHaveValue(emptySnapshot.network.download_slots);
  fireEvent.click(automatic);
  expect(maximum).toHaveValue(2);
});

it("keeps automatic mode enabled when editing other transfer limits", () => {
  render(<ConnectionSettings state={emptySnapshot} token="fixture" />);
  for (const label of [
    "Maximum simultaneous uploads",
    "Maximum simultaneous downloads",
    "Maximum uploads per user",
  ]) {
    fireEvent.change(screen.getByLabelText(label, { exact: true }), {
      target: { value: "6" },
    });
  }
  expect(
    screen.getByLabelText(
      "Automatically use simultaneous downloads offered by a user",
      { exact: true },
    ),
  ).toBeChecked();
  expect(
    screen.getByLabelText("Maximum downloads per user", { exact: true }),
  ).toHaveValue(6);
});
