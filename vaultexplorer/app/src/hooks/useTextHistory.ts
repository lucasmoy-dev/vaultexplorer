import { useCallback, useEffect, useRef, useState } from "react";

// Undo/redo for the text editors. Neither editing surface can rely on the
// browser's own history: the textarea is a controlled React input whose
// value is also rewritten programmatically (list continuation, find,
// toolbar buttons), and the markdown preview is a contentEditable whose
// innerHTML gets replaced wholesale on re-render -- both of which
// silently drop native undo entries, so Ctrl+Z did nothing (or worse,
// undid one keystroke out of a rewrite). This keeps its own stack of
// whole-document snapshots instead, which is cheap for the size of file
// these panes open and survives every kind of edit the same way.
//
// Snapshots are grouped into bursts: the first edit after a quiet moment
// records the pre-edit text, and everything typed within COALESCE_MS of
// it folds into that one step -- so undo moves in visible chunks rather
// than one character at a time.
const COALESCE_MS = 600;
const MAX_DEPTH = 300;

export function useTextHistory(
  content: string | null,
  setContent: (value: string) => void,
  // Changing this (the open file's path) throws the history away --
  // undoing into a *different* file's text would write it into this one.
  resetKey: string
): {
  // Call right *before* applying an edit, with the pre-edit text still in
  // `content`.
  record: () => void;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  // Bumped only by undo/redo, so a surface rendered *from* `content` (the
  // markdown preview) can tell "restored" from "the user is typing" and
  // re-render only for the former.
  restoreRevision: number;
} {
  const undoStack = useRef<string[]>([]);
  const redoStack = useRef<string[]>([]);
  const burstTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [depth, setDepth] = useState({ undo: 0, redo: 0 });
  const [restoreRevision, setRestoreRevision] = useState(0);

  const contentRef = useRef(content);
  contentRef.current = content;

  useEffect(() => {
    undoStack.current = [];
    redoStack.current = [];
    if (burstTimer.current) clearTimeout(burstTimer.current);
    burstTimer.current = null;
    setDepth({ undo: 0, redo: 0 });
  }, [resetKey]);

  useEffect(
    () => () => {
      if (burstTimer.current) clearTimeout(burstTimer.current);
    },
    []
  );

  const record = useCallback(() => {
    const current = contentRef.current;
    if (current === null) return;
    // Any fresh edit abandons whatever was undone: the redo branch it
    // would have led back to no longer exists.
    redoStack.current = [];
    if (burstTimer.current) {
      setDepth({ undo: undoStack.current.length, redo: 0 });
      return;
    }
    if (undoStack.current[undoStack.current.length - 1] !== current) {
      undoStack.current.push(current);
      if (undoStack.current.length > MAX_DEPTH) undoStack.current.shift();
    }
    burstTimer.current = setTimeout(() => {
      burstTimer.current = null;
    }, COALESCE_MS);
    setDepth({ undo: undoStack.current.length, redo: 0 });
  }, []);

  const undo = useCallback(() => {
    const current = contentRef.current;
    if (current === null) return;
    const previous = undoStack.current.pop();
    if (previous === undefined) return;
    // End the current burst, or the next keystroke would fold into a step
    // that no longer describes what's on screen.
    if (burstTimer.current) {
      clearTimeout(burstTimer.current);
      burstTimer.current = null;
    }
    redoStack.current.push(current);
    setContent(previous);
    setDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
    setRestoreRevision((v) => v + 1);
  }, [setContent]);

  const redo = useCallback(() => {
    const current = contentRef.current;
    if (current === null) return;
    const next = redoStack.current.pop();
    if (next === undefined) return;
    if (burstTimer.current) {
      clearTimeout(burstTimer.current);
      burstTimer.current = null;
    }
    undoStack.current.push(current);
    setContent(next);
    setDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
    setRestoreRevision((v) => v + 1);
  }, [setContent]);

  return {
    record,
    undo,
    redo,
    canUndo: depth.undo > 0,
    canRedo: depth.redo > 0,
    restoreRevision,
  };
}

// Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z and Ctrl+Y, for a keydown handler on the
// editing surface itself (not the window -- a rename field or the search
// box must keep their own native undo). Returns whether it handled the
// event.
export function handleHistoryKeyDown(
  e: React.KeyboardEvent,
  history: { undo: () => void; redo: () => void }
): boolean {
  if (!(e.ctrlKey || e.metaKey)) return false;
  const key = e.key.toLowerCase();
  if (key === "z") {
    e.preventDefault();
    // The native contentEditable/textarea undo would otherwise run *as
    // well*, on a stack that no longer matches the text on screen.
    e.stopPropagation();
    e.shiftKey ? history.redo() : history.undo();
    return true;
  }
  if (key === "y") {
    e.preventDefault();
    e.stopPropagation();
    history.redo();
    return true;
  }
  return false;
}
