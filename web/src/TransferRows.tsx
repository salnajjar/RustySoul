import type { ReactNode } from "react";
import type { Transfer } from "./types";
import { VirtualBlock } from "./VirtualTable";
export default function TransferRows({
  rows,
  render,
}: {
  rows: Transfer[];
  render: (t: Transfer) => ReactNode;
}) {
  // Small lists render immediately; large user groups keep all their data but
  // mount only nearby blocks of rows, including when re-expanded.
  if (rows.length <= 100) return <>{rows.map(render)}</>;
  return (
    <>
      {Array.from({ length: Math.ceil(rows.length / 50) }, (_, index) => (
        <VirtualBlock
          key={index}
          estimate={Math.min(50, rows.length - index * 50) * 100}
        >
          {() => rows.slice(index * 50, (index + 1) * 50).map(render)}
        </VirtualBlock>
      ))}
    </>
  );
}
