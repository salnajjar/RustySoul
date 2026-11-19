import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import HistoryDelete from "./HistoryDelete";
afterEach(cleanup);
function setup() {
  const send = vi.fn().mockResolvedValue(true),
    close = vi.fn();
  render(
    <HistoryDelete target="alice" privateChat send={send} close={close} />,
  );
  return { send, close };
}
it("keeps all-history commands compatible and scopes them to this conversation", async () => {
  const { send, close } = setup();
  fireEvent.click(
    screen.getByRole("button", { name: "Delete history permanently" }),
  );
  await waitFor(() => expect(close).toHaveBeenCalledOnce());
  expect(send).toHaveBeenCalledWith({
    type: "clear_message_history",
    target: "alice",
    private: true,
  });
});
it("sends an age cutoff and leaves history untouched on cancellation", async () => {
  const { send, close } = setup();
  fireEvent.change(screen.getByLabelText("Messages to delete"), {
    target: { value: "older_than" },
  });
  fireEvent.change(screen.getByLabelText("Older than (days)"), {
    target: { value: "90" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Delete history permanently" }),
  );
  await waitFor(() => expect(close).toHaveBeenCalledOnce());
  expect(send).toHaveBeenCalledWith({
    type: "clear_message_history",
    target: "alice",
    private: true,
    range: { mode: "older_than", days: 90 },
  });
});
it("converts local date ranges to seconds and rejects inverted bounds", async () => {
  const { send } = setup();
  fireEvent.change(screen.getByLabelText("Messages to delete"), {
    target: { value: "between" },
  });
  fireEvent.change(screen.getByLabelText("Delete messages from"), {
    target: { value: "2026-11-10T10:00" },
  });
  fireEvent.change(screen.getByLabelText("Delete messages before"), {
    target: { value: "2026-11-09T10:00" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Delete history permanently" }),
  );
  expect(send).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toHaveTextContent("end after the start");
  fireEvent.change(screen.getByLabelText("Delete messages before"), {
    target: { value: "2026-11-11T10:00" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Delete history permanently" }),
  );
  await waitFor(() =>
    expect(send).toHaveBeenCalledWith(
      expect.objectContaining({
        range: {
          mode: "between",
          from: new Date("2026-11-10T10:00").getTime() / 1000,
          to: new Date("2026-11-11T10:00").getTime() / 1000,
        },
      }),
    ),
  );
});
it("does not close or clear anything when the command fails", async () => {
  const { send, close } = setup();
  send.mockResolvedValue(false);
  fireEvent.click(
    screen.getByRole("button", { name: "Delete history permanently" }),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Delete history permanently" }),
    ).toBeEnabled(),
  );
  expect(close).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(close).toHaveBeenCalledOnce();
  expect(send).toHaveBeenCalledOnce();
});
