import { useEffect, useRef, useState } from "react";
import { Entry, api, joinPath, parentPath } from "../api";
import { kindOf, FileIcon } from "../icons";
import { renderMarkdownToHtml, serializePreviewToMarkdown } from "../markdown";
import { useAutoSaveText } from "../hooks/useAutoSaveText";
import { EditableFileName, PreviewColumn } from "./PreviewColumn";

export function TextEditorPane({
  entry,
  fullPath,
  inVault,
  onRename,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onRename?: (newName: string) => void;
}) {
  const { content, error, saving, setContent } = useAutoSaveText(fullPath, inVault);
  return (
    <div className="preview-pane text-editor-pane">
      <div className="preview-name-row">
        <EditableFileName name={entry.name} onRename={onRename} />
        {saving && <span className="saving-hint"> — saving…</span>}
      </div>
      {error && <p className="error">{error}</p>}
      {content !== null && (
        <textarea
          className="text-editor-area"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          spellCheck={false}
        />
      )}
    </div>
  );
}

// Lets the user pick an image via VaultExplorer's own browsing, not the
// OS-native file dialog -- the native picker can't see into a vault at all
// (there's no real fs path to hand it), so "Insert image" used to be unable
// to reach anything actually stored in the vault. Defaults to browsing the
// vault when the note itself is in one, with a toggle to browse the real
// filesystem instead (e.g. to pull in a photo from outside the vault).
function ImagePickerModal({
  startDir,
  startInVault,
  canBrowseVault,
  onCancel,
  onPick,
}: {
  startDir: string;
  startInVault: boolean;
  canBrowseVault: boolean;
  onCancel: () => void;
  onPick: (entry: Entry, dir: string, source: boolean) => void;
}) {
  const [source, setSource] = useState(startInVault); // true = vault, false = fs
  const [dir, setDir] = useState(startDir);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    setSelected(null);
    setError("");
    const call = source ? api.listDir(dir) : api.fsList(dir, false);
    call
      .then((list) =>
        setEntries([...list].sort((a, b) => (a.is_dir === b.is_dir ? a.name.localeCompare(b.name) : a.is_dir ? -1 : 1)))
      )
      .catch((e) => setError(String(e)));
  }, [dir, source]);

  function switchSource(toVault: boolean) {
    if (toVault === source) return;
    setSource(toVault);
    if (toVault) {
      setDir(startInVault ? startDir : "");
    } else {
      api.homeDir().then(setDir).catch(() => setDir("/"));
    }
  }

  const atRoot = source ? dir === "" : dir === "/";
  const selectedEntry = entries.find((e) => e.name === selected) ?? null;
  const canInsert = !!selectedEntry && !selectedEntry.is_dir && kindOf(selectedEntry) === "image";

  function open(entry: Entry) {
    if (entry.is_dir) setDir(joinPath(dir, entry.name));
    else if (kindOf(entry) === "image") onPick(entry, dir, source);
  }

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card image-picker-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Insert Image</h3>
        {canBrowseVault && (
          <div className="segmented" style={{ marginBottom: 10 }}>
            <button className={`seg seg-text ${source ? "on" : ""}`} onClick={() => switchSource(true)}>
              Vault
            </button>
            <button className={`seg seg-text ${!source ? "on" : ""}`} onClick={() => switchSource(false)}>
              Computer
            </button>
          </div>
        )}
        <div className="picker-toolbar">
          <button className="btn-plain" disabled={atRoot} onClick={() => setDir(parentPath(dir) || (source ? "" : "/"))}>
            Up
          </button>
          <span className="picker-path-input" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {source ? dir || "/" : dir}
          </span>
        </div>
        {error && <p className="error">{error}</p>}
        <div className="picker-list">
          {entries.map((e) => (
            <div
              key={e.name}
              className={`picker-row ${selected === e.name ? "selected" : ""}`}
              style={!e.is_dir && kindOf(e) !== "image" ? { opacity: 0.4 } : undefined}
              onClick={() => setSelected(e.name)}
              onDoubleClick={() => open(e)}
            >
              <FileIcon entry={e} />
              <span>{e.name}</span>
            </div>
          ))}
        </div>
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" disabled={!canInsert} onClick={() => selectedEntry && onPick(selectedEntry, dir, source)}>
            Insert
          </button>
        </div>
      </div>
    </div>
  );
}

