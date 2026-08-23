// Raw-text list behaviour shared by both text editors (the plain one used
// by Notes/mobile and the markdown pane's Source mode): Enter continues
// the list, Enter on an empty item ends it, Tab/Shift+Tab nest, and
// Ctrl/Cmd+Enter ticks a checkbox off.
//
// Numbered lists are also renumbered after every structural change --
// without that, continuing "1." only ever produced the *next* number for
// the line right below it, and any item inserted in the middle (or an
// item deleted) left the rest of the list stuck on whatever number it was
// first typed with, which reads as "the editor always writes 1.".

// Group 1: indent. 2: bullet char. 3: checkbox state, if any. 4: the
// digits of a numbered item. Both "1." and "1)" count as numbered.
export const LIST_RE = /^(\s*)(?:([-*+])\s+(?:\[( |x|X)\]\s+)?|(\d+)[.)]\s+)/;

export type TextEdit = { text: string; caret: number };

/// Rewrite every numbered item so each run counts 1, 2, 3… at its own
/// indent level. `caret` is carried through the length changes so the
/// cursor doesn't drift when a number gains or loses a digit.
export function renumberOrderedLists(text: string, caret: number): TextEdit {
  const lines = text.split("\n");
  // Highest number used so far at each indent width. Cleared whenever the
  // run ends, so two separate lists both start at 1.
  const counters = new Map<number, number>();
  let inFence = false;
  let origOffset = 0;
  let newCaret = caret;
  const out = lines.map((line) => {
    const lineStart = origOffset;
    origOffset += line.length + 1; // +1 for the "\n" split removed
    if (/^\s*```/.test(line)) {
      inFence = !inFence;
      counters.clear();
      return line;
    }
    // A fenced block can contain anything, including text that looks
    // exactly like a list -- never touch it.
    if (inFence) return line;
    const m = LIST_RE.exec(line);
    if (!m) {
      // An indented continuation line (a soft-wrapped item, a paragraph
      // under an item) keeps the run alive; a blank line or a plain
      // left-margin line ends it.
      if (line.trim() !== "" && /^\s+\S/.test(line)) return line;
      counters.clear();
      return line;
    }
    const indent = m[1].length;
    // Going back out a level abandons the deeper levels' counters, so a
    // new sublist further down starts at 1 again.
    for (const key of [...counters.keys()]) if (key > indent) counters.delete(key);
    if (m[4] === undefined) {
      // A bullet at this level -- numbering here starts over if numbers
      // come back later.
      counters.delete(indent);
      return line;
    }
    const next = (counters.get(indent) ?? 0) + 1;
    counters.set(indent, next);
    const digitsStart = indent;
    const digitsEnd = indent + m[4].length;
    if (String(next) === m[4]) return line;
    const rewritten = line.slice(0, digitsStart) + next + line.slice(digitsEnd);
    const delta = rewritten.length - line.length;
    if (caret > lineStart + digitsEnd) newCaret += delta;
    return rewritten;
  });
  return { text: out.join("\n"), caret: newCaret };
}

/// Handle a keystroke in a plain-text markdown editor. Returns the new
/// text + caret when it took over the key (and has already called
/// `preventDefault`), or null to let the textarea handle it itself.
export function listKeyDown(
  e: React.KeyboardEvent<HTMLTextAreaElement>,
  text: string
): TextEdit | null {
  const el = e.currentTarget;
  const start = el.selectionStart;
  const end = el.selectionEnd;
  const lineStart = text.lastIndexOf("\n", start - 1) + 1;
  const lineEnd = text.indexOf("\n", start) === -1 ? text.length : text.indexOf("\n", start);
  const line = text.slice(lineStart, lineEnd);
  const m = LIST_RE.exec(line);
  if (!m) return null;

  const splice = (from: number, to: number, insert: string, caret: number): TextEdit =>
    renumberOrderedLists(text.slice(0, from) + insert + text.slice(to), caret);

  if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey && start === end) {
    e.preventDefault();
    const marker = m[0];
    // Enter on an item with no text ends the list instead of adding
    // another empty one -- the universal "I'm done" gesture.
    if (line.trim() === marker.trim()) {
      return splice(lineStart, lineEnd, m[1], lineStart + m[1].length);
    }
    // A continued checkbox always starts unchecked, whatever the line
    // above was: copying "[x]" onto a brand-new item would mark work
    // done that nobody has done. The number here is only a first guess --
    // renumberOrderedLists below is what actually makes the run count up,
    // including every item after this one.
    const next = m[4]
      ? `${m[1]}${Number(m[4]) + 1}. `
      : `${m[1]}${m[2]}${m[3] !== undefined ? " [ ]" : ""} `;
    return splice(start, end, "\n" + next, start + 1 + next.length);
  }

  if (e.key === "Tab") {
    e.preventDefault();
    if (e.shiftKey) {
      const dedented = m[1].slice(0, Math.max(0, m[1].length - 2));
      return splice(
        lineStart,
        lineStart + m[1].length,
        dedented,
        Math.max(lineStart, start - (m[1].length - dedented.length))
      );
    }
    return splice(lineStart, lineStart, "  ", start + 2);
  }

  // Ctrl/Cmd+Enter ticks the current item off without reaching for the
  // mouse or retyping the marker.
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && m[3] !== undefined) {
    e.preventDefault();
    const toggled = line.replace(/\[( |x|X)\]/, m[3] === " " ? "[x]" : "[ ]");
    return splice(lineStart, lineEnd, toggled, start);
  }

  return null;
}

/// Prefix every selected line with an incrementing number ("1. ", "2. ",
/// …) -- the numbered-list toolbar button. Plain prefixing can't do this:
/// every line would come out as "1.".
export function numberLines(text: string, from: number, to: number): TextEdit {
  const lineStart = text.lastIndexOf("\n", from - 1) + 1;
  const block = text.slice(lineStart, to);
  let n = 0;
  const numbered = block
    .split("\n")
    .map((l) => `${++n}. ${l}`)
    .join("\n");
  const next = text.slice(0, lineStart) + numbered + text.slice(to);
  return { text: next, caret: lineStart + numbered.length };
}
