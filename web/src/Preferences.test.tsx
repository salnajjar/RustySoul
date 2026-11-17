import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import Appearance, { usePreferences } from "./Preferences";
function Harness() {
  const [preferences, setPreferences] = usePreferences();
  return <Appearance preferences={preferences} onChange={setPreferences} />;
}
afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.restoreAllMocks();
  delete document.documentElement.dataset.theme;
});
describe("browser appearance preferences", () => {
  it("keeps the original theme by default and persists non-sensitive choices", () => {
    render(<Harness />);
    expect(document.documentElement.dataset.theme).toBe("original");
    fireEvent.change(screen.getByLabelText("Theme"), {
      target: { value: "dark" },
    });
    fireEvent.click(screen.getByLabelText("Show library update banner"));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(JSON.parse(localStorage.getItem("rustysoul.preferences")!)).toEqual({
      theme: "dark",
      libraryBanner: false,
    });
    cleanup();
    render(<Harness />);
    expect(
      screen.getByLabelText("Show library update banner"),
    ).not.toBeChecked();
    expect(screen.getByLabelText("Theme")).toHaveValue("dark");
  });
  it("follows operating system changes and removes the listener on unmount", () => {
    let changed: () => void = () => {};
    const media = {
      matches: true,
      addEventListener: vi.fn((_type, fn) => {
        changed = fn;
      }),
      removeEventListener: vi.fn(),
    };
    Object.defineProperty(window, "matchMedia", {
      value: vi.fn(() => media),
      configurable: true,
    });
    render(<Harness />);
    fireEvent.change(screen.getByLabelText("Theme"), {
      target: { value: "system" },
    });
    expect(document.documentElement.dataset.theme).toBe("dark");
    media.matches = false;
    changed();
    expect(document.documentElement.dataset.theme).toBe("light");
    cleanup();
    expect(media.removeEventListener).toHaveBeenCalled();
    delete (window as unknown as { matchMedia?: unknown }).matchMedia;
  });
  it("recovers from invalid stored preferences", () => {
    localStorage.setItem("rustysoul.preferences", "{broken");
    render(<Harness />);
    expect(screen.getByLabelText("Theme")).toHaveValue("original");
  });
});