export function MarkdownEditorPane({
  entry,
  fullPath,
  inVault,
  onRename,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onRename?: (newName: string) => void;
}) {
  const { content, error, saving, setContent, externalRevision } = useAutoSaveText(fullPath, inVault);
  const [mode, setMode] = useState<"preview" | "source">("preview");
  const [pasteError, setPasteError] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const previewRef = useRef<HTMLDivElement>(null);

  // Push rendered HTML into the editable preview on load / file switch /
  // mode switch / an external change (see `externalRevision`) -- never on
  // every keystroke (that's what onPreviewInput below is for). Re-rendering
  // on every input would fight the user's own typing and reset the cursor
  // position each time.
  useEffect(() => {
    if (mode === "preview" && previewRef.current && content !== null) {
      previewRef.current.innerHTML = renderMarkdownToHtml(content);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fullPath, mode, content !== null, externalRevision]);

  function onPreviewInput() {
    if (!previewRef.current) return;
    setContent(serializePreviewToMarkdown(previewRef.current));
  }

  // Tracks a live resize (see the `.md-img-wrap` CSS `resize: both`) back
  // into the attribute htmlNodeToMarkdown reads, and re-serializes so the
  // new size actually gets saved -- a native CSS resize handle has no
  // "resize finished" event of its own to hook into instead. Observes the
  // wrapper span, not the <img> -- that's the element the resize handle
  // actually lives on.
  const imageResizeObserverRef = useRef<ResizeObserver | null>(null);
  useEffect(() => {
    imageResizeObserverRef.current = new ResizeObserver((observed) => {
      for (const o of observed) {
        (o.target as HTMLElement).setAttribute("data-md-width", String(Math.round(o.contentRect.width)));
      }
      onPreviewInput();
    });
    return () => imageResizeObserverRef.current?.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // `renderMarkdownToHtml` only ever emits `data-md-src` (a path relative
  // to this note), never a directly-usable `src` -- resolving that into
  // actual displayable bytes needs a Tauri call (there's no plain URL a
  // webview can just point at: a vault path isn't a real file at all, and
  // even a real-fs one isn't guaranteed readable via a bare `file://`),
  // so it happens here, async, after the synchronous HTML gets set.
  useEffect(() => {
    const preview = previewRef.current;
    if (mode !== "preview" || !preview) return;
    const dir = parentPath(fullPath);
    preview.querySelectorAll<HTMLImageElement>("img[data-md-src]").forEach((img) => {
      if (img.parentElement?.classList.contains("md-img-wrap")) {
        imageResizeObserverRef.current?.observe(img.parentElement);
      }
      if (img.src) return;
      const rel = img.getAttribute("data-md-src");
      if (!rel) return;
      // Base64 data: URIs are self-contained -- use directly, no file read.
      if (rel.startsWith("data:")) {
        img.src = rel;
        return;
      }
      const call = inVault ? api.vaultThumbnail(joinPath(dir, rel), 2000) : api.fsThumbnail(joinPath(dir, rel), 2000);
      call.then((uri) => {
        img.src = uri;
      }).catch(() => {
        /* broken/missing image reference -- leave it un-rendered */
      });
    });
  }, [content, mode, fullPath, inVault]);

  function insertImageAtCaret(relSrc: string, dataUrl: string) {
    const preview = previewRef.current;
    if (!preview) return;
    const wrap = document.createElement("span");
    wrap.className = "md-img-wrap";
    const img = document.createElement("img");
    img.className = "md-img";
    img.setAttribute("data-md-src", relSrc);
    img.src = dataUrl;
    wrap.appendChild(img);
    imageResizeObserverRef.current?.observe(wrap);
    const sel = window.getSelection();
    const range =
      sel && sel.rangeCount > 0 && preview.contains(sel.getRangeAt(0).commonAncestorContainer)
        ? sel.getRangeAt(0)
        : null;
    if (range) {
      range.collapse(false);
      range.insertNode(wrap);
    } else {
      preview.appendChild(wrap);
    }
    onPreviewInput();
  }

  // Embed images as base64 data: URIs directly in the Markdown (`![](data:
  // …)`), rather than writing a sibling file -- self-contained, and works
  // inside a vault too (no plaintext file leaks next to the encrypted note).
  function fileToDataUrl(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => resolve(r.result as string);
      r.onerror = () => reject(r.error);
      r.readAsDataURL(file);
    });
  }
  async function insertImageFile(file: File) {
    try {
      const dataUrl = await fileToDataUrl(file);
      insertImageAtCaret(dataUrl, dataUrl);
    } catch (err) {
      setPasteError(String(err));
    }
  }
  async function onPreviewPaste(e: React.ClipboardEvent) {
    const item = [...e.clipboardData.items].find((it) => it.type.startsWith("image/"));
    if (!item) return;
    const file = item.getAsFile();
    if (!file) return;
    e.preventDefault();
    await insertImageFile(file);
  }
  const [showImagePicker, setShowImagePicker] = useState(false);
  async function insertPickedImage(picked: Entry, dir: string, pickedInVault: boolean) {
    setShowImagePicker(false);
    try {
      const path = joinPath(dir, picked.name);
      const dataUrl = pickedInVault ? await api.vaultThumbnail(path, 2000) : await api.fsThumbnail(path, 2000);
      insertImageAtCaret(dataUrl, dataUrl);
    } catch (err) {
      setPasteError(String(err));
    }
  }

  // ---- Task-list helpers (checkbox items, nesting, keyboard, drag) ----

  function findTaskLi(node: Node | null): HTMLLIElement | null {
    let n: Node | null = node;
    while (n && n !== previewRef.current) {
      if (n.nodeType === Node.ELEMENT_NODE && (n as HTMLElement).matches?.("li.md-task")) {
        return n as HTMLLIElement;
      }
      n = n.parentNode;
    }
    return null;
  }
  function caretToEnd(el: HTMLElement) {
    const r = document.createRange();
    r.selectNodeContents(el);
    r.collapse(false);
    const s = window.getSelection();
    s?.removeAllRanges();
    s?.addRange(r);
  }
  function makeTaskLi(): HTMLLIElement {
    const li = document.createElement("li");
    li.className = "md-task";
    li.setAttribute("data-task", "1");
    li.setAttribute("data-checked", "false");
    li.setAttribute("draggable", "true");
    const chk = document.createElement("span");
    chk.className = "md-task-check";
    chk.contentEditable = "false";
    const txt = document.createElement("span");
    txt.className = "md-task-text";
    li.append(chk, txt);
    return li;
  }
  const taskText = (li: HTMLLIElement) => li.querySelector<HTMLElement>(":scope > .md-task-text");
  const isNestedLi = (li: HTMLLIElement) =>
    li.parentElement?.parentElement?.tagName === "LI";

  function indentTask(li: HTMLLIElement) {
    const prev = li.previousElementSibling as HTMLElement | null;
    if (!prev || prev.tagName !== "LI") return; // first item can't indent
    let sub = prev.querySelector<HTMLUListElement>(":scope > ul");
    if (!sub) {
      sub = document.createElement("ul");
      sub.className = "md-tasklist";
      prev.appendChild(sub);
    }
    sub.appendChild(li);
    const t = taskText(li);
    if (t) caretToEnd(t);
    onPreviewInput();
  }
  function outdentTask(li: HTMLLIElement) {
    const ul = li.parentElement as HTMLElement | null;
    const parentLi = ul?.parentElement as HTMLElement | null;
    if (!parentLi || parentLi.tagName !== "LI") return; // already top level
    parentLi.after(li);
    if (ul && ul.children.length === 0) ul.remove();
    const t = taskText(li);
    if (t) caretToEnd(t);
    onPreviewInput();
  }
  // True when the caret sits exactly at the start of a task's text (or the
  // text is empty) -- i.e. nothing between the start of `.md-task-text` and
  // the caret.
  function caretAtStartOfTaskText(li: HTMLLIElement): boolean {
    const t = taskText(li);
    if (!t) return true;
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return false;
    const r = sel.getRangeAt(0);
    if (!r.collapsed) return false;
    const probe = document.createRange();
    probe.selectNodeContents(t);
    try {
      probe.setEnd(r.startContainer, r.startOffset);
    } catch {
      return false; // caret isn't inside this task's text at all
    }
    return probe.toString().length === 0;
  }
  function caretAtEndOfTaskText(li: HTMLLIElement): boolean {
    const t = taskText(li);
    if (!t) return true;
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return false;
    const r = sel.getRangeAt(0);
    if (!r.collapsed) return false;
    const probe = document.createRange();
    probe.selectNodeContents(t);
    try {
      probe.setStart(r.startContainer, r.startOffset);
    } catch {
      return false;
    }
    return probe.toString().length === 0;
  }

  // Backspace at the start of a task merges its text (and any nested
  // sublist) into the end of the previous same-level task, then removes it
  // -- native contentEditable's own merge-two-blocks behavior would instead
  // drag this item's checkbox marker into the previous row too, since it has
  // no idea `.md-task-check` is a marker and not content. Returns whether a
  // merge happened (false when there's no previous sibling to merge into).
  function mergeTaskWithPrevious(li: HTMLLIElement): boolean {
    const prev = li.previousElementSibling as HTMLElement | null;
    if (!prev || !prev.matches("li.md-task")) return false;
    const prevText = taskText(prev as HTMLLIElement);
    const curText = taskText(li);
    const joinOffset = prevText?.textContent?.length ?? 0;
    if (prevText && curText) {
      while (curText.firstChild) prevText.appendChild(curText.firstChild);
      prevText.normalize();
    }
    const curSub = li.querySelector(":scope > ul.md-tasklist");
    if (curSub) {
      const prevSub = prev.querySelector(":scope > ul.md-tasklist");
      if (prevSub) {
        while (curSub.firstChild) prevSub.appendChild(curSub.firstChild);
      } else {
        prev.appendChild(curSub);
      }
    }
    li.remove();
    if (prevText) {
      const node = prevText.firstChild;
      const r = document.createRange();
      if (node) r.setStart(node, Math.min(joinOffset, node.textContent?.length ?? 0));
      else r.selectNodeContents(prevText);
      r.collapse(true);
      const s = window.getSelection();
      s?.removeAllRanges();
      s?.addRange(r);
    }
    onPreviewInput();
    return true;
  }

  // Delete at the end of a task pulls the *next* same-level task's text up
  // into this one and removes it -- the simple sibling case only (a next
  // task with its own children is left to native behavior rather than risk
  // mangling a subtree).
  function mergeNextTaskInto(li: HTMLLIElement): boolean {
    const next = li.nextElementSibling as HTMLElement | null;
    if (!next || !next.matches("li.md-task")) return false;
    if (next.querySelector(":scope > ul.md-tasklist")) return false;
    const curText = taskText(li);
    const nextText = taskText(next as HTMLLIElement);
    if (curText && nextText) {
      const node = curText.lastChild;
      const joinOffset = node?.textContent?.length ?? curText.textContent?.length ?? 0;
      while (nextText.firstChild) curText.appendChild(nextText.firstChild);
      curText.normalize();
      const r = document.createRange();
      const target = curText.firstChild;
      if (target) r.setStart(target, Math.min(joinOffset, target.textContent?.length ?? 0));
      else r.selectNodeContents(curText);
      r.collapse(true);
      const s = window.getSelection();
      s?.removeAllRanges();
      s?.addRange(r);
    }
    next.remove();
    onPreviewInput();
    return true;
  }

  function newTaskAfter(li: HTMLLIElement) {
    const t = taskText(li);
    if (t && (t.textContent ?? "").trim() === "") {
      // Enter on an empty item: outdent if nested, else break out of the list.
      if (isNestedLi(li)) {
        outdentTask(li);
        return;
      }
      const ul = li.parentElement as HTMLElement;
      const p = document.createElement("div");
      p.innerHTML = "<br>";
      ul.after(p);
      li.remove();
      if (ul.children.length === 0) ul.remove();
      caretToEnd(p);
      onPreviewInput();
      return;
    }
    const nli = makeTaskLi();
    li.after(nli);
    const nt = taskText(nli);
    if (nt) caretToEnd(nt);
    onPreviewInput();
  }

  function onPreviewKeyDown(e: React.KeyboardEvent) {
    const li = findTaskLi(window.getSelection()?.anchorNode ?? null);
    if (li) {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        newTaskAfter(li);
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        e.shiftKey ? outdentTask(li) : indentTask(li);
        return;
      }
      if (e.key === "Backspace" && caretAtStartOfTaskText(li)) {
        e.preventDefault();
        if (!mergeTaskWithPrevious(li) && isNestedLi(li)) outdentTask(li);
        return;
      }
      if (e.key === "Delete" && caretAtEndOfTaskText(li)) {
        e.preventDefault();
        mergeNextTaskInto(li);
        return;
      }
      // Shift+Enter falls through to the soft-line-break below.
    }
    // A plain Enter should behave like any ordinary text box (one new line),
    // not contentEditable's default of starting a whole new block element
    // (which serializes back as a blank-line-separated paragraph, i.e.
    // visibly "double spaced"). Shift+Enter inside a task also lands here.
    if (e.key === "Enter") {
      e.preventDefault();
      document.execCommand("insertLineBreak");
      onPreviewInput();
    }
  }

  // Toggle a task's checkbox on click of its marker: flips data-checked +
  // the `done` class (strikethrough + dimmed via CSS) and saves.
  function onPreviewClick(e: React.MouseEvent) {
    const t = e.target as HTMLElement;
    if (!t.classList?.contains("md-task-check")) return;
    const li = t.closest<HTMLLIElement>("li.md-task");
    if (!li) return;
    const checked = li.getAttribute("data-checked") === "true";
    li.setAttribute("data-checked", String(!checked));
    li.classList.toggle("done", !checked);
    onPreviewInput();
  }

  // Drag-to-reorder (Google-Keep style): drop above/below the hovered item
  // based on the cursor's position within it; the dragged item keeps its
  // own subtree. Nesting is still done with Tab / Shift+Tab.
  const draggedLiRef = useRef<HTMLLIElement | null>(null);
  function onPreviewDragStart(e: React.DragEvent) {
    const li = findTaskLi(e.target as Node);
    if (!li) return;
    draggedLiRef.current = li;
    e.dataTransfer.effectAllowed = "move";
    // WebKitGTK needs dataTransfer populated for drop to fire later.
    e.dataTransfer.setData("text/plain", "");
    li.classList.add("dragging");
  }
  function onPreviewDragOver(e: React.DragEvent) {
    if (!draggedLiRef.current) return;
    const over = findTaskLi(e.target as Node);
    if (over && over !== draggedLiRef.current) e.preventDefault(); // allow drop
  }
  function onPreviewDrop(e: React.DragEvent) {
    const dragged = draggedLiRef.current;
    draggedLiRef.current = null;
    if (dragged) dragged.classList.remove("dragging");
    if (!dragged) return;
    const over = findTaskLi(e.target as Node);
    if (!over || over === dragged || dragged.contains(over)) return;
    e.preventDefault();
    const rect = over.getBoundingClientRect();
    const after = e.clientY > rect.top + rect.height / 2;
    if (after) over.after(dragged);
    else over.before(dragged);
    onPreviewInput();
  }
  function onPreviewDragEnd() {
    draggedLiRef.current?.classList.remove("dragging");
    draggedLiRef.current = null;
  }

  // Insert an empty task list (one item) at the caret and focus it.
  function insertTaskList() {
    const preview = previewRef.current;
    if (!preview) return;
    preview.focus();
    const ul = document.createElement("ul");
    ul.className = "md-tasklist";
    const li = makeTaskLi();
    ul.appendChild(li);
    const sel = window.getSelection();
    const range =
      sel && sel.rangeCount > 0 && preview.contains(sel.getRangeAt(0).commonAncestorContainer)
        ? sel.getRangeAt(0)
        : null;
    if (range) {
      range.collapse(false);
      range.insertNode(ul);
    } else {
      preview.appendChild(ul);
    }
    const t = taskText(li);
    if (t) caretToEnd(t);
    onPreviewInput();
  }

  const PREVIEW_PLACEHOLDER: Record<string, string> = {
    strong: "bold text",
    em: "italic text",
    u: "underlined text",
    del: "strikethrough text",
    code: "code",
  };

  // Wraps the current selection *inside the editable preview* in a
  // specific tag -- deliberately not execCommand for this part, since its
  // browser-chosen output tag (<b> vs <strong>, style attributes...)
  // wouldn't reliably match what serializePreviewToMarkdown() knows how
  // to read back. Block-level ops (heading/list) below still use
  // execCommand, where the DOM restructuring involved is much more than
  // a plain wrap and browser-native behavior is good enough.
  //
  // With nothing selected, there's no text to wrap -- insert placeholder
  // text in the tag instead (pre-selected, so typing right away replaces
  // it) rather than silently doing nothing.
  function wrapPreviewSelection(tagName: string) {
    const preview = previewRef.current;
    if (!preview) return;
    const sel = window.getSelection();
    const activeRange =
      sel && sel.rangeCount > 0 && preview.contains(sel.getRangeAt(0).commonAncestorContainer)
        ? sel.getRangeAt(0)
        : null;
    const el = document.createElement(tagName);
    if (activeRange && !activeRange.collapsed) {
      try {
        activeRange.surroundContents(el);
      } catch {
        const contents = activeRange.extractContents();
        el.appendChild(contents);
        activeRange.insertNode(el);
      }
      sel!.removeAllRanges();
    } else {
      el.textContent = PREVIEW_PLACEHOLDER[tagName] ?? "text";
      if (activeRange) {
        activeRange.collapse(false);
        activeRange.insertNode(el);
      } else {
        preview.appendChild(el);
      }
      const newRange = document.createRange();
      newRange.selectNodeContents(el);
      sel?.removeAllRanges();
      sel?.addRange(newRange);
    }
    onPreviewInput();
  }
  function previewBlockCommand(command: string, value?: string) {
    previewRef.current?.focus();
    document.execCommand(command, false, value);
    onPreviewInput();
  }

  function wrapSelection(before: string, after: string = before) {
    const ta = textareaRef.current;
    if (!ta || content === null) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const next = content.slice(0, start) + before + content.slice(start, end) + after + content.slice(end);
    setContent(next);
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(start + before.length, end + before.length);
    });
  }
  function prefixLines(prefix: string) {
    const ta = textareaRef.current;
    if (!ta || content === null) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const lineStart = content.lastIndexOf("\n", start - 1) + 1;
    const next =
      content.slice(0, lineStart) +
      content
        .slice(lineStart, end)
        .split("\n")
        .map((l) => prefix + l)
        .join("\n") +
      content.slice(end);
    setContent(next);
  }

  const bold = () => (mode === "preview" ? wrapPreviewSelection("strong") : wrapSelection("**"));
  const italic = () => (mode === "preview" ? wrapPreviewSelection("em") : wrapSelection("*"));
  const underline = () => (mode === "preview" ? wrapPreviewSelection("u") : wrapSelection("<u>", "</u>"));
  const strikethrough = () => (mode === "preview" ? wrapPreviewSelection("del") : wrapSelection("~~"));
  const code = () => (mode === "preview" ? wrapPreviewSelection("code") : wrapSelection("`"));
  const bulletList = () =>
    mode === "preview" ? previewBlockCommand("insertUnorderedList") : prefixLines("- ");
  const heading = () => (mode === "preview" ? previewBlockCommand("formatBlock", "H1") : prefixLines("# "));

  return (
    <div className="preview-pane markdown-pane">
      <div className="markdown-pane-header">
        <div className="preview-name-row">
          <EditableFileName name={entry.name} onRename={onRename} />
          {saving && <span className="saving-hint"> — saving…</span>}
        </div>
        <div className="segmented">
          <button
            className={`seg seg-text ${mode === "preview" ? "on" : ""}`}
            onClick={() => setMode("preview")}
          >
            Preview
          </button>
          <button
            className={`seg seg-text ${mode === "source" ? "on" : ""}`}
            onClick={() => setMode("source")}
          >
            Source
          </button>
        </div>
      </div>
      {error && <p className="error">{error}</p>}
      {pasteError && <p className="error">{pasteError}</p>}
      {content !== null && (
        <>
          <div className="markdown-toolbar">
            {/* onMouseDown/preventDefault on every button here, not just
                onClick, matters a lot: a plain click focuses the button
                first, which collapses whatever text selection was live in
                the contentEditable preview below -- by the time onClick
                ran, wrapPreviewSelection had nothing left to wrap. */}
            <button
              className="btn-plain small"
              title="Bold"
              onMouseDown={(e) => e.preventDefault()}
              onClick={bold}
            >
              <strong>B</strong>
            </button>
            <button
              className="btn-plain small"
              title="Italic"
              onMouseDown={(e) => e.preventDefault()}
              onClick={italic}
            >
              <em>I</em>
            </button>
            <button
              className="btn-plain small"
              title="Underline"
              onMouseDown={(e) => e.preventDefault()}
              onClick={underline}
            >
              <u>U</u>
            </button>
            <button
              className="btn-plain small"
              title="Strikethrough"
              onMouseDown={(e) => e.preventDefault()}
              onClick={strikethrough}
            >
              <s>S</s>
            </button>
            <button
              className="btn-plain small"
              title="Code"
              onMouseDown={(e) => e.preventDefault()}
              onClick={code}
            >
              {"</>"}
            </button>
            <button
              className="btn-plain small"
              title="Bullet List"
              onMouseDown={(e) => e.preventDefault()}
              onClick={bulletList}
            >
              •
            </button>
            <button
              className="btn-plain small"
              title="Heading"
              onMouseDown={(e) => e.preventDefault()}
              onClick={heading}
            >
              H
            </button>
            <button
              className="btn-plain small"
              title="Checklist"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => (mode === "preview" ? insertTaskList() : prefixLines("- [ ] "))}
            >
              ☑
            </button>
            <button
              className="btn-plain small"
              title="Insert image"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => setShowImagePicker(true)}
            >
              🖼
            </button>
          </div>
          {mode === "preview" ? (
            <div
              ref={previewRef}
              className="markdown-rendered markdown-editable"
              contentEditable
              suppressContentEditableWarning
              onInput={onPreviewInput}
              onKeyDown={onPreviewKeyDown}
              onPaste={onPreviewPaste}
              onClick={onPreviewClick}
              onDragStart={onPreviewDragStart}
              onDragOver={onPreviewDragOver}
              onDrop={onPreviewDrop}
              onDragEnd={onPreviewDragEnd}
            />
          ) : (
            <textarea
              ref={textareaRef}
              className="text-editor-area"
              value={content}
              onChange={(e) => setContent(e.target.value)}
              spellCheck={false}
            />
          )}
        </>
      )}
      {showImagePicker && (
        <ImagePickerModal
          startDir={parentPath(fullPath)}
          startInVault={inVault}
          canBrowseVault={inVault}
          onCancel={() => setShowImagePicker(false)}
          onPick={insertPickedImage}
        />
      )}
    </div>
  );
}

