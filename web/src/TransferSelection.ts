import { useEffect, useRef, useState } from "react";
import type { Transfer } from "./types";
export const transferKey = (t: Pick<Transfer, "username" | "path">) =>
  JSON.stringify([t.username, t.path]);
export const canCancelTransfer = (t: Transfer, uploads: boolean) =>
  (uploads
    ? ["queued", "uploading", "checking_shares"]
    : [
        "waiting",
        "waiting_for_space",
        "queued",
        "downloading",
        "paused",
        "paused_waiting",
        "retry_waiting",
        "interrupted",
      ]
  ).includes(t.status);

export function selectTransferRange(
  selected: Map<string, Transfer>,
  ordered: Transfer[],
  target: Transfer,
  anchor: string | null,
  toggle: boolean,
  range: boolean,
) {
  const key = transferKey(target);
  const a = ordered.findIndex((t) => transferKey(t) === anchor),
    b = ordered.findIndex((t) => transferKey(t) === key);
  if (range && a >= 0 && b >= 0) {
    const next = toggle ? new Map(selected) : new Map<string, Transfer>();
    for (const t of ordered.slice(Math.min(a, b), Math.max(a, b) + 1))
      next.set(transferKey(t), t);
    return { selected: next, anchor };
  }
  const next = toggle ? new Map(selected) : new Map<string, Transfer>();
  if (toggle && next.has(key)) next.delete(key);
  else next.set(key, target);
  return { selected: next, anchor: key };
}

export function useTransferSelection(
  all: Transfer[],
  ordered: Transfer[],
  criterion: string,
  uploads: boolean,
) {
  const [selected, setSelected] = useState<Map<string, Transfer>>(new Map());
  const anchor = useRef<{ criterion: string; key: string | null }>({
    criterion,
    key: null,
  });
  useEffect(() => {
    setSelected((old) => {
      let next = old;
      for (const t of all) {
        const key = transferKey(t);
        if (!old.has(key)) continue;
        if (!canCancelTransfer(t, uploads) || old.get(key) !== t) {
          if (next === old) next = new Map(old);
          if (canCancelTransfer(t, uploads)) next.set(key, t);
          else next.delete(key);
        }
      }
      return next;
    });
  }, [all, uploads]);
  function select(
    t: Transfer,
    modifiers: { ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean },
    checkbox = false,
  ) {
    if (!canCancelTransfer(t, uploads)) return;
    const prior =
      anchor.current.criterion === criterion ? anchor.current.key : null;
    const toggle = !!(
      modifiers.ctrlKey ||
      modifiers.metaKey ||
      (checkbox && !modifiers.shiftKey)
    );
    // Event handlers update the anchor immediately; functional state updates
    // retain selections if several clicks arrive before React renders again.
    const nextAnchor =
      modifiers.shiftKey && ordered.some((r) => transferKey(r) === prior)
        ? prior
        : transferKey(t);
    anchor.current = { criterion, key: nextAnchor };
    setSelected(
      (old) =>
        selectTransferRange(
          old,
          ordered,
          t,
          prior,
          toggle,
          !!modifiers.shiftKey,
        ).selected,
    );
  }
  function add(rows: Transfer[]) {
    setSelected((old) => {
      const next = new Map(old);
      for (const t of rows)
        if (canCancelTransfer(t, uploads)) next.set(transferKey(t), t);
      return next;
    });
  }
  function clear(user?: string) {
    anchor.current = { criterion, key: null };
    setSelected((old) =>
      user
        ? new Map([...old].filter(([, t]) => t.username !== user))
        : new Map(),
    );
  }
  return { selected, setSelected, select, add, clear };
}

// Stay within both the command's path-count bound and the ordinary 8-KiB
// request envelope, including UTF-8 and JSON escaping for long filenames.
export function cancelTransferBatches(
  uploads: boolean,
  username: string,
  rows: Transfer[],
) {
  const type = uploads ? "cancel_uploads" : "cancel_downloads";
  const batches: { type: string; username: string; paths: string[] }[] = [];
  let batch = { type, username, paths: [] as string[] };
  const encoder = new TextEncoder();
  for (const t of rows) {
    if (
      encoder.encode(JSON.stringify({ type, username, paths: [t.path] }))
        .length > 7500
    )
      throw new Error(
        "This filename exceeds the batch request limit. You can cancel all transfers for this user instead.",
      );
    const candidate = { ...batch, paths: [...batch.paths, t.path] };
    if (
      batch.paths.length &&
      (candidate.paths.length > 256 ||
        encoder.encode(JSON.stringify(candidate)).length > 7500)
    ) {
      batches.push(batch);
      batch = { type, username, paths: [t.path] };
    } else batch = candidate;
  }
  if (batch.paths.length) batches.push(batch);
  return batches;
}
