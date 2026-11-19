import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api } from "./api";
import type { Message } from "./types";

export function useRoomRead({
  enabled,
  room,
  messages,
  readTo,
  token,
  acknowledge = true,
  privateChat = false,
}: {
  enabled: boolean;
  room: string;
  messages: Message[];
  readTo: number;
  token: string;
  acknowledge?: boolean;
  privateChat?: boolean;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const end = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const previous = useRef("");
  const visible = useRef(false);
  const pending = useRef({ key: "", at: 0 });
  const [bottom, setBottom] = useState(true);
  const latest = messages.at(-1)?.id || 0;
  const preserved = useRef<{
    room: string;
    first: number;
    height: number;
    top: number;
  } | null>(null);
  const current = useRef({ room, latest, readTo });
  current.current = { room, latest, readTo };
  const atBottom = () => {
    const el = viewport.current;
    return !el || el.scrollHeight - el.scrollTop - el.clientHeight < 12;
  };
  const onScroll = () => {
    const value = atBottom();
    follow.current = value;
    setBottom(value);
  };
  const preserve = () => {
    const el = viewport.current;
    if (!el) return;
    follow.current = false;
    preserved.current = {
      room,
      first: messages[0]?.id || 0,
      height: el.scrollHeight,
      top: el.scrollTop,
    };
  };
  useLayoutEffect(() => {
    const saved = preserved.current,
      el = viewport.current;
    if (!saved || !el) return;
    if (saved.room !== room) {
      preserved.current = null;
      return;
    }
    if ((messages[0]?.id || 0) >= saved.first) return;
    el.scrollTop = saved.top + el.scrollHeight - saved.height;
    preserved.current = null;
    onScroll();
  }, [room, messages[0]?.id]);
  useEffect(() => {
    if (!enabled) return;
    const marker = end.current;
    if (!marker || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (entries) => {
        visible.current = entries[0]?.isIntersecting || false;
      },
      { threshold: 1 },
    );
    observer.observe(marker);
    return () => {
      visible.current = false;
      observer.disconnect();
    };
  }, [enabled, room]);
  useEffect(() => {
    if (!enabled) {
      previous.current = "";
      return;
    }
    const frame = requestAnimationFrame(() => {
      const el = viewport.current;
      if (!el) return;
      if (previous.current !== room) {
        previous.current = room;
        const first = messages.find((m) => (m.id || 0) > readTo);
        const node = first?.id
          ? el.querySelector<HTMLElement>(`[data-message-id="${first.id}"]`)
          : null;
        el.scrollTop = node ? node.offsetTop : el.scrollHeight;
      } else if (follow.current && !document.hidden && document.hasFocus()) {
        el.scrollTop = el.scrollHeight;
      }
      onScroll();
    });
    return () => cancelAnimationFrame(frame);
  }, [enabled, room, latest, messages.length]);
  useEffect(() => {
    if (!enabled || !acknowledge) return;
    const timer = setInterval(() => {
      const c = current.current;
      if (
        !c.room ||
        !c.latest ||
        c.latest <= c.readTo ||
        document.hidden ||
        !document.hasFocus() ||
        !visible.current ||
        !atBottom()
      )
        return;
      const key =
        (privateChat ? "private:" : "room:") + c.room + ":" + c.latest;
      if (pending.current.key === key && Date.now() - pending.current.at < 3000)
        return;
      pending.current = { key, at: Date.now() };
      void api(token, privateChat ? "messages/read" : "rooms/read", {
        [privateChat ? "username" : "room"]: c.room,
        through_id: c.latest,
      }).catch(() => {
        pending.current = { key: "", at: 0 };
      });
    }, 750);
    return () => clearInterval(timer);
  }, [enabled, token, acknowledge, privateChat]);
  const jump = () => {
    if (viewport.current) {
      viewport.current.scrollTop = viewport.current.scrollHeight;
      end.current?.scrollIntoView({ block: "nearest" });
      onScroll();
    }
  };
  return { viewport, end, onScroll, bottom, jump, preserve };
}
