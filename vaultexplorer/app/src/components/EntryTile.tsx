import { useEffect, useRef } from "react";
import { Entry } from "../api";
import { View } from "../types";
import { formatSize, formatDate } from "../api";
import { FileIcon, PinGlyph, LockGlyph } from "../icons";
import { displayEntryName, kindLabel } from "../entryHelpers";
import { useThumbnail } from "../hooks/useThumbnail";

export function EntryTile({
  entry,
  fullPath,
  inVault,
  view,
  compact,
  selected,
  cut,
  isDropTarget,
  tagHex,
  customIcon,
  hideExtensions,
  pinned,
  sensitive,
  mobile,
  editing,
  editValue,
  onEditChange,
  onEditCommit,
  onEditCancel,
  onClick,
  onOpen,
  onMenu,
  onDragStart,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  view: View;
  compact?: boolean;
  selected: boolean;
  cut?: boolean;
  isDropTarget: boolean;
  tagHex?: string;
  customIcon?: string;
  hideExtensions?: boolean;
  pinned?: boolean;
  sensitive?: boolean;
  mobile?: boolean;
  editing: boolean;
  editValue: string;
  onEditChange: (v: string) => void;
  onEditCommit: () => void;
  onEditCancel: () => void;
  onClick: (e: React.MouseEvent) => void;
  onOpen: () => void;
  onMenu: (e: React.MouseEvent) => void;
  // Pointer moved far enough with the button held: start dragging out.
  onDragStart: () => void;
  onDragOver?: (e: React.DragEvent) => void;
  onDragLeave?: () => void;
  onDrop?: (e: React.DragEvent) => void;
}) {
  const tileRef = useRef<HTMLDivElement>(null);
  const editRef = useRef<HTMLTextAreaElement>(null);
  // Grow the rename field to fit the whole wrapped filename instead of
  // scrolling inside a fixed 2-line box. `rows` only counts *logical*
  // lines (a filename is one), so height has to be driven off the actual
  // rendered scrollHeight -- recomputed on every keystroke and on entering
  // rename. Only meaningful in icon view (list rename stays single-line).
  useEffect(() => {
    if (!editing || view !== "icon") return;
    const el = editRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [editing, editValue, view]);
  // Always the same size regardless of `view`: the <img> is CSS-scaled
  // down for list/listPreview's smaller thumbnail box either way, so
  // there's no quality reason to ask for two sizes -- asking for a
  // different one per view was why switching views took a couple of
  // seconds, forcing a full re-decode+resize+re-encode (and, for images
  // inside a vault, a full re-decrypt too -- vault thumbnails are never
  // disk-cached) of every visible thumbnail purely because the view
  // changed, not because the folder did.
  const thumb = useThumbnail(entry, fullPath, inVault, 160, tileRef);

  // Dragging a tile out starts the native OS drag (see beginDrag in
  // App.tsx) once the pointer has moved past a small threshold with the
  // button held. It used to wait for WebKit's own HTML5 `dragstart`, with
  // `draggable` switched on only after that same threshold: WebKit decides
  // at mousedown whether a press may start a drag, so a press on a tile
  // that wasn't draggable *yet* often never produced one -- the first
  // drag after opening a folder did nothing, and which part of the tile
  // you grabbed seemed to matter. The tile is never `draggable` now (which
  // also keeps a double-click's jitter from being read as a drag).
  const pointerDownAt = useRef<{ x: number; y: number } | null>(null);

  function onPointerDown(e: React.PointerEvent) {
    if (e.button !== 0 || editing || mobile) return;
    pointerDownAt.current = { x: e.clientX, y: e.clientY };
  }
  function onPointerMove(e: React.PointerEvent) {
    const start = pointerDownAt.current;
    if (!start) return;
    if (!(e.buttons & 1)) {
      pointerDownAt.current = null;
      return;
    }
    if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) {
      pointerDownAt.current = null;
      onDragStart();
    }
  }
  function onPointerUp() {
    pointerDownAt.current = null;
  }

  return (
    <div
      ref={tileRef}
      className={`entry ${view} ${compact ? "compact" : ""} ${selected ? "selected" : ""} ${
        isDropTarget ? "drop" : ""
      } ${entry.is_hidden ? "dimmed" : ""} ${cut ? "cut" : ""} ${
        ""
      }`}
      data-name={entry.name}
      title={editing ? undefined : entry.name}
      draggable={false}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onClick={editing ? undefined : onClick}
      onDoubleClick={editing ? undefined : onOpen}
      onContextMenu={editing ? undefined : onMenu}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <span className="entry-icon">
        {thumb ? (
          <img src={thumb} className="entry-thumb" alt="" decoding="async" draggable={false} />
        ) : (
          <FileIcon entry={entry} tagHex={entry.is_dir ? tagHex : undefined} customIcon={customIcon} />
        )}
        {tagHex && !entry.is_dir && <span className="entry-tag-dot" style={{ background: tagHex }} />}
        {pinned && (
          <span className="entry-pin-badge">
            <PinGlyph size={11} />
          </span>
        )}
        {sensitive && (
          <span className="entry-sensitive-badge" title="Sensitive">
            <LockGlyph size={11} />
          </span>
        )}
      </span>
      {editing ? (
        <textarea
          ref={editRef}
          autoFocus
          rows={1}
          // Width: a textarea sizes itself from `cols`, not from its content,
          // so leaving the default (20) made every icon-view rename box come
          // out at its max width even for a 3-character name. Driving cols
          // off the text (clamped, and only in icon view -- list rename is a
          // fixed-width single line) makes the field hug the name, with the
          // CSS min/max-width as the real bounds.
          cols={view === "icon" ? Math.max(4, Math.min(14, editValue.length)) : undefined}
          className="entry-name-edit"
          value={editValue}
          onClick={(e) => e.stopPropagation()}
          onChange={(e) => onEditChange(e.target.value)}
          onFocus={(e) => {
            const dot = editValue.lastIndexOf(".");
            e.currentTarget.setSelectionRange(0, dot > 0 ? dot : editValue.length);
          }}
          onBlur={onEditCommit}
          onKeyDown={(e) => {
            e.stopPropagation();
            // A plain textarea would otherwise insert a literal newline
            // here instead of committing -- filenames are one logical
            // line even though this field wraps that line visually across
            // several.
            if (e.key === "Enter") {
              e.preventDefault();
              onEditCommit();
            }
            if (e.key === "Escape") onEditCancel();
          }}
        />
      ) : (
        <span className="entry-name">
          {displayEntryName(entry, !!hideExtensions)}

        </span>
      )}
      {view === "list" && !compact && (
        <>
          <span className="entry-date">{formatDate(entry.mtime)}</span>
          <span className="entry-size">{entry.is_dir ? "--" : formatSize(entry.size)}</span>
          <span className="entry-kind">{kindLabel(entry)}</span>
        </>
      )}
    </div>
  );
}
