import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ChevronLeft, ChevronRight, X } from "lucide-react";
import type { Search } from "./types";

export default function SearchTabs({
  searches,
  activeId,
  select,
  close,
}: {
  searches: Search[];
  activeId?: string;
  select: (id: string) => void;
  close: (id: string) => void;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ older: false, newer: false });
  const updateEdges = () => {
    const node = viewport.current;
    if (node)
      setEdges({
        older: node.scrollLeft > 1,
        newer: node.scrollLeft + node.clientWidth < node.scrollWidth - 1,
      });
  };
  useEffect(() => {
    const node = viewport.current!;
    const observer = new ResizeObserver(updateEdges);
    observer.observe(node);
    updateEdges();
    return () => observer.disconnect();
  }, []);
  useEffect(updateEdges, [searches]);
  // A live result count must not pull someone browsing older tabs back right.
  useEffect(() => {
    const node = viewport.current!;
    const tab = node.querySelector<HTMLElement>(
      '[aria-selected="true"]',
    )?.parentElement;
    if (tab) {
      const outer = node.getBoundingClientRect(),
        inner = tab.getBoundingClientRect();
      if (inner.left < outer.left) node.scrollLeft += inner.left - outer.left;
      else if (inner.right > outer.right)
        node.scrollLeft += inner.right - outer.right;
    }
    updateEdges();
  }, [activeId]);
  function navigate(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const target =
      event.key === "ArrowLeft"
        ? Math.max(0, index - 1)
        : event.key === "ArrowRight"
          ? Math.min(searches.length - 1, index + 1)
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? searches.length - 1
              : undefined;
    if (target === undefined) return;
    event.preventDefault();
    select(searches[target].id);
    viewport.current
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      [target]?.focus();
  }
  const scroll = (direction: number) => {
    const node = viewport.current!;
    node.scrollBy({ left: direction * node.clientWidth * 0.8 });
  };
  return (
    <div className="search-tabs">
      <button
        className="search-tab-scroll"
        aria-label="Scroll to older searches"
        disabled={!edges.older}
        onClick={() => scroll(-1)}
      >
        <ChevronLeft size={18} />
      </button>
      <div
        ref={viewport}
        className="tabs"
        role="tablist"
        aria-label="Open searches"
        onScroll={updateEdges}
      >
        {searches.map((s, index) => (
          <div
            className={`tab ${activeId === s.id ? "active" : ""}`}
            key={s.id}
          >
            <button
              role="tab"
              aria-selected={activeId === s.id}
              tabIndex={activeId === s.id ? 0 : -1}
              title={s.query}
              onClick={() => select(s.id)}
              onKeyDown={(event) => navigate(event, index)}
            >
              {s.collecting && <span className="dot online pulse" />}
              <span className="tab-query">{s.query}</span>
              <span className="tab-count">{s.total ?? s.results.length}</span>
            </button>
            <button
              aria-label={`Close search ${s.query}`}
              onClick={() => close(s.id)}
            >
              <X size={13} />
            </button>
          </div>
        ))}
      </div>
      <button
        className="search-tab-scroll"
        aria-label="Scroll to newer searches"
        disabled={!edges.newer}
        onClick={() => scroll(1)}
      >
        <ChevronRight size={18} />
      </button>
    </div>
  );
}
