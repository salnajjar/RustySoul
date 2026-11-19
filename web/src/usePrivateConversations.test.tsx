import { act, cleanup, renderHook } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, it } from "vitest";
import { emptySnapshot, type Snapshot } from "./types";
import {
  nextPrivateConversation,
  usePrivateConversations,
} from "./usePrivateConversations";

afterEach(() => {
  cleanup();
  localStorage.clear();
});
it("prioritizes the next unread chat and wraps without choosing the closed chat", () => {
  const names = ["Alice", "Bob", "Carol", "Dave"];
  expect(
    nextPrivateConversation(names, "Bob", { alice: 1, bob: 2, dave: 1 }),
  ).toBe("Dave");
  expect(nextPrivateConversation(names, "Dave", { alice: 1, dave: 1 })).toBe(
    "Alice",
  );
  expect(nextPrivateConversation(names, "Bob", { bob: 2 })).toBe("Alice");
});
it("falls back to the chat above, then below, then an empty conversation", () => {
  expect(nextPrivateConversation(["Alice", "Bob", "Carol"], "bOB")).toBe(
    "Alice",
  );
  expect(nextPrivateConversation(["Alice", "Bob"], "Alice")).toBe("Bob");
  expect(nextPrivateConversation(["Alice"], "Alice", { alice: 1 })).toBe("");
  expect(nextPrivateConversation(["Alice"], "Unsent user")).toBe("Alice");
});
const initial: Snapshot = {
  ...emptySnapshot,
  username: "Self",
  messages: ["Alice", "Bob", "Carol"].map((target, i) => ({
    id: i + 1,
    target,
    username: target,
    private: true,
    timestamp: i,
    text: `Saved ${target}`,
  })),
};
it("keeps a closed chat hidden after remount, preserves messages, and permits manual reopening", () => {
  const first = renderHook(() => {
    const [selected, select] = useState("Bob");
    return {
      selected,
      ...usePrivateConversations(initial, [], selected, select),
    };
  });
  act(() => first.result.current.close());
  expect(first.result.current.selected).toBe("Alice");
  expect(first.result.current.names).toEqual(["Alice", "Carol"]);
  expect(initial.messages).toHaveLength(3);
  first.unmount();
  const second = renderHook(
    ({ selected }) => usePrivateConversations(initial, [], selected, () => {}),
    { initialProps: { selected: "" } },
  );
  expect(second.result.current.names).toEqual(["Alice", "Carol"]);
  second.rerender({ selected: "bOB" });
  expect(second.result.current.names).toContain("Bob");
  second.rerender({ selected: "" });
  expect(second.result.current.names).toContain("Bob");
});
it("reopens on genuinely new archived or live messages, but not stale archive responses", () => {
  localStorage.setItem(
    "rustysoul.closedChats.self",
    JSON.stringify({ bob: 3 }),
  );
  const first = renderHook(
    ({ archive }) => usePrivateConversations(initial, archive, "", () => {}),
    {
      initialProps: {
        archive: [{ target: "BOB", private: true, latest_id: 3 }],
      },
    },
  );
  expect(first.result.current.names).toEqual(["Alice", "Carol"]);
  first.rerender({ archive: [{ target: "BOB", private: true, latest_id: 4 }] });
  expect(first.result.current.names).toEqual(["Alice", "Bob", "Carol"]);
  first.unmount();
  expect(
    JSON.parse(localStorage.getItem("rustysoul.closedChats.self")!),
  ).toEqual({});
  localStorage.setItem(
    "rustysoul.closedChats.self",
    JSON.stringify({ bob: 3 }),
  );
  const live = {
    ...initial,
    messages: [...initial.messages, { ...initial.messages[1], id: 5 }],
  };
  const second = renderHook(() =>
    usePrivateConversations(live, [], "", () => {}),
  );
  expect(second.result.current.names).toContain("Bob");
});
it("scopes closed chats to the account and ignores corrupt storage or ignored-user arrivals", () => {
  localStorage.setItem(
    "rustysoul.closedChats.self",
    JSON.stringify({ bob: 2 }),
  );
  localStorage.setItem("rustysoul.closedChats.other", "corrupt");
  const ignored = {
    ...initial,
    user_rules: [
      { username: "Bob", ignored: true, banned: false, buddy: false },
    ],
  };
  const hook = renderHook(
    ({ state }) =>
      usePrivateConversations(
        state,
        [{ target: "Bob", private: true, latest_id: 99 }],
        "",
        () => {},
      ),
    { initialProps: { state: ignored } },
  );
  expect(hook.result.current.names).not.toContain("Bob");
  hook.rerender({ state: { ...ignored, username: "Other" } });
  expect(hook.result.current.names).toContain("Bob");
  hook.rerender({ state: ignored });
  expect(hook.result.current.names).not.toContain("Bob");
});
