import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import Privileges, {
  privilegeDuration,
  privilegeLabel,
  PrivilegeBadge,
} from "./Privileges";
import { emptySnapshot, type Snapshot } from "./types";
const state = (change: Partial<Snapshot> = {}): Snapshot => ({
  ...emptySnapshot,
  connection: "connected",
  username: "A & B/中",
  credentials_configured: true,
  privileges: {
    official_server: true,
    remaining_seconds: 176400,
    checked_at: 1700000000,
    expires_at: 1700176400,
    pending: false,
    watching: false,
    error: null,
  },
  ...change,
});
const clipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  if (clipboard) Object.defineProperty(navigator, "clipboard", clipboard);
  else Reflect.deleteProperty(navigator, "clipboard");
});
it("shows balance, safely prefills the official purchase page and copies the network username", async () => {
  const copy = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: copy },
  });
  render(<Privileges token="session" state={state()} />);
  expect(screen.getByText("Privileges active")).toBeVisible();
  expect(screen.getByText("2 days 1 hour")).toBeVisible();
  const link = screen.getByRole("link", { name: "Extend Soulseek privileges" });
  const url = new URL(link.getAttribute("href")!);
  expect(url.origin + url.pathname).toBe("https://www.slsknet.org/qtlogin.php");
  expect(url.searchParams.get("username")).toBe("A & B/中");
  expect(link).toHaveAttribute("rel", "noopener noreferrer");
  fireEvent.click(
    screen.getByRole("button", { name: "Copy Soulseek username" }),
  );
  await screen.findByRole("button", { name: "Username copied" });
  expect(copy).toHaveBeenCalledWith("A & B/中");
});
it("keeps disconnected and unknown status distinct from confirmed zero and links the badge to settings", () => {
  const disconnected = state({ connection: "disconnected" }),
    open = vi.fn();
  render(
    <>
      <PrivilegeBadge state={disconnected} open={open} />
      <Privileges token="" state={disconnected} />
    </>,
  );
  expect(
    screen.getByRole("button", { name: "Refresh privileges" }),
  ).toBeDisabled();
  expect(
    screen.queryByRole("link", { name: /Get|Extend/ }),
  ).not.toBeInTheDocument();
  fireEvent.click(
    screen.getByRole("button", {
      name: "Soulseek privileges: Privileges not checked",
    }),
  );
  expect(open).toHaveBeenCalledOnce();
  expect(privilegeLabel(state({ privileges: undefined }))).toBe(
    "Privileges not checked",
  );
  expect(
    privilegeLabel(
      state({
        privileges: {
          ...state().privileges!,
          remaining_seconds: 0,
          expires_at: null,
        },
      }),
    ),
  ).toBe("No active privileges");
  expect(
    privilegeLabel(
      state({ privileges: { ...state().privileges!, remaining_seconds: 0 } }),
    ),
  ).toContain("expired");
});
it("rechecks once on return from purchase and requests bounded activation watching", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async () => new Response(JSON.stringify({ status: "queued" })),
    );
  render(<Privileges token="session" state={state()} />);
  fireEvent.click(
    screen.getByRole("link", { name: "Extend Soulseek privileges" }),
  );
  fireEvent(window, new Event("focus"));
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(2));
  for (const call of fetcher.mock.calls)
    expect(JSON.parse(call[1]!.body as string)).toEqual({
      type: "check_privileges",
      watch_for_activation: true,
    });
});
it("supports refresh, dismissible failures and a clipboard fallback", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(
      async () => new Response(JSON.stringify({ status: "queued" })),
    );
  render(
    <Privileges
      token="session"
      state={state({
        privileges: { ...state().privileges!, error: "Server check timed out" },
      })}
    />,
  );
  expect(screen.getByRole("alert")).toHaveTextContent("last successful check");
  fireEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Refresh privileges" }));
  await waitFor(() => expect(fetcher).toHaveBeenCalledOnce());
  expect(JSON.parse(fetcher.mock.calls[0][1]!.body as string)).toEqual({
    type: "check_privileges",
    watch_for_activation: false,
  });
  Reflect.deleteProperty(navigator, "clipboard");
  fireEvent.click(
    screen.getByRole("button", { name: "Copy Soulseek username" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("Select and copy");
});
it("avoids official purchase links for custom servers and formats small balances", () => {
  render(
    <Privileges
      token=""
      state={state({
        privileges: { ...state().privileges!, official_server: false },
      })}
    />,
  );
  expect(screen.queryByRole("link")).not.toBeInTheDocument();
  expect(screen.getByText(/custom Soulseek server/)).toBeVisible();
  expect(privilegeDuration(1)).toBe("Less than a minute");
  expect(privilegeDuration(60)).toBe("1 minute");
  expect(privilegeDuration(3660)).toBe("1 hour 1 minute");
});
