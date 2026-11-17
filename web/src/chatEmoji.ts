// Display only: storage and Soulseek wire messages retain the sender's text.
// These replacements become real text nodes, so native selection/copy yields
// Unicode emoji that can be pasted straight back into either chat composer.
const shortcodes: Record<string, string> = {
  grinning: "😀",
  smiley: "😃",
  smile: "😄",
  grin: "😁",
  laughing: "😆",
  sweat_smile: "😅",
  joy: "😂",
  rofl: "🤣",
  slightly_smiling_face: "🙂",
  upside_down_face: "🙃",
  wink: "😉",
  blush: "😊",
  heart_eyes: "😍",
  smiling_face_with_three_hearts: "🥰",
  kissing_heart: "😘",
  sunglasses: "😎",
  thinking: "🤔",
  thinking_face: "🤔",
  hugging_face: "🤗",
  star_struck: "🤩",
  partying_face: "🥳",
  sleeping: "😴",
  confused: "😕",
  cry: "😢",
  sob: "😭",
  open_mouth: "😮",
  grimacing: "😬",
  pensive: "😔",
  angry: "😠",
  rage: "😡",
  exploding_head: "🤯",
  innocent: "😇",
  shushing_face: "🤫",
  stuck_out_tongue: "😛",
  stuck_out_tongue_winking_eye: "😜",
  thumbsup: "👍",
  thumbs_up: "👍",
  "+1": "👍",
  thumbsdown: "👎",
  thumbs_down: "👎",
  "-1": "👎",
  clap: "👏",
  raised_hands: "🙌",
  pray: "🙏",
  wave: "👋",
  ok_hand: "👌",
  muscle: "💪",
  handshake: "🤝",
  heart: "❤️",
  broken_heart: "💔",
  blue_heart: "💙",
  green_heart: "💚",
  purple_heart: "💜",
  yellow_heart: "💛",
  fire: "🔥",
  star: "⭐",
  sparkles: "✨",
  "100": "💯",
  white_check_mark: "✅",
  check: "✅",
  x: "❌",
  warning: "⚠️",
  eyes: "👀",
  musical_note: "🎵",
  notes: "🎶",
  headphones: "🎧",
  microphone: "🎤",
  guitar: "🎸",
  drum: "🥁",
  saxophone: "🎷",
  musical_keyboard: "🎹",
  cd: "💿",
  coffee: "☕",
  beer: "🍺",
  beers: "🍻",
  tada: "🎉",
  birthday: "🎂",
  gift: "🎁",
  rocket: "🚀",
  trophy: "🏆",
  rainbow: "🌈",
  sunflower: "🌻",
};
const faces: Record<string, string> = {
  ":)": "🙂",
  ":-)": "🙂",
  ":]": "🙂",
  "=)": "🙂",
  ";)": "😉",
  ";-)": "😉",
  ":D": "😄",
  ":-D": "😄",
  ":d": "😄",
  ":-d": "😄",
  ":P": "😛",
  ":-P": "😛",
  ":p": "😛",
  ":-p": "😛",
  ";P": "😜",
  ";p": "😜",
  ":(": "🙁",
  ":-(": "🙁",
  ":[": "🙁",
  ":'(": "😢",
  ":'-(": "😢",
  ":O": "😮",
  ":-O": "😮",
  ":o": "😮",
  ":-o": "😮",
  ":/": "😕",
  ":-/": "😕",
  ":|": "😐",
  ":-|": "😐",
  ">:(": "😠",
  ">:-(": "😠",
  "<3": "❤️",
  "</3": "💔",
};
const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const forms = Object.keys(faces)
  .sort((a, b) => b.length - a.length)
  .map(escape)
  .join("|");
// Conservative boundaries avoid changing filenames, paths, punctuation or code.
const tokens = new RegExp(
  `(^|[\\s([{])(:[a-zA-Z0-9_+\\-]+:|${forms})(?=$|[\\s,!?;)\\]}]|\\.(?:$|\\s))`,
  "g",
);
const protectedText =
  /(`{3}[\s\S]*?(?:`{3}|$)|`[^`\r\n]*`|(?:https?:\/\/|ftp:\/\/|www\.)[^\s<>]+|[a-z]:[\\/][^\s<>]+|[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,})/gi;

export function displayChatEmoji(text: string): string {
  return text
    .split(protectedText)
    .map((part, index) =>
      index % 2
        ? part
        : part.replace(
            tokens,
            (_match, prefix: string, token: string) =>
              prefix +
              (faces[token] ??
                shortcodes[token.slice(1, -1).toLowerCase()] ??
                token),
          ),
    )
    .join("");
}
