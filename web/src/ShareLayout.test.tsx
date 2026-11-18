import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import ShareLayout from "./ShareLayout";

const key = "rustysoul.shareFolders.width";
const layout = () =>
  render(
    <ShareLayout folders={<aside id="share-folders-pane">Folders</aside>}>
      <div>Files</div>
    </ShareLayout>,
  );
beforeEach(() => {
  localStorage.clear();
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 1200,
  } as DOMRect);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("supports keyboard resizing, bounds and reset and restores width after remount", () => {
  const view = layout();
  const separator = screen.getByRole("separator", {
    name: "Resize folders pane",
  });
  expect(separator).toHaveAttribute("aria-valuenow", "240");
  fireEvent.keyDown(separator, { key: "ArrowRight" });
  expect(localStorage.getItem(key)).toBe("256");
  fireEvent.keyDown(separator, { key: "ArrowRight", shiftKey: true });
  expect(separator).toHaveAttribute("aria-valuenow", "320");
  view.unmount();
  layout();
  const restored = screen.getByRole("separator");
  expect(restored).toHaveAttribute("aria-valuenow", "320");
  fireEvent.keyDown(restored, { key: "Home" });
  fireEvent.keyDown(restored, { key: "ArrowLeft" });
  expect(restored).toHaveAttribute("aria-valuenow", "180");
  fireEvent.keyDown(restored, { key: "End" });
  fireEvent.keyDown(restored, { key: "ArrowRight" });
  expect(restored).toHaveAttribute("aria-valuenow", "720");
  fireEvent.doubleClick(restored);
  expect(localStorage.getItem(key)).toBe("240");
});

it("constrains the pane to available space without replacing the saved preference", () => {
  localStorage.setItem(key, "600");
  layout();
  const separator = screen.getByRole("separator");
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 800,
  } as DOMRect);
  act(() => {
    window.dispatchEvent(new Event("resize"));
  });
  expect(separator).toHaveAttribute("aria-valuenow", "444");
  expect(localStorage.getItem(key)).toBe("600");
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 1200,
  } as DOMRect);
  act(() => {
    window.dispatchEvent(new Event("resize"));
  });
  expect(separator).toHaveAttribute("aria-valuenow", "600");
  act(() => {
    window.dispatchEvent(new StorageEvent("storage", { key, newValue: "350" }));
  });
  expect(separator).toHaveAttribute("aria-valuenow", "350");
});

it("uses a safe default for corrupt preferences and still resizes when storage is unavailable", () => {
  localStorage.setItem(key, "Infinity");
  const view = layout();
  expect(screen.getByRole("separator")).toHaveAttribute("aria-valuenow", "240");
  view.unmount();
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw Error("Blocked");
  });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw Error("Blocked");
  });
  layout();
  fireEvent.keyDown(screen.getByRole("separator"), { key: "ArrowRight" });
  expect(screen.getByRole("separator")).toHaveAttribute("aria-valuenow", "256");
});
