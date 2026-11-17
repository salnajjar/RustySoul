import type { Message } from "./types";

export const CHAT_GROUP_GAP_SECONDS = 60;
export interface ChatGroup {
  messages: Message[];
  artwork: boolean;
}

// Keep wire/database messages distinct and in their original arrival order.
export function groupChatMessages(messages: readonly Message[]): ChatGroup[] {
  const groups: ChatGroup[] = [];
  for (const message of messages) {
    const last = groups.at(-1);
    const previous = last?.messages.at(-1);
    const gap = previous ? message.timestamp - previous.timestamp : NaN;
    if (
      previous &&
      message.username === previous.username &&
      message.target === previous.target &&
      message.private === previous.private &&
      Number.isFinite(message.timestamp) &&
      Number.isFinite(previous.timestamp) &&
      previous.timestamp > 0 &&
      gap >= 0 &&
      gap < CHAT_GROUP_GAP_SECONDS
    ) {
      last!.messages.push(message);
    } else {
      groups.push({ messages: [message], artwork: false });
    }
  }
  for (const group of groups) {
    group.artwork = isChatArtwork(group.messages.map((m) => m.text).join("\n"));
  }
  return groups;
}

export function isChatArtwork(text: string): boolean {
  const lines = text.split(/\r?\n/);
  // Block/box-drawing banners, including messages containing a single art row.
  if (lines.some((line) => (line.match(/[\u2500-\u259f]/gu)?.length ?? 0) >= 2))
    return true;
  // Conservative fallback for traditional punctuation drawings. A single smiley,
  // prose, named emoji, URL or code fragment should retain its normal rendering.
  return (
    lines.filter((line) => {
      const content = line.trim();
      return (
        content.length >= 4 &&
        /^[\s/\\|_~^()<>{}\[\]`'".:+*#%@=\-]+$/.test(content)
      );
    }).length >= 2
  );
}
