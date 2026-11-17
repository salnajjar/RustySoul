import { expect, it } from "vitest";
import { displayChatEmoji } from "./chatEmoji";

it("displays common ASCII emoticons as Unicode emoji", () => {
  expect(
    displayChatEmoji("Thanks :) ;-) :D :P :( :'( :o :/ :| >:( <3 </3"),
  ).toBe("Thanks 🙂 😉 😄 😛 🙁 😢 😮 😕 😐 😠 ❤️ 💔");
});
it("recognizes named aliases, case and punctuation without changing unknown names", () => {
  expect(
    displayChatEmoji(
      "(:smile:), :THUMBSUP:! :+1: :musical_note:. :not_a_known_emoji:",
    ),
  ).toBe("(😄), 👍! 👍 🎵. :not_a_known_emoji:");
});
it("preserves already graphical and combined emoji, including copied replies", () => {
  const text = "🙂 😄 👍🏽 👩🏽‍🎤 ❤️ 🇬🇧";
  expect(displayChatEmoji(text)).toBe(text);
  expect(
    displayChatEmoji(displayChatEmoji("Hello :) :heart: :musical_note:")),
  ).toBe("Hello 🙂 ❤️ 🎵");
});
it("leaves links, email addresses, code, drive paths and embedded filename text intact", () => {
  const text =
    "https://example.org/:smile: http://host/:) www.example.org/:heart: ftp://host/:D C:\\Music\\:smile: test:smile:@example.org `:) :heart:` ```\n:)\n:smile:\n``` foo:smile:bar album:)track.flac";
  expect(displayChatEmoji(text)).toBe(text);
});
it("does not consume spaces, newlines or punctuation when converting adjacent tokens", () => {
  expect(displayChatEmoji(":)  :D\n\t:heart: :smile:!")).toBe(
    "🙂  😄\n\t❤️ 😄!",
  );
  expect(displayChatEmoji("ratio 3:2 normal syntax x:=y and :unknown:")).toBe(
    "ratio 3:2 normal syntax x:=y and :unknown:",
  );
});
it("returns inert text rather than parsing user supplied HTML", () => {
  expect(
    displayChatEmoji(
      '<script>alert("x")</script> :) <img src=x onerror=alert(1)>',
    ),
  ).toBe('<script>alert("x")</script> 🙂 <img src=x onerror=alert(1)>');
});
