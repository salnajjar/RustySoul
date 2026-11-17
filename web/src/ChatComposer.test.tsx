import { useState } from "react";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import ChatComposer from "./ChatComposer";
import { insertEmoji, messageBytes } from "./emoji";

afterEach(cleanup);
function Harness({
  send,
  initial = "hello music",
  disabled = false,
}: {
  send: (c: object) => unknown;
  initial?: string;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState(initial);
  return (
    <ChatComposer
      draft={draft}
      setDraft={setDraft}
      send={send}
      target="club"
      privateChat={false}
      disabled={disabled}
    />
  );
}
it("inserts Unicode at the caret without sending or altering surrounding text", async () => {
  const send = vi.fn();
  render(<Harness send={send} />);
  const input = screen.getByLabelText("Message") as HTMLInputElement;
  input.focus();
  input.setSelectionRange(6, 11);
  fireEvent.select(input);
  fireEvent.mouseDown(screen.getByLabelText("Insert emoji"));
  fireEvent.click(screen.getByLabelText("Insert emoji"));
  fireEvent.change(screen.getByLabelText("Search emoji"), {
    target: { value: ":thumbsup:" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "thumbs up thumbsup 👍" }),
  );
  expect(input.value).toBe("hello 👍");
  expect(send).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(input.value).toBe(""));
  expect(send).toHaveBeenCalledWith({
    type: "send_message",
    target: "club",
    text: "hello 👍",
    private: false,
  });
});
it("retains a draft after failed submission and allows a retry", async () => {
  const send = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  render(<Harness send={send} initial="🎵 👩🏽‍🎤 ❤️ 🇬🇧" />);
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Send" })).toBeEnabled(),
  );
  expect(screen.getByLabelText("Message")).toHaveValue("🎵 👩🏽‍🎤 ❤️ 🇬🇧");
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(screen.getByLabelText("Message")).toHaveValue(""));
});
it("enforces the network byte limit and preserves drafts while offline", () => {
  render(<Harness send={vi.fn()} initial={"🎵".repeat(501)} />);
  expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  expect(screen.getByRole("alert")).toHaveTextContent("too long");
  expect(messageBytes("🎵".repeat(500))).toBe(2000);
  cleanup();
  render(<Harness send={vi.fn()} initial="offline draft" disabled />);
  expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  expect(screen.getByLabelText("Message")).toHaveValue("offline draft");
});
it("does not split surrogate pairs when replacing a selection", () => {
  expect(insertEmoji("a🎵b", 2, 2, "👍")).toEqual({ text: "a👍b", cursor: 3 });
  expect(insertEmoji("text", -10, 100, "👩🏽‍🎤")).toEqual({
    text: "👩🏽‍🎤",
    cursor: 7,
  });
});
