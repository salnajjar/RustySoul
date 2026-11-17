import { expect, it } from "vitest";
import { groupChatMessages, isChatArtwork } from "./chatMessages";
import type { Message } from "./types";
const message = (change: Partial<Message> = {}): Message => ({
  id: 1,
  username: "alice",
  target: "club",
  private: false,
  text: "Hello",
  timestamp: 100,
  ...change,
});
it("joins rapid consecutive messages using the previous message time, without changing originals", () => {
  const messages = [
    message(),
    message({ id: 2, timestamp: 159, text: "  second  " }),
    message({ id: 3, timestamp: 218, text: "third\n\nline" }),
  ];
  const before = JSON.stringify(messages);
  const groups = groupChatMessages(messages);
  expect(groups).toHaveLength(1);
  expect(groups[0].messages.map((m) => m.text).join("\n")).toBe(
    "Hello\n  second  \nthird\n\nline",
  );
  expect(JSON.stringify(messages)).toBe(before);
});
it("starts fresh after a minute or a sender, conversation or direction change", () => {
  for (const change of [
    { timestamp: 160 },
    { username: "bob" },
    { target: "another" },
    { private: true },
  ])
    expect(groupChatMessages([message(), message(change)])).toHaveLength(2);
  expect(
    groupChatMessages([
      message(),
      message({ timestamp: 159 }),
      message({ timestamp: 219 }),
    ]),
  ).toHaveLength(2);
});
it("never sorts messages or joins reversed, unknown or invalid timestamps", () => {
  for (const timestamp of [99, 0, NaN, Infinity])
    expect(groupChatMessages([message(), message({ timestamp })])).toHaveLength(
      2,
    );
  expect(
    groupChatMessages([message({ timestamp: 0 }), message({ timestamp: 0 })]),
  ).toHaveLength(2);
  expect(
    groupChatMessages([message(), message({ timestamp: 100 })]),
  ).toHaveLength(1);
  expect(groupChatMessages([])).toEqual([]);
});
it("recognizes block banners and traditional multiline drawings without mistaking ordinary chat for art", () => {
  expect(isChatArtwork("░██████╗██████╗░")).toBe(true);
  expect(isChatArtwork(" /\\_/\\\n( o.o )\n > ^ <")).toBe(true);
  for (const text of [
    "Thanks :) :heart:",
    "https://example.org/",
    "A regular sentence!",
    "👩🏽‍🎤 🎵",
    ":)\n:D",
  ])
    expect(isChatArtwork(text)).toBe(false);
});
it("detects drawings spanning distinct messages and preserves explicit blank lines", () => {
  const groups = groupChatMessages([
    message({ text: " /\\_/\\" }),
    message({ id: 2, text: "( o.o )", timestamp: 101 }),
    message({ id: 3, text: " > ^ <", timestamp: 102 }),
  ]);
  expect(groups[0].artwork).toBe(true);
  expect(groups[0].messages.map((m) => m.text).join("\n")).toBe(
    " /\\_/\\\n( o.o )\n > ^ <",
  );
});
