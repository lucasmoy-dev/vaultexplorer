import { useEffect, useMemo, useRef, useState } from "react";
import { Entry, api, joinPath, parentPath } from "../api";
import { kindOf, FileIcon } from "../icons";
import { highlightCode, renderMarkdownToHtml, serializePreviewToMarkdown } from "../markdown";
import { useAutoSaveText } from "../hooks/useAutoSaveText";
import { useTextHistory, handleHistoryKeyDown } from "../hooks/useTextHistory";
import { useThumbnail } from "../hooks/useThumbnail";
import { listKeyDown, numberLines, renumberOrderedLists } from "../listEdit";
import { EditableFileName, PreviewColumn } from "./PreviewColumn";

// Extensions worth colouring. Deliberately a list rather than "anything
// that isn't prose": highlighting a .txt shopping list would be noise, and
// the highlighter is a small hand-rolled one (see markdown.ts), not a
// grammar per language -- it earns its keep on source files and nowhere
// else.
const CODE_EXT_RE =
  /\.(rs|ts|tsx|js|jsx|mjs|cjs|py|go|c|h|cpp|hpp|cc|java|kt|rb|php|swift|sh|bash|zsh|sql|json|jsonc|toml|yaml|yml|css|scss|html|xml|ini|conf)$/i;

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
  const history = useTextHistory(content, setContent, fullPath);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const ext = entry.name.slice(entry.name.lastIndexOf(".") + 1).toLowerCase();
  const isCode = CODE_EXT_RE.test(entry.name);
  // Source files open coloured (reading is the common case) but stay one
  // click from the plain textarea, since the highlighted view is rendered
  // HTML and can't be typed into.
  const [reading, setReading] = useState(isCode);
  const highlighted = useMemo(
    () => (isCode && reading && content !== null ? highlightCode(content, ext) : ""),
    [isCode, reading, content, ext]
  );
  // Markdown gets the same list continuation the markdown pane's Source
  // mode has -- this is the editor Notes and the mobile editor screen
  // actually open a .md in, so without it Enter after "1. milk" landed on
  // a bare line and every item had to be numbered by hand.
  const isMarkdown = ext === "md" || ext === "markdown" || ext === "mdown";
  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (handleHistoryKeyDown(e, history)) return;
    if (!isMarkdown || content === null) return;
    const edit = listKeyDown(e, content);
    if (!edit) return;
    history.record();
    setContent(edit.text);
    requestAnimationFrame(() => textareaRef.current?.setSelectionRange(edit.caret, edit.caret));
  }
  return (
    <div className="preview-pane text-editor-pane">
      <div className="preview-name-row">
        <EditableFileName name={entry.name} onRename={onRename} />
        {saving && <span className="saving-hint"> — saving…</span>}
        {/* Buttons, not just Ctrl+Z: this pane is what a phone opens a
            note in, and a touch keyboard has no chord for undo. */}
        <button
          className="code-mode-btn"
          title="Undo"
          disabled={!history.canUndo}
          onMouseDown={(e) => e.preventDefault()}
          onClick={history.undo}
          aria-label="Undo"
        >
          ↺
        </button>
        <button
          className="code-mode-btn"
          title="Redo"
          disabled={!history.canRedo}
          onMouseDown={(e) => e.preventDefault()}
          onClick={history.redo}
          aria-label="Redo"
        >
          ↻
        </button>
        {isMarkdown && (
          <button
            className="code-mode-btn"
            title="Numbered list"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              const ta = textareaRef.current;
              if (!ta || content === null) return;
              const edit = numberLines(content, ta.selectionStart, ta.selectionEnd);
              history.record();
              setContent(edit.text);
              requestAnimationFrame(() => ta.setSelectionRange(edit.caret, edit.caret));
            }}
          >
            1.
          </button>
        )}
        {isCode && (
          <button className="code-mode-btn" onClick={() => setReading((r) => !r)}>
            {reading ? "Edit" : "Highlight"}
          </button>
        )}
      </div>
      {error && <p className="error">{error}</p>}
      {content !== null &&
        (isCode && reading ? (
          <pre
            className="md-code text-editor-code"
            data-md-lang={ext}
            onDoubleClick={() => setReading(false)}
            title="Double-click to edit"
            dangerouslySetInnerHTML={{ __html: `<code>${highlighted}</code>` }}
          />
        ) : (
          <textarea
            ref={textareaRef}
            className="text-editor-area"
            value={content}
            onChange={(e) => {
              history.record();
              setContent(e.target.value);
            }}
            onKeyDown={onKeyDown}
            spellCheck={false}
          />
        ))}
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
  const history = useTextHistory(content, setContent, fullPath);
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
  }, [fullPath, mode, content !== null, externalRevision, history.restoreRevision]);

  function onPreviewInput() {
    if (!previewRef.current) return;
    // Snapshot *before* the edit lands (see useTextHistory): `content` is
    // still the pre-edit text at this point.
    history.record();
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

  // ---- find in file (Ctrl/Cmd+F) --------------------------------------
  // Editing a long note without a way to jump to a word means scrolling
  // and reading -- the one thing every other text surface on the machine
  // can do. Works on the raw text (selecting each hit in the textarea) and
  // on the rendered view (scrolling the match into view), so it is not
  // tied to which mode happens to be open.
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [findIndex, setFindIndex] = useState(0);
  const findInputRef = useRef<HTMLInputElement>(null);

  const findHits = useMemo(() => {
    const text = content ?? "";
    if (!findQuery) return [] as number[];
    const hits: number[] = [];
    const needle = findQuery.toLowerCase();
    const hay = text.toLowerCase();
    let from = 0;
    // Overlapping matches are not what "next result" means to anyone, so
    // this steps past each hit rather than by one character.
    for (;;) {
      const at = hay.indexOf(needle, from);
      if (at === -1) break;
      hits.push(at);
      from = at + needle.length;
    }
    return hits;
  }, [content, findQuery]);

  function gotoHit(index: number) {
    if (findHits.length === 0) return;
    const wrapped = (index + findHits.length) % findHits.length;
    setFindIndex(wrapped);
    const at = findHits[wrapped];
    const el = textareaRef.current;
    if (el) {
      el.focus();
      el.setSelectionRange(at, at + findQuery.length);
      // Scroll the selection into view: a textarea won't do it on its own
      // when the selection is set programmatically. Measuring by line is
      // close enough and costs nothing.
      const line = (content ?? "").slice(0, at).split("\n").length - 1;
      const lineHeight = parseFloat(getComputedStyle(el).lineHeight || "18") || 18;
      el.scrollTop = Math.max(0, line * lineHeight - el.clientHeight / 2);
      return;
    }
    // Rendered mode: walk the text nodes for the same offset-th match.
    const root = previewRef.current;
    if (!root) return;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let seen = 0;
    while (walker.nextNode()) {
      const node = walker.currentNode as Text;
      const idx = node.data.toLowerCase().indexOf(findQuery.toLowerCase());
      if (idx === -1) continue;
      if (seen === wrapped) {
        const range = document.createRange();
        range.setStart(node, idx);
        range.setEnd(node, idx + findQuery.length);
        const sel = window.getSelection();
        sel?.removeAllRanges();
        sel?.addRange(range);
        (node.parentElement as HTMLElement | null)?.scrollIntoView({ block: "center", behavior: "smooth" });
        return;
      }
      seen++;
    }
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        e.stopPropagation();
        setFindOpen(true);
        requestAnimationFrame(() => findInputRef.current?.select());
      } else if (e.key === "Escape" && findOpen) {
        setFindOpen(false);
      }
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [findOpen]);

  // ---- raw-text list behaviour ---------------------------------------
  // Source mode shares its Enter/Tab list handling (and the renumbering
  // that keeps "1. 2. 3." counting up) with the plain text editor above
  // -- see listEdit.ts.
  function onTextareaKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (handleHistoryKeyDown(e, history)) return;
    if (content === null) return;
    const edit = listKeyDown(e, content);
    if (!edit) return;
    history.record();
    setContent(edit.text);
    requestAnimationFrame(() => textareaRef.current?.setSelectionRange(edit.caret, edit.caret));
  }

  // ---- plain list items (bullets and numbers) in the rendered editor ---
  // Task items have had their own keyboard handling for a while; plain
  // ones had none, so Enter fell through to the soft-line-break below and
  // a numbered list could never grow past its first item -- every later
  // line folded into item 1 on save, which is what "it always writes 1."
  // looks like from the outside. The browser numbers an <ol> itself, so
  // all this has to do is produce a real new <li>.
  function findPlainLi(node: Node | null): HTMLLIElement | null {
    let n: Node | null = node;
    while (n && n !== previewRef.current) {
      if (n.nodeType === Node.ELEMENT_NODE && (n as HTMLElement).matches?.("li:not(.md-task)")) {
        return n as HTMLLIElement;
      }
      n = n.parentNode;
    }
    return null;
  }

  // Everything from the caret to the end of the item, moved into a new
  // sibling item -- so Enter in the middle of an item splits it, and
  // Enter at the end just opens an empty one.
  function splitPlainLi(li: HTMLLIElement) {
    const list = li.parentElement as HTMLElement | null;
    if (!list) return;
    const sel = window.getSelection();
    const nli = document.createElement("li");
    // A nested sublist belongs to the item being left behind, not to the
    // new one, so the extraction stops before it.
    const sub = li.querySelector(":scope > ul, :scope > ol");
    const lastOwn = sub ? sub.previousSibling : li.lastChild;
    // Never set the range's end outside `li` itself -- extracting from a
    // caret to a point *after* the item would tear the item out of the
    // list rather than split it.
    if (sel && sel.rangeCount > 0 && lastOwn && li.contains(sel.getRangeAt(0).startContainer)) {
      const range = sel.getRangeAt(0).cloneRange();
      range.setEndAfter(lastOwn);
      try {
        nli.appendChild(range.extractContents());
      } catch {
        /* nothing after the caret -- new item just starts empty */
      }
    }
    if (!nli.firstChild) nli.appendChild(document.createElement("br"));
    li.after(nli);
    if (!li.firstChild) li.appendChild(document.createElement("br"));
    caretToStart(nli);
    onPreviewInput();
  }

  function caretToStart(el: HTMLElement) {
    const r = document.createRange();
    r.selectNodeContents(el);
    r.collapse(true);
    const s = window.getSelection();
    s?.removeAllRanges();
    s?.addRange(r);
  }

  // Enter on an empty item leaves the list: one level out if nested,
  // otherwise back to ordinary text below the list.
  function exitPlainList(li: HTMLLIElement): boolean {
    const list = li.parentElement as HTMLElement | null;
    if (!list) return false;
    if (isNestedLi(li)) {
      outdentPlainLi(li);
      return true;
    }
    const p = document.createElement("div");
    p.innerHTML = "<br>";
    list.after(p);
    li.remove();
    if (list.children.length === 0) list.remove();
    caretToEnd(p);
    onPreviewInput();
    return true;
  }

  function indentPlainLi(li: HTMLLIElement) {
    const prev = li.previousElementSibling as HTMLElement | null;
    if (!prev || prev.tagName !== "LI") return; // first item can't indent
    const listTag = (li.parentElement?.tagName ?? "UL").toLowerCase();
    let sub = prev.querySelector<HTMLElement>(":scope > ul, :scope > ol");
    if (!sub) {
      sub = document.createElement(listTag);
      prev.appendChild(sub);
    }
    sub.appendChild(li);
    caretToEnd(li);
    onPreviewInput();
  }
  function outdentPlainLi(li: HTMLLIElement) {
    const list = li.parentElement as HTMLElement | null;
    const parentLi = list?.parentElement as HTMLElement | null;
    if (!parentLi || parentLi.tagName !== "LI") return; // already top level
    parentLi.after(li);
    if (list && list.children.length === 0) list.remove();
    caretToEnd(li);
    onPreviewInput();
  }

  function onPreviewKeyDown(e: React.KeyboardEvent) {
    // Before anything else: the browser's own contentEditable undo stack
    // is useless here (the pane re-renders from `content`, which wipes
    // it), so Ctrl+Z has to go through this pane's own history.
    if (handleHistoryKeyDown(e, history)) return;
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
    const plainLi = li ? null : findPlainLi(window.getSelection()?.anchorNode ?? null);
    if (plainLi) {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        if ((plainLi.textContent ?? "").trim() === "") {
          if (exitPlainList(plainLi)) return;
        }
        splitPlainLi(plainLi);
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        e.shiftKey ? outdentPlainLi(plainLi) : indentPlainLi(plainLi);
        return;
      }
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
    // Ticked items sink to the bottom of their own list, the way Keep
    // does it: what's left to do stays together at the top instead of
    // being interleaved with what's already finished. Unticking pulls the
    // item back up above the done ones, so undo lands where you expect.
    const ul = li.parentElement;
    if (ul) {
      const siblings = [...ul.children].filter((n): n is HTMLLIElement => n.matches("li.md-task"));
      if (!checked) {
        ul.appendChild(li);
      } else {
        const firstDone = siblings.find((n) => n !== li && n.getAttribute("data-checked") === "true");
        if (firstDone) ul.insertBefore(li, firstDone);
        else ul.appendChild(li);
      }
    }
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
    history.record();
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
    history.record();
    setContent(next);
  }

  // Numbered list, source mode: each line gets its own number, and the
  // whole run is renumbered afterwards so it still counts up if it ran
  // into a list that was already there.
  function numberSelectedLines() {
    const ta = textareaRef.current;
    if (!ta || content === null) return;
    const numbered = numberLines(content, ta.selectionStart, ta.selectionEnd);
    const edit = renumberOrderedLists(numbered.text, numbered.caret);
    history.record();
    setContent(edit.text);
    requestAnimationFrame(() => ta.setSelectionRange(edit.caret, edit.caret));
  }

  const bold = () => (mode === "preview" ? wrapPreviewSelection("strong") : wrapSelection("**"));
  const italic = () => (mode === "preview" ? wrapPreviewSelection("em") : wrapSelection("*"));
  const underline = () => (mode === "preview" ? wrapPreviewSelection("u") : wrapSelection("<u>", "</u>"));
  const strikethrough = () => (mode === "preview" ? wrapPreviewSelection("del") : wrapSelection("~~"));
  const code = () => (mode === "preview" ? wrapPreviewSelection("code") : wrapSelection("`"));
  const bulletList = () =>
    mode === "preview" ? previewBlockCommand("insertUnorderedList") : prefixLines("- ");
  const numberedList = () =>
    mode === "preview" ? previewBlockCommand("insertOrderedList") : numberSelectedLines();
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
              title="Numbered List"
              onMouseDown={(e) => e.preventDefault()}
              onClick={numberedList}
            >
              1.
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
            {/* Undo/redo as buttons as well as Ctrl+Z / Ctrl+Shift+Z:
                both editing surfaces here rewrite their own content
                programmatically, which is exactly what the browser's
                native undo can't follow (see useTextHistory). */}
            <button
              className="btn-plain small"
              title="Undo (Ctrl+Z)"
              disabled={!history.canUndo}
              onMouseDown={(e) => e.preventDefault()}
              onClick={history.undo}
              aria-label="Undo"
            >
              ↺
            </button>
            <button
              className="btn-plain small"
              title="Redo (Ctrl+Shift+Z)"
              disabled={!history.canRedo}
              onMouseDown={(e) => e.preventDefault()}
              onClick={history.redo}
              aria-label="Redo"
            >
              ↻
            </button>
          </div>
          {findOpen && (
            <div className="find-bar">
              <input
                ref={findInputRef}
                className="find-input"
                value={findQuery}
                placeholder="Find in file"
                onChange={(e) => {
                  setFindQuery(e.target.value);
                  setFindIndex(0);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    gotoHit(e.shiftKey ? findIndex - 1 : findIndex + (findHits.length && findQuery ? 1 : 0));
                  }
                  if (e.key === "Escape") setFindOpen(false);
                }}
              />
              <span className="find-count">
                {findQuery ? (findHits.length ? `${findIndex + 1}/${findHits.length}` : "0/0") : ""}
              </span>
              <button className="find-btn" onClick={() => gotoHit(findIndex - 1)} disabled={!findHits.length} aria-label="Previous match">
                ‹
              </button>
              <button className="find-btn" onClick={() => gotoHit(findIndex + 1)} disabled={!findHits.length} aria-label="Next match">
                ›
              </button>
              <button className="find-btn" onClick={() => setFindOpen(false)} aria-label="Close find">
                ✕
              </button>
            </div>
          )}
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
              onChange={(e) => {
                history.record();
                setContent(e.target.value);
              }}
              onKeyDown={onTextareaKeyDown}
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
  onChildActivate,
  onChildMenu,
  reloadKey = 0,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onRename?: (newName: string) => void;
  // Double-click on a listed child: navigate into it (folder) or open it
  // (file) -- wired by App.tsx to the same `activate` the main grid uses.
  onChildActivate?: (child: Entry) => void;
  // Right-click on a listed child -- App.tsx shows its path-based menu.
  onChildMenu?: (e: React.MouseEvent, child: Entry) => void;
  // Bumped by the parent when a menu action changed this folder's
  // contents (the fs watcher only covers the browsed dir, not this one).
  reloadKey?: number;
}) {
  const [items, setItems] = useState<Entry[] | null>(null);
  const [lockedVault, setLockedVault] = useState(false);
  // A vault folder previews its DECRYPTED top level when unlocked, and a
  // lock notice otherwise -- never the raw ciphertext names fs_list sees.
  const isVaultEntry = !!entry.is_vault;
  useEffect(() => {
    setLockedVault(false);
    const call = isVaultEntry
      ? inVault
        ? Promise.reject(new Error("nested vault")) // no root key available here
        : api.vaultListDirAt(fullPath, "")
      : inVault
      ? api.listDir(fullPath)
      : api.fsList(fullPath, false);
    call.then(setItems).catch(() => {
      setLockedVault(isVaultEntry);
      setItems([]);
    });
  }, [fullPath, inVault, isVaultEntry, reloadKey]);
  const sorted = (items ?? [])
    .slice()
    .sort((a, b) => (a.is_dir === b.is_dir ? a.name.localeCompare(b.name) : a.is_dir ? -1 : 1));
  return (
    <div className="preview-pane text-editor-pane">
      <div className="preview-name-row">
        <EditableFileName name={entry.name} onRename={onRename} />
      </div>
      <p className="hint" style={{ marginTop: 2 }}>
        {lockedVault
          ? "🔒 Locked vault — unlock it to preview its contents"
          : items === null
          ? "…"
          : `${items.length} item${items.length === 1 ? "" : "s"}`}
      </p>
      {/* A thumbnail grid, not a list of names: a folder of screenshots is
          thirty near-identical filenames, which read as one solid block and
          tell you nothing about what's in there. Tiles show the actual
          contents and the names sit under them where length doesn't
          matter. */}
      <div className="folder-preview-grid">
        {sorted.map((e) => (
          <FolderPreviewTile
            key={e.name}
            entry={e}
            fullPath={`${fullPath}/${e.name}`}
            inVault={inVault}
            // Children of a previewed vault live in vault space, not under
            // this fs path -- activating/right-clicking them as fs entries
            // would target paths that don't exist. Enter the vault instead.
            onActivate={onChildActivate && !isVaultEntry ? () => onChildActivate(e) : undefined}
            onMenu={onChildMenu && !isVaultEntry ? (ev: React.MouseEvent) => onChildMenu(ev, e) : undefined}
          />
        ))}
      </div>
    </div>
  );
}

