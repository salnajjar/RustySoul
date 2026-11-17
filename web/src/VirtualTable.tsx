import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { FileResult } from "./types";
export function VirtualBlock({
  estimate,
  children,
}: {
  estimate: number;
  children: () => ReactNode;
}) {
  const element = useRef<HTMLDivElement>(null),
    height = useRef(estimate);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => setVisible(entries.some((e) => e.isIntersecting)),
      { rootMargin: "1000px 0px" },
    );
    if (element.current) observer.observe(element.current);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (!visible || !element.current) return;
    height.current = Math.max(
      1,
      element.current.getBoundingClientRect().height,
    );
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      if (entries[0]?.contentRect.height)
        height.current = entries[0].contentRect.height;
    });
    observer.observe(element.current);
    return () => observer.disconnect();
  }, [visible]);
  return (
    <div
      ref={element}
      className="virtual-result-block"
      style={visible ? undefined : { height: height.current }}
    >
      {visible ? children() : null}
    </div>
  );
}
export default function VirtualTable({
  files,
  header,
  render,
}: {
  files: FileResult[];
  header: ReactNode;
  render: (file: FileResult) => ReactNode;
}) {
  return (
    <div className="table-scroll">
      <div className="virtual-result-tables">
        {Array.from({ length: Math.ceil(files.length / 100) }, (_, index) => (
          <VirtualBlock
            key={index}
            estimate={Math.min(100, files.length - index * 100) * 56 + 44}
          >
            {() => (
              <table>
                {header}
                <tbody>
                  {files.slice(index * 100, (index + 1) * 100).map(render)}
                </tbody>
              </table>
            )}
          </VirtualBlock>
        ))}
      </div>
    </div>
  );
}