// Folder preview: a folder has no "content" to render, so show its listing
// (name + item count + a scrollable peek at what's inside), same in a vault
// or on the real fs. Uses list_dir/fs_list -- no decryption involved.
function FolderPreview({
  entry,
  fullPath,
  inVault,
  onRename,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onRename?: (newName: string) => void;
}) {
  const [items, setItems] = useState<Entry[] | null>(null);
  useEffect(() => {
    const call = inVault ? api.listDir(fullPath) : api.fsList(fullPath, false);
    call.then(setItems).catch(() => setItems([]));
  }, [fullPath, inVault]);
  const sorted = (items ?? [])
    .slice()
    .sort((a, b) => (a.is_dir === b.is_dir ? a.name.localeCompare(b.name) : a.is_dir ? -1 : 1));
  return (
    <div className="preview-pane text-editor-pane">
      <div className="preview-name-row">
        <EditableFileName name={entry.name} onRename={onRename} />
      </div>
      <p className="hint" style={{ marginTop: 2 }}>
        {items === null ? "…" : `${items.length} item${items.length === 1 ? "" : "s"}`}
      </p>
      <div className="folder-preview-list">
        {sorted.map((e) => (
          <div className="folder-preview-item" key={e.name}>
            <FileIcon entry={e} />
            <span>{e.name}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function FilePreviewPane({
  target,
  inVault,
  root,
  onRename,
}: {
  target: { dir: string; entry: Entry } | null;
  inVault: boolean;
  root?: string;
  onRename?: (newName: string) => void;
}) {
  if (!target) {
    return (
      <div className="preview-pane preview-pane-empty">
        <p className="hint">Select a file to preview it here.</p>
      </div>
    );
  }
  const { dir, entry } = target;
  const fullPath = joinPath(dir, entry.name);
  const ext = entry.name.toLowerCase().split(".").pop() ?? "";

  if (entry.is_dir) {
    return <FolderPreview key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} onRename={onRename} />;
  }
  if (ext === "md") {
    return <MarkdownEditorPane key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} onRename={onRename} />;
  }
  if (kindOf(entry) === "text") {
    return <TextEditorPane key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} onRename={onRename} />;
  }
  // Unlike the other branches above, PreviewColumn is also used as-is by
  // ColumnView's genuine Miller columns, where its `.column` class's fixed
  // 390px width is correct (every column there is fixed-width). Wrapping
  // it in `.preview-pane` here is what makes *this* call site (List with
  // Preview's right-hand pane) stretch to fill the remaining width instead
  // of inheriting that same fixed 390px meant for a different layout.
  return (
    <div className="preview-pane">
      <PreviewColumn key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} root={root} onRename={onRename} />
    </div>
  );
}
