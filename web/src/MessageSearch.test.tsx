import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import MessageSearch from "./MessageSearch";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("searches persisted text and sender together and pages older matches", async () => {
  const fetcher = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          messages: [
            {
              id: 200,
              target: "club",
              username: "Alice",
              text: "Saved older jazz",
              timestamp: 1,
              private: false,
            },
          ],
          has_more: true,
        }),
      ),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          messages: [
            {
              id: 2,
              target: "club",
              username: "Alice",
              text: "Earliest jazz",
              timestamp: 1,
              private: false,
            },
          ],
          has_more: false,
        }),
      ),
    );
  render(
    <MessageSearch
      token="token"
      target="club"
      privateChat={false}
      visibility="1"
      close={() => {}}
    />,
  );
  fireEvent.change(screen.getByLabelText("Search message text"), {
    target: { value: " jazz " },
  });
  fireEvent.change(screen.getByLabelText("Search messages from user"), {
    target: { value: "Alice" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Search saved messages" }),
  );
  await screen.findByText("Saved older jazz");
  const first = new URL(String(fetcher.mock.calls[0][0]), "http://localhost");
  expect(Object.fromEntries(first.searchParams)).toEqual({
    target: "club",
    private: "false",
    filter: "jazz",
    username: "Alice",
  });
  fireEvent.click(screen.getByRole("button", { name: "Load older matches" }));
  await screen.findByText("Earliest jazz");
  expect(String(fetcher.mock.calls[1][0])).toContain("before_id=200");
  expect(screen.getByText("Saved older jazz")).toBeInTheDocument();
  expect(fetcher.mock.calls.every(([, options]) => !options?.body)).toBe(true);
});
it("drops stale results when ignore rules or history change", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(
      JSON.stringify({
        messages: [
          {
            id: 2,
            target: "club",
            username: "Alice",
            text: "Old visible result",
            timestamp: 1,
            private: false,
          },
        ],
        has_more: false,
      }),
    ),
  );
  const props = {
    token: "token",
    target: "club",
    privateChat: false,
    visibility: "1",
    close: () => {},
  };
  const { rerender } = render(<MessageSearch {...props} />);
  fireEvent.change(screen.getByLabelText("Search messages from user"), {
    target: { value: "Alice" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Search saved messages" }),
  );
  await screen.findByText("Old visible result");
  rerender(<MessageSearch {...props} visibility="2" />);
  await waitFor(() =>
    expect(screen.queryByText("Old visible result")).not.toBeInTheDocument(),
  );
});
