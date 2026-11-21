import { useEffect, useRef } from "react";
import { api } from "./api";
/** Reports activity age, never input content. Wake-up bypasses the normal ten-second throttle. */
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
      activity = performance.now(),
      activityVersion = 0,
      failures = 0,
      retryAfter = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const delay = () =>
      Math.max(
        0,
        automatic.current && !wakeRequested.current
          ? retryAfter - performance.now()
          : 10000 - (performance.now() - lastSent),
      );
    const schedule = (wait: number) => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void flush(), wait);
    };
    const flush = async () => {
      if (!active || inFlight) return;
      if (timer) clearTimeout(timer);
      timer = undefined;
      const sentVersion = activityVersion;
      const waking = automatic.current && !wakeRequested.current;
      lastSent = performance.now();
      if (waking) wakeRequested.current = true;
      inFlight = true;
      let failed = false;
      try {
        await api(token, "commands", {
          type: "ui_activity",
          idle_seconds: Math.min(
            86400,
            Math.max(0, Math.floor((performance.now() - activity) / 1000)),
          ),
        });
        failures = 0;
        retryAfter = 0;
      } catch {
        // A rejected/throttled wake-up must not remain marked as acknowledged.
        // Retry at most twice without further input; keep the original activity age.
        failed = true;
        failures++;
        retryAfter = performance.now() + 2000;
        if (waking) wakeRequested.current = false;
      } finally {
        inFlight = false;
        if (active && activityVersion !== sentVersion) schedule(delay());
        else if (active && failed && automatic.current && failures < 3)
          schedule(2000);
      }
    };
    const record = () => {
      activity = performance.now();
      activityVersion++;
      if (inFlight) return; // finally schedules any activity received during a request.
      const wait = delay();
      if (wait === 0) void flush();
      else schedule(wait);
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
      document.addEventListener(event, record, {
        passive: true,
        capture: true,
      });
    window.addEventListener("focus", visible);
    document.addEventListener("visibilitychange", visible);
    visible();
    return () => {
      active = false;
      if (timer) clearTimeout(timer);
      for (const event of events)
        document.removeEventListener(event, record, true);
      window.removeEventListener("focus", visible);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [token, enabled]);
}
