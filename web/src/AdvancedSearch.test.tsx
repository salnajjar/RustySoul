import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import AdvancedSearch, { matchesCriteria } from "./AdvancedSearch";
import { describeAttributes } from "./FileAttributes";
afterEach(cleanup);
const file = {
  username: "peer",
  path: "a.flac",
  size: 1048576,
  speed: 100,
  free_slots: 1,
  attributes: { "1": 125, "4": 96000, "5": 24 },
};
it("filters known attributes and never treats missing attributes as zero", () => {
  expect(
    matchesCriteria(file, {
      min_sample_rate: 48000,
      max_bit_depth: 24,
      min_size: 1048576,
      max_size: 1048576,
      free_slots: true,
    }),
  ).toBe(true);
  expect(matchesCriteria(file, { min_bitrate: 0 })).toBe(false);
  expect(matchesCriteria(file, { vbr: false })).toBe(false);
  expect(matchesCriteria(file, { max_duration: 124 })).toBe(false);
  expect(
    matchesCriteria(
      { ...file, attributes: { "0": 320, "1": 125, "2": 0 } },
      { min_bitrate: 320, max_bitrate: 320, vbr: false },
    ),
  ).toBe(true);
});
it("applies ranges in wire units, rejects inverted ranges and clears them explicitly", () => {
  const change = vi.fn();
  render(<AdvancedSearch value={{}} change={change} />);
  fireEvent.change(screen.getByLabelText("Minimum File size (MiB)"), {
    target: { value: "1.5" },
  });
  fireEvent.change(screen.getByLabelText("Minimum Sample rate (Hz)"), {
    target: { value: "48000" },
  });
  fireEvent.change(screen.getByLabelText("Maximum Sample rate (Hz)"), {
    target: { value: "44100" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Apply advanced filters" }),
  );
  expect(change).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toHaveTextContent("minimum");
  fireEvent.change(screen.getByLabelText("Maximum Sample rate (Hz)"), {
    target: { value: "96000" },
  });
  fireEvent.change(screen.getByLabelText("Bitrate mode"), {
    target: { value: "cbr" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Apply advanced filters" }),
  );
  expect(change).toHaveBeenLastCalledWith({
    min_size: 1572864,
    min_sample_rate: 48000,
    max_sample_rate: 96000,
    vbr: false,
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Clear advanced filters" }),
  );
  expect(change).toHaveBeenLastCalledWith({});
});
it("renders supported attributes including zero duration without inventing missing ones", () => {
  expect(describeAttributes(file.attributes)).toBe("24-bit · 96 kHz · 2:05");
  expect(describeAttributes({ "0": 320, "1": 0, "2": 1 })).toBe(
    "320 kbps · VBR · 0:00",
  );
  expect(describeAttributes({})).toBe("");
});
