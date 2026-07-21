import { useState } from "react";

// Multi-select state shared by every view (icon/list/column) -- tracks
// which entry names are selected plus the last-clicked anchor used for
// shift-click range selection.
export function useSelection() {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [lastClicked, setLastClicked] = useState<string | null>(null);

  function selectOnly(name: string) {
    setSelected(new Set([name]));
    setLastClicked(name);
  }
  function toggle(name: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(name) ? next.delete(name) : next.add(name);
      return next;
    });
    setLastClicked(name);
  }
  // `orderedNames` is the caller's current sort order (e.g. sorted
  // entries mapped to names) -- range selection needs it to know what
  // "everything between the anchor and this click" means.
  function selectRange(to: string, orderedNames: string[]) {
    if (!lastClicked) return selectOnly(to);
    const a = orderedNames.indexOf(lastClicked);
    const b = orderedNames.indexOf(to);
    if (a === -1 || b === -1) return selectOnly(to);
    const [lo, hi] = a < b ? [a, b] : [b, a];
    setSelected(new Set(orderedNames.slice(lo, hi + 1)));
  }

  return { selected, setSelected, lastClicked, setLastClicked, selectOnly, toggle, selectRange };
}
