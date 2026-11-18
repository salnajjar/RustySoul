import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

const preference = "rustysoul.shareFolders.width";
const minimum = 180;
const maximum = 720;
const initial = 240;
const clamp = (width: number, max = maximum) =>
  Math.max(minimum, Math.min(max, width));
function readWidth(value: string | null) {
  const width = value === null ? initial : Number(value);
  return Number.isFinite(width) && width >= minimum ? clamp(width) : initial;
}
function savedWidth() {
  try {
    return readWidth(localStorage.getItem(preference));
  } catch {
    return initial;
  }
}
function persist(width: number) {
  try {
    localStorage.setItem(preference, String(width));
  } catch {
    /* Resizing still works when browser storage is unavailable. */
  }
}

export default function ShareLayout({
  folders,
  children,
}: {
  folders: ReactNode;
  children: ReactNode;
}) {
  const layout = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    pointer: number;
    x: number;
    width: number;
    latest: number;
  } | null>(null);
  const [width, setWidth] = useState(savedWidth);
  const [limit, setLimit] = useState(maximum);
  const effectiveWidth = clamp(width, limit);
  useEffect(() => {
    const element = layout.current;
    if (!element) return;
    const measure = () =>
      setLimit(clamp(element.getBoundingClientRect().width - 356));
    measure();
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(measure);
    observer?.observe(element);
    window.addEventListener("resize", measure);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);
  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key === preference && !drag.current)
        setWidth(readWidth(event.newValue));
    };
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, []);
  function change(next: number) {
    next = clamp(next, limit);
    setWidth(next);
    persist(next);
  }
  function finish() {
    if (!drag.current) return;
    persist(drag.current.latest);
    drag.current = null;
  }
  return (
    <div
      ref={layout}
      className="share-layout"
      style={{ "--share-folder-width": `${effectiveWidth}px` } as CSSProperties}
    >
      {folders}
      <div
        role="separator"
        aria-label="Resize folders pane"
        aria-orientation="vertical"
        aria-controls="share-folders-pane"
        aria-valuemin={minimum}
        aria-valuemax={limit}
        aria-valuenow={effectiveWidth}
        aria-valuetext={`${effectiveWidth} pixels`}
        tabIndex={0}
        className="share-resizer"
        title="Drag to resize folders. Use arrow keys to adjust, or double-click to reset."
        onPointerDown={(event) => {
          if (event.button !== 0 || drag.current) return;
          event.preventDefault();
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = {
            pointer: event.pointerId,
            x: event.clientX,
            width: effectiveWidth,
            latest: effectiveWidth,
          };
        }}
        onPointerMove={(event) => {
          const current = drag.current;
          if (!current || current.pointer !== event.pointerId) return;
          current.latest = clamp(
            current.width + event.clientX - current.x,
            limit,
          );
          setWidth(current.latest);
        }}
        onPointerUp={(event) => {
          if (drag.current?.pointer !== event.pointerId) return;
          finish();
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={(event) => {
          if (drag.current?.pointer === event.pointerId) finish();
        }}
        onLostPointerCapture={finish}
        onDoubleClick={() => change(initial)}
        onKeyDown={(event) => {
          const step = event.shiftKey ? 64 : 16;
          const next =
            event.key === "ArrowLeft"
              ? effectiveWidth - step
              : event.key === "ArrowRight"
                ? effectiveWidth + step
                : event.key === "Home"
                  ? minimum
                  : event.key === "End"
                    ? limit
                    : null;
          if (next === null) return;
          event.preventDefault();
          change(next);
        }}
      />
      {children}
    </div>
  );
}
