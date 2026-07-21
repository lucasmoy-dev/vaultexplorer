import { useRef } from "react";
import { Entry } from "../api";
import { View } from "../types";
import { formatSize, formatDate } from "../api";
import { FileIcon, GitBranchGlyph, CloudSyncGlyph, LocalSyncGlyph, CheckGlyph, PinGlyph } from "../icons";
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
  syncBadge,
  syncState,
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
  syncBadge?: "git" | "drive" | "local" | null;
  syncState?: "syncing" | "synced" | null;
  editing: boolean;
  editValue: string;
  onEditChange: (v: string) => void;
  onEditCommit: () => void;
  onEditCancel: () => void;
  onClick: (e: React.MouseEvent) => void;
  onOpen: () => void;
  onMenu: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragOver?: (e: React.DragEvent) => void;
  onDragLeave?: () => void;
  onDrop?: (e: React.DragEvent) => void;
}) {
  const tileRef = useRef<HTMLDivElement>(null);
  // Always the same size regardless of `view`: the <img> is CSS-scaled
  // down for list/listPreview's smaller thumbnail box either way, so
  // there's no quality reason to ask for two sizes -- asking for a
  // different one per view was why switching views took a couple of
  // seconds, forcing a full re-decode+resize+re-encode (and, for images
  // inside a vault, a full re-decrypt too -- vault thumbnails are never
  // disk-cached) of every visible thumbnail purely because the view
  // changed, not because the folder did.
  const thumb = useThumbnail(entry, fullPath, inVault, 160, tileRef);

  return (
    <div
      ref={tileRef}
      className={`entry ${view} ${compact ? "compact" : ""} ${selected ? "selected" : ""} ${
        isDropTarget ? "drop" : ""
      } ${entry.is_hidden ? "dimmed" : ""} ${cut ? "cut" : ""}`}
      data-name={entry.name}
      draggable={!editing}
      onClick={editing ? undefined : onClick}
      onDoubleClick={editing ? undefined : onOpen}
      onContextMenu={editing ? undefined : onMenu}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <span className="entry-icon">
        {thumb ? (
          <img src={thumb} className="entry-thumb" alt="" draggable={false} />
        ) : (
          <FileIcon entry={entry} tagHex={entry.is_dir ? tagHex : undefined} customIcon={customIcon} />
        )}
        {tagHex && !entry.is_dir && <span className="entry-tag-dot" style={{ background: tagHex }} />}
        {syncBadge && (
          <span
            className={`entry-tag-dot entry-sync-badge ${syncState === "syncing" ? "syncing" : ""} ${
              syncState === "synced" ? "synced" : ""
            }`}
          >
            {syncState === "synced" ? (
              <CheckGlyph size={16} />
            ) : syncBadge === "git" ? (
              <GitBranchGlyph size={20} />
            ) : syncBadge === "drive" ? (
              <CloudSyncGlyph size={20} />
            ) : (
              <LocalSyncGlyph size={20} />
            )}
          </span>
        )}
        {pinned && (
          <span className="entry-pin-badge">
            <PinGlyph size={11} />
          </span>
        )}
      </span>
      {editing ? (
        <input
          autoFocus
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
            if (e.key === "Enter") onEditCommit();
            if (e.key === "Escape") onEditCancel();
          }}
        />
      ) : (
        <span className="entry-name">{displayEntryName(entry, !!hideExtensions)}</span>
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