// One tile in the folder preview. Its own component so the thumbnail is
// only requested once the tile scrolls into view (useThumbnail's ref
// gate) -- previewing a folder of hundreds of photos shouldn't decode all
// of them just to show the top row.
function FolderPreviewTile({
  entry,
  fullPath,
  inVault,
  onActivate,
  onMenu,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  onActivate?: () => void;
  onMenu?: (e: React.MouseEvent) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const thumb = useThumbnail(entry, fullPath, inVault, 128, ref);
  return (
    <div
      ref={ref}
      className="folder-preview-tile"
      title={entry.name}
      onDoubleClick={onActivate}
      onContextMenu={onMenu}
    >
      <span className="folder-preview-thumb">
        {thumb ? <img src={thumb} alt="" draggable={false} /> : <FileIcon entry={entry} />}
      </span>
      <span className="folder-preview-name">{entry.name}</span>
    </div>
  );
}

export function FilePreviewPane({
  target,
  inVault,
  root,
  onRename,
  textEditorExts,
  onOpenInEditor,
  onChildActivate,
  onChildMenu,
  reloadKey,
}: {
  target: { dir: string; entry: Entry } | null;
  inVault: boolean;
  root?: string;
  onRename?: (newName: string) => void;
  // Extensions the user chose to edit as text even though `kindOf` doesn't
  // classify them as text (see App.tsx). Lowercase, no dot.
  textEditorExts: Set<string>;
  // Called by the preview's "Edit" button: remembers this format so it (and
  // every later file of it) lands in the editor instead of the info panel.
  onOpenInEditor: (ext: string) => void;
  // Folder-preview interactions, threaded to FolderPreview (see it).
  onChildActivate?: (child: Entry) => void;
  onChildMenu?: (e: React.MouseEvent, child: Entry) => void;
  reloadKey?: number;
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
    return (
      <FolderPreview
        key={fullPath}
        entry={entry}
        fullPath={fullPath}
        inVault={inVault}
        onRename={onRename}
        onChildActivate={onChildActivate}
        onChildMenu={onChildMenu}
        reloadKey={reloadKey}
      />
    );
  }
  if (ext === "md") {
    return <MarkdownEditorPane key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} onRename={onRename} />;
  }
  const kind = kindOf(entry);
  if (kind === "text" || textEditorExts.has(ext)) {
    return <TextEditorPane key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} onRename={onRename} />;
  }
  // Which formats are worth offering "Edit" for at all: source/config files
  // (kind "code") and anything unrecognised, which is where an extension the
  // app has never heard of lands. Deliberately not offered for the kinds we
  // know are binary -- image/video/audio/pdf/archive/office -- since loading
  // those into a text editor and saving would corrupt them.
  const editable = kind === "code" || kind === "generic";
  // Unlike the other branches above, PreviewColumn is also used as-is by
  // ColumnView's genuine Miller columns, where its `.column` class's fixed
  // 390px width is correct (every column there is fixed-width). Wrapping
  // it in `.preview-pane` here is what makes *this* call site (List with
  // Preview's right-hand pane) stretch to fill the remaining width instead
  // of inheriting that same fixed 390px meant for a different layout.
  return (
    <div className="preview-pane">
      <PreviewColumn
        key={fullPath}
        entry={entry}
        fullPath={fullPath}
        inVault={inVault}
        root={root}
        onRename={onRename}
        onEdit={editable && ext ? () => onOpenInEditor(ext) : undefined}
      />
    </div>
  );
}
