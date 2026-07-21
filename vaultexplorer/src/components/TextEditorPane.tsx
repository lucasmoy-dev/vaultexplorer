import { useEffect, useRef, useState } from "react";
import { Entry, api, joinPath, parentPath } from "../api";
import { kindOf } from "../icons";
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

  // Tracks a live resize (see the `.md-img` CSS `resize: both`) back into
  // the attribute htmlNodeToMarkdown reads, and re-serializes so the new
  // size actually gets saved -- a native CSS resize handle has no "resize
  // finished" event of its own to hook into instead.
  const imageResizeObserverRef = useRef<ResizeObserver | null>(null);
  useEffect(() => {
    imageResizeObserverRef.current = new ResizeObserver((observed) => {
      for (const o of observed) {
        (o.target as HTMLImageElement).setAttribute("data-md-width", String(Math.round(o.contentRect.width)));
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
      imageResizeObserverRef.current?.observe(img);
      if (img.src) return;
      const rel = img.getAttribute("data-md-src");
      if (!rel) return;
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
    const img = document.createElement("img");
    img.className = "md-img";
    img.setAttribute("data-md-src", relSrc);
    img.src = dataUrl;
    imageResizeObserverRef.current?.observe(img);
    const sel = window.getSelection();
    const range =
      sel && sel.rangeCount > 0 && preview.contains(sel.getRangeAt(0).commonAncestorContainer)
        ? sel.getRangeAt(0)
        : null;
    if (range) {
      range.collapse(false);
      range.insertNode(img);
    } else {
      preview.appendChild(img);
    }
    onPreviewInput();
  }

  // Real fs only: a vault file has nowhere to write a plaintext sibling
  // image file into without breaking the vault's own invariant.
  async function onPreviewPaste(e: React.ClipboardEvent) {
    if (inVault) return;
    const item = [...e.clipboardData.items].find((it) => it.type.startsWith("image/"));
    if (!item) return;
    const file = item.getAsFile();
    if (!file) return;
    e.preventDefault();
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    const dir = parentPath(fullPath);
    try {
      const relName = await api.fsSavePastedImage(dir, bytes);
      const dataUrl = await api.fsThumbnail(joinPath(dir, relName), 2000);
      insertImageAtCaret(relName, dataUrl);
    } catch (err) {
      setPasteError(String(err));
    }
  }

  // A plain Enter should behave like any ordinary text box (one new
  // line), not the browser's contentEditable default of starting a whole
  // new block element -- which serializePreviewToMarkdown renders back as
  // a blank-line-separated paragraph, i.e. visibly "double spaced".
  function onPreviewKeyDown(e: React.KeyboardEvent) {
    if (e.key !== "Enter") return;
    e.preventDefault();
    document.execCommand("insertLineBreak");
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

  if (ext === "md") {
    return <MarkdownEditorPane key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} onRename={onRename} />;
  }
  if (kindOf(entry) === "text") {
    return <TextEditorPane key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} onRename={onRename} />;
  }
  return (
    <PreviewColumn key={fullPath} entry={entry} fullPath={fullPath} inVault={inVault} root={root} onRename={onRename} />
  );
}
