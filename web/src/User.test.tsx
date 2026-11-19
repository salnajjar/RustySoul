import { afterEach, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import User, { UserContext, UserDialog } from "./User";
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

it("shows ignore and share-ban controls immediately, preserves both choices and rolls back rejected saves", async () => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.setAttribute("open", "");
    },
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.removeAttribute("open");
    },
  });
  const send = vi.fn().mockResolvedValueOnce(true).mockResolvedValueOnce(false);
  const state = {
    ...emptySnapshot,
    username: "self",
    user_rules: [{ username: "peer", ignored: false, banned: true }],
  };
  const props = {
    username: "Peer",
    state,
    close: () => {},
    send,
    browse: () => {},
    message: () => {},
  };
  const { rerender } = render(<UserDialog {...props} />);
  const ignore = screen.getByRole("checkbox", {
    name: "Ignore messages from this user",
  });
  const ban = screen.getByRole("checkbox", {
    name: "Ban this user from my shares",
  });
  expect(ignore).toBeVisible();
  expect(ban).toBeChecked();
  fireEvent.click(ignore);
  expect(ignore).toBeDisabled();
  rerender(
    <UserDialog
      {...props}
      state={{
        ...state,
        user_rules: [{ username: "peer", ignored: true, banned: true }],
      }}
    />,
  );
  await waitFor(() => expect(ignore).toBeEnabled());
  expect(ignore).toBeChecked();
  expect(send).toHaveBeenNthCalledWith(1, {
    type: "set_user_rule",
    username: "Peer",
    ignored: true,
    banned: true,
  });
  fireEvent.click(ban);
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent("Could not save"),
  );
  expect(ban).toBeChecked();
  expect(ignore).toBeChecked();
  expect(send).toHaveBeenNthCalledWith(2, {
    type: "set_user_rule",
    username: "Peer",
    ignored: true,
    banned: false,
  });
});

it("does not treat a queued rule as confirmed when no daemon update arrives", async () => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.setAttribute("open", "");
    },
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.removeAttribute("open");
    },
  });
  vi.useFakeTimers();
  try {
    render(
      <UserDialog
        username="Peer"
        state={emptySnapshot}
        close={() => {}}
        send={async () => true}
        browse={() => {}}
        message={() => {}}
      />,
    );
    const ignore = screen.getByRole("checkbox", {
      name: "Ignore messages from this user",
    });
    await act(async () => {
      fireEvent.click(ignore);
    });
    expect(ignore).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Saving user controls",
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(8001);
    });
    expect(ignore).not.toBeChecked();
    expect(ignore).toBeEnabled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not confirm user controls",
    );
  } finally {
    vi.useRealTimers();
  }
});
