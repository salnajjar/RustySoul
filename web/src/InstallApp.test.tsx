import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import InstallApp, { InstallAppProvider } from "./InstallApp";

beforeEach(() => {
  vi.stubGlobal("isSecureContext", true);
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete (navigator as Navigator & { standalone?: boolean }).standalone;
});

function offer(
  prompt = vi.fn().mockResolvedValue(undefined),
  outcome = "dismissed",
) {
  const event = Object.assign(
    new Event("beforeinstallprompt", { cancelable: true }),
    {
      prompt,
      userChoice: Promise.resolve({ outcome }),
    },
  );
  act(() => {
    window.dispatchEvent(event);
  });
  return event;
}

it("retains a prompt received before Appearance opens and consumes it only once", async () => {
  const view = render(
    <InstallAppProvider>
      <div>Search page</div>
    </InstallAppProvider>,
  );
  const event = offer();
  expect(event.defaultPrevented).toBe(true);
  view.rerender(
    <InstallAppProvider>
      <InstallApp />
    </InstallAppProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Install RustySoul" }));
  await waitFor(() =>
    expect(screen.queryByRole("button")).not.toBeInTheDocument(),
  );
  expect(event.prompt).toHaveBeenCalledTimes(1);
  expect(
    screen.getByText(/Use your browser’s Install app/),
  ).toBeInTheDocument();
});

it("reflects an actual installation event and removes its listeners on unmount", () => {
  const media = {
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  vi.stubGlobal("matchMedia", () => media);
  const view = render(
    <InstallAppProvider>
      <InstallApp />
    </InstallAppProvider>,
  );
  offer();
  act(() => {
    window.dispatchEvent(new Event("appinstalled"));
  });
  expect(
    screen.getByText("RustySoul is installed on this device."),
  ).toBeInTheDocument();
  expect(screen.queryByRole("button")).toBeNull();
  view.unmount();
  expect(media.removeEventListener).toHaveBeenCalledWith(
    "change",
    expect.any(Function),
  );
  expect(offer().defaultPrevented).toBe(false);
});

it("recognises standalone launches including the iOS standalone indicator", () => {
  Object.defineProperty(navigator, "standalone", {
    configurable: true,
    value: true,
  });
  render(
    <InstallAppProvider>
      <InstallApp />
    </InstallAppProvider>,
  );
  expect(
    screen.getByText("RustySoul is installed on this device."),
  ).toBeInTheDocument();
});

it("explains HTTPS requirements instead of offering a broken install prompt", () => {
  vi.stubGlobal("isSecureContext", false);
  render(
    <InstallAppProvider>
      <InstallApp />
    </InstallAppProvider>,
  );
  expect(offer().defaultPrevented).toBe(false);
  expect(screen.queryByRole("button")).toBeNull();
  expect(
    screen.getByText(/For installation, open RustySoul over HTTPS/),
  ).toBeInTheDocument();
});

it("handles a rejected install prompt without retaining an unusable event", async () => {
  render(
    <InstallAppProvider>
      <InstallApp />
    </InstallAppProvider>,
  );
  const prompt = vi.fn().mockRejectedValue(new Error("Install not available"));
  offer(prompt);
  fireEvent.click(screen.getByRole("button", { name: "Install RustySoul" }));
  expect(await screen.findByRole("status")).toHaveTextContent(
    "Could not open the install prompt",
  );
  expect(screen.queryByRole("button")).toBeNull();
});
