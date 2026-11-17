import { Fragment } from "react";
import type { Message } from "./types";
import User from "./User";
import { displayChatEmoji } from "./chatEmoji";
import { groupChatMessages } from "./chatMessages";

export default function ConversationMessages({
  messages,
}: {
  messages: Message[];
}) {
  return groupChatMessages(messages).map((group, index) => {
    const first = group.messages[0];
    return (
      <div
        className="message"
        data-message-id={first.id}
        key={first.id ?? index}
      >
        <span className="message-avatar">
          {first.username.slice(0, 2).toUpperCase()}
        </span>
        <div>
          <strong>
            <User username={first.username} />
            <time>
              {new Date(first.timestamp * 1000).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              })}
            </time>
          </strong>
          <p
            className={`chat-message${group.artwork ? " chat-artwork" : ""}`}
            tabIndex={group.artwork ? 0 : undefined}
          >
            {group.messages.map((message, i) => (
              <Fragment key={message.id ?? i}>
                {i > 0 && "\n"}
                <span
                  data-message-id={message.id}
                  title={new Date(message.timestamp * 1000).toLocaleString()}
                >
                  {group.artwork
                    ? message.text
                    : displayChatEmoji(message.text)}
                </span>
              </Fragment>
            ))}
          </p>
        </div>
      </div>
    );
  });
}
