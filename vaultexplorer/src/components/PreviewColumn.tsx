import { useEffect, useRef, useState } from "react";
import { Entry, api, joinPath, formatSize, formatDate } from "../api";
import { Loc } from "../types";
import { FileIcon } from "../icons";
import { kindLabel } from "../entryHelpers";
import { useThumbnail } from "../hooks/useThumbnail";

// A file's own name, shown wherever a preview pane needs it -- click to
// rename in place (extension hidden while editing, same convention as the
// grid's inline rename). Read-only when `onRename` isn't given (e.g. the
// column-view preview pane, which doesn't offer renaming from here).
export function EditableFileName({ name, onRename }: { name: string; onRename?: (newName: string) => void }) {
  const [editing, setEditing] = useState(false);
  const dot = name.lastIndexOf(".");
  const base = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  const [value, setValue] = useState(base);

  function commit() {
    setEditing(false);
    const trimmed = value.trim();
    if (trimmed && trimmed !== base) onRename?.(trimmed + ext);
    else setValue(base);
  }

  if (editing) {
    return (
      <input
        autoFocus
        className="preview-name-edit"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onFocus={(e) => e.currentTarget.select()}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") {
            setValue(base);
            setEditing(false);
          }
        }}
      />
    );
  }
  return (
    <div
      className={`preview-name ${onRename ? "editable" : ""}`}
      title={name}
      onClick={() => {
        if (!onRename) return;
        setValue(base);
        setEditing(true);
      }}
    >
      {name}
    </div>
  );
}

// The rightmost column shown when a file (not a folder) is selected: a
// bigger preview plus the handful of Get-Info facts, so column view can
// answer "what is this" without opening a separate sheet.
export function PreviewColumn({
  entry,
  fullPath,
  inVault,
  root,
  onRename,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  root?: string;
  onRename?: (newName: string) => void;
}) {
  const kind: Loc["kind"] = inVault ? "vault" : "fs";
  const [fileMeta, setFileMeta] = useState<[string, string][]>([]);
  useEffect(() => {
    setFileMeta([]);
    const call = kind === "vault" ? api.vaultFileInfo(fullPath) : api.fsFileInfo(fullPath);
    call.then(setFileMeta).catch(() => setFileMeta([]));
  }, [fullPath, kind]);

  // Plain scroll (no modifier needed) zooms the previewed image in place;
  // once zoomed, holding a regular or middle click pans it around. Same
  // "native, non-passive wheel listener" reasoning as the icon-view zoom
  // effect -- React's onWheel can't reliably preventDefault since it's
  // registered passive by default.
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  useEffect(() => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
  }, [fullPath]);
  const thumbRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = thumbRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setZoom((z) => Math.min(8, Math.max(1, z - e.deltaY * 0.003)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // Requesting a sharper render as the user zooms in, rather than just
  // CSS-stretching the same 480px thumbnail -- debounced so a mid-scroll
  // zoom gesture doesn't fire a fresh request on every tick, only once it
  // settles.
  const [thumbSize, setThumbSize] = useState(480);
  useEffect(() => {
    const t = setTimeout(() => setThumbSize(Math.min(2400, Math.round(480 * Math.max(1, zoom)))), 250);
    return () => clearTimeout(t);
  }, [zoom]);
  const thumb = useThumbnail(entry, fullPath, inVault, thumbSize);

  const dragRef = useRef<{ startX: number; startY: number; panX: number; panY: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  useEffect(() => {
    if (!dragging) return;
    function onMove(e: MouseEvent) {
      if (!dragRef.current) return;
      setPan({
        x: dragRef.current.panX + (e.clientX - dragRef.current.startX),
        y: dragRef.current.panY + (e.clientY - dragRef.current.startY),
      });
    }
    function onUp() {
      dragRef.current = null;
      setDragging(false);
    }
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragging]);
  function onThumbMouseDown(e: React.MouseEvent) {
    if (zoom <= 1 || (e.button !== 0 && e.button !== 1)) return;
    e.preventDefault();
    dragRef.current = { startX: e.clientX, startY: e.clientY, panX: pan.x, panY: pan.y };
    setDragging(true);
  }

  const displayPath = kind === "vault" && root ? joinPath(root, fullPath) : fullPath;

  return (
    <div className="column preview-column">
      <div
        className="preview-thumb"
        ref={thumbRef}
        onMouseDown={onThumbMouseDown}
        style={{ cursor: zoom > 1 ? (dragging ? "grabbing" : "grab") : "default" }}
      >
        {thumb ? (
          <img
            src={thumb}
            alt=""
            draggable={false}
            style={{ transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})` }}
          />
        ) : (
          <div className="preview-icon">
            <FileIcon entry={entry} />
          </div>
        )}
      </div>
      <EditableFileName name={entry.name} onRename={onRename} />
      <div className="info-rows">
        <div className="info-row">
          <span>Type</span>
          <span>{kindLabel(entry)}</span>
        </div>
        <div className="info-row">
          <span>Size</span>
          <span>{formatSize(entry.size)}</span>
        </div>
        <div className="info-row">
          <span>Location</span>
          <span className="info-path" title={displayPath}>
            {displayPath}
          </span>
        </div>
        <div className="info-row">
          <span>Modified</span>
          <span>{formatDate(entry.mtime)}</span>
        </div>
        {fileMeta.map(([label, value]) => (
          <div className="info-row" key={label}>
            <span>{label}</span>
            <span>{value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
