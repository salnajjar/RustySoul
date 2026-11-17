import { useEffect, useRef } from "react";
import { api } from "./api";
/** Reports activity age, never input content. One leading/trailing request per ten seconds. */
export function useActivity(
  token: string,
  enabled: boolean,
  automaticAway: boolean,
) {
  const wakeRequested = useRef(false);
  if (!automaticAway) wakeRequested.current = false;
  const automatic = useRef(automaticAway);
  automatic.current = automaticAway;
  useEffect(() => {
    if (!enabled) return;
    let active = true,
      inFlight = false,
      lastSent = -Infinity,
      activity = performance.now();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const flush = async () => {
      if (!active || inFlight) return;
      if (timer) clearTimeout(timer);
      timer = undefined;
      const sentActivity = activity;
      lastSent = performance.now();
      if (automatic.current) wakeRequested.current = true;
      inFlight = true;
      try {
        await api(token, "commands", {
          type: "ui_activity",
          idle_seconds: Math.min(
            86400,
            Math.max(0, Math.floor((performance.now() - activity) / 1000)),
          ),
        });
      } catch {
        /* Connectivity failures are already handled by the live connection. */
      } finally {
        inFlight = false;
        if (active && activity > sentActivity)
          timer = setTimeout(
            () => void flush(),
            Math.max(0, 10000 - (performance.now() - lastSent)),
          );
      }
    };
    const record = () => {
      activity = performance.now();
      if (
        (automatic.current && !wakeRequested.current) ||
        performance.now() - lastSent >= 10000
      )
        void flush();
      else {
        if (timer) clearTimeout(timer);
        timer = setTimeout(
          () => void flush(),
          10000 - (performance.now() - lastSent),
        );
      }
    };
    const visible = () => {
      if (document.visibilityState === "visible") record();
    };
    const events = [
      "pointerdown",
      "pointermove",
      "keydown",
      "wheel",
      "touchstart",
    ];
    for (const event of events)
      document.addEventListener(event, record, { passive: true });
    window.addEventListener("focus", visible);
    document.addEventListener("visibilitychange", visible);
    visible();
    return () => {
      active = false;
      if (timer) clearTimeout(timer);
      for (const event of events) document.removeEventListener(event, record);
      window.removeEventListener("focus", visible);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [token, enabled]);
}
