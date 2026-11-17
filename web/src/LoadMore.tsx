import { useEffect, useRef } from "react";
export default function LoadMore({
  load,
  loading = false,
  children,
  trigger,
}: {
  load: () => void;
  loading?: boolean;
  children: string;
  trigger?: unknown;
}) {
  const ref = useRef<HTMLDivElement>(null),
    callback = useRef(load);
  callback.current = load;
  useEffect(() => {
    if (loading || !ref.current || typeof IntersectionObserver === "undefined")
      return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          callback.current();
        }
      },
      { rootMargin: "600px 0px" },
    );
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, [loading, trigger]);
  return (
    <div ref={ref} className="infinite-sentinel">
      <button
        className="button small secondary"
        disabled={loading}
        onClick={load}
      >
        {loading ? "Loading results…" : children}
      </button>
    </div>
  );
}
