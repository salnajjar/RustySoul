import { afterEach, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import ConversationMessages from "./ConversationMessages";
import type { Message } from "./types";
afterEach(cleanup);
const message = (id: number, text: string, timestamp: number): Message => ({
  id,
  text,
  timestamp,
  username: "alice",
  target: "club",
  private: false,
});
it("renders one header and exactly one newline between messages, with each read anchor retained", () => {
  const messages = [message(1, "░██╗  ", 100), message(2, "  ╚██╝", 101)];
  const { container } = render(<ConversationMessages messages={messages} />);
  expect(container.querySelectorAll(".message")).toHaveLength(1);
  expect(container.querySelectorAll(".message time")).toHaveLength(1);
  expect(container.querySelector(".chat-message")!.textContent).toBe(
    "░██╗  \n  ╚██╝",
  );
  expect(container.querySelector(".chat-message")).toHaveClass("chat-artwork");
  expect(container.querySelector('[data-message-id="2"]')).toBeInTheDocument();
});
it("keeps ordinary emoji rendering, literal hostile text and a new header after a gap", () => {
  const { container } = render(
    <ConversationMessages
      messages={[
        message(1, "Thanks :)", 100),
        message(2, "<script>literal</script>", 101),
        message(3, "Later :heart:", 161),
      ]}
    />,
  );
  expect(container.querySelectorAll(".message")).toHaveLength(2);
  expect(
    [...container.querySelectorAll(".chat-message")].map((p) => p.textContent),
  ).toEqual(["Thanks 🙂\n<script>literal</script>", "Later ❤️"]);
  expect(container.querySelector("script")).not.toBeInTheDocument();
});
