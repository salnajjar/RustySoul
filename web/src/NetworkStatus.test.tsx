import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import NetworkStatus from "./NetworkStatus";
import { emptySnapshot } from "./types";

afterEach(cleanup);
it("shows actual presence and sends manual presence and disconnect commands", () => {
  const send = vi.fn().mockResolvedValue(true);
  const state = {
    ...emptySnapshot,
    connection: "connected",
    username: "myself",
    away: true,
    away_automatic: true,
  };
  const { rerender } = render(
    <NetworkStatus
      state={state}
      busy={false}
      send={send}
      openAccounts={vi.fn()}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Soulseek status: Away (idle)" }),
  );
  expect(screen.getByRole("button", { name: "Away" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Online" }));
  expect(send).toHaveBeenLastCalledWith({ type: "set_away", away: false });
  expect(screen.queryByRole("region")).not.toBeInTheDocument();
  rerender(
    <NetworkStatus
      state={{ ...state, away: false, away_automatic: false }}
      busy={false}
      send={send}
      openAccounts={vi.fn()}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Soulseek status: Online" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Away" }));
  expect(send).toHaveBeenLastCalledWith({ type: "set_away", away: true });
  rerender(
    <NetworkStatus
      state={{ ...state, away_automatic: false }}
      busy={false}
      send={send}
      openAccounts={vi.fn()}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Soulseek status: Away" }),
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Disconnect from Soulseek" }),
  );
  expect(send).toHaveBeenLastCalledWith({ type: "disconnect" });
});

it("closes with Escape and outside clicks, and disables busy actions", () => {
  const state = { ...emptySnapshot, connection: "connected" };
  render(
    <NetworkStatus
      state={state}
      busy={true}
      send={vi.fn()}
      openAccounts={vi.fn()}
    />,
  );
  const trigger = screen.getByRole("button", {
    name: "Soulseek status: Online",
  });
  fireEvent.click(trigger);
  expect(screen.getByRole("button", { name: "Online" })).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Disconnect from Soulseek" }),
  ).toBeDisabled();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(trigger).toHaveFocus();
  expect(trigger).toHaveAttribute("aria-expanded", "false");
  fireEvent.click(trigger);
  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole("region")).not.toBeInTheDocument();
});

it("offers reconnection or account setup when disconnected", () => {
  const send = vi.fn().mockResolvedValue(true),
    accounts = vi.fn();
  const state = { ...emptySnapshot, credentials_configured: true };
  const { rerender } = render(
    <NetworkStatus
      state={state}
      busy={false}
      send={send}
      openAccounts={accounts}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Soulseek status: Offline" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Connect to Soulseek" }));
  expect(send).toHaveBeenCalledWith({ type: "connect" });
  rerender(
    <NetworkStatus
      state={{ ...state, credentials_configured: false }}
      busy={false}
      send={send}
      openAccounts={accounts}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Soulseek status: Offline" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Sign in to Soulseek" }));
  expect(accounts).toHaveBeenCalledOnce();
});
