import { useEffect, useRef, useState } from "react";
import { bytes } from "./api";

const windowMs = 5000;
type Sample = { at: number; speed: number };

// Keep changes and the value at the window's left edge, rather than weighting
// frequent API responses more heavily. This state is local to the small readout.
export default function TransferSpeed({ speed }: { speed?: number }) {
  const samples = useRef<Sample[]>([]);
  const [average, setAverage] = useState(0);
  function measure(now: number) {
    const history = samples.current;
    if (!history.length) return 0;
    const start = Math.max(history[0].at, now - windowMs);
    while (history.length > 1 && history[1].at <= start) history.shift();
    if (now <= start) return history.at(-1)!.speed;
    let volume = 0;
    for (let i = 0; i < history.length; i++) {
      const end = history[i + 1]?.at ?? now;
      volume += history[i].speed * (end - Math.max(start, history[i].at));
    }
    return volume / (now - start);
  }
  useEffect(() => {
    // A paged feed can be loading its first complete summary or changing filters.
    // Missing data is not a zero-speed measurement.
    if (speed === undefined) return;
    const value = Number.isFinite(speed) && speed > 0 ? speed : 0;
    const now = performance.now();
    if (samples.current.at(-1)?.speed !== value)
      samples.current.push({ at: now, speed: value });
    setAverage(measure(now));
  }, [speed]);
  useEffect(() => {
    const timer = setInterval(
      () => setAverage(measure(performance.now())),
      1000,
    );
    return () => clearInterval(timer);
  }, []);
  return (
    <span title="Average transfer speed over the last 5 seconds">
      {bytes(average)}/s
    </span>
  );
}
