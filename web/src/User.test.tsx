import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import User, { UserContext } from "./User";
import { emptySnapshot } from "./types";
afterEach(cleanup);
it("opens user options without toggling the enclosing result group", () => {
  const open = vi.fn();
  const parent = vi.fn();
  render(
    <UserContext.Provider value={{ state: emptySnapshot, open }}>
      <div onClick={parent}>
        <User username="alice" />
      </div>
    </UserContext.Provider>,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "User options for alice" }),
  );
  expect(open).toHaveBeenCalledWith("alice");
  expect(parent).not.toHaveBeenCalled();
  expect(screen.getByText("Unknown")).toBeInTheDocument();
});
it("does not show stale online status while disconnected", () => {
  const state = {
    ...emptySnapshot,
    users: {
      alice: {
        status: "online",
        exists: true,
        pending: false,
        error: null,
        average_speed: null,
        shared_files: null,
        shared_folders: null,
        description: null,
        upload_slots: null,
        queue_size: null,
        slots_free: null,
        profile_pending: false,
        profile_error: null,
      },
    },
  };
  const { rerender } = render(
    <UserContext.Provider
      value={{ state: { ...state, connection: "connected" }, open: () => {} }}
    >
      <User username="alice" />
    </UserContext.Provider>,
  );
  expect(screen.getByText("Online")).toBeInTheDocument();
  rerender(
    <UserContext.Provider value={{ state, open: () => {} }}>
      <User username="alice" />
    </UserContext.Provider>,
  );
  expect(screen.getByText("Unknown")).toBeInTheDocument();
  expect(screen.queryByText("Online")).not.toBeInTheDocument();
});
