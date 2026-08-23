import { useEffect, useState } from "react";
import { api, Entry, formatSize, formatDate, joinPath } from "../../api";
import { FileIcon } from "../../icons";
import { Loc } from "../../types";
import { kindLabel } from "../../entryHelpers";

export function GetInfoSheet({
  entry,
  fullPath,
  root,
  kind,
  customIcon,
  onChangeIcon,
  onClose,
  opensInEditor,
  onSetOpensInEditor,
}: {
  entry: Entry;
  fullPath: string;
  root?: string;
  kind: Loc["kind"];
  customIcon?: string;
  onChangeIcon?: () => void;
  onClose: () => void;
  // Whether this file's *format* is set to open in the built-in text editor
  // (see textEditorExts in App.tsx). `null` when the question doesn't apply:
  // a folder, an extensionless file, or a format we know is binary.
  opensInEditor: boolean | null;
  onSetOpensInEditor: (on: boolean) => void;
}) {
  const [realSize, setRealSize] = useState<number | null>(null);
  useEffect(() => {
    if (!entry.is_dir) return;
    setRealSize(null);
    const call = kind === "vault" ? api.dirSize(fullPath) : api.fsDirSize(fullPath);
    call.then(setRealSize).catch(() => {});
  }, [entry.is_dir, fullPath, kind]);

  const [readonly, setReadonly] = useState<boolean | null>(null);
  useEffect(() => {
    if (kind !== "fs") return;
    api.fsIsReadonly(fullPath).then(setReadonly).catch(() => setReadonly(null));
  }, [fullPath, kind]);

  const [fileMeta, setFileMeta] = useState<[string, string][]>([]);
  useEffect(() => {
    if (entry.is_dir) return;
    const call = kind === "vault" ? api.vaultFileInfo(fullPath) : api.fsFileInfo(fullPath);
    call.then(setFileMeta).catch(() => setFileMeta([]));
  }, [entry.is_dir, fullPath, kind]);
  async function toggleReadonly(checked: boolean) {
    try {
      await api.fsSetReadonly(fullPath, checked);
      setReadonly(checked);
    } catch {
      /* ignore */
    }
  }

  const sizeLabel = entry.is_dir
    ? realSize === null
      ? "Calculating…"
      : formatSize(realSize)
    : formatSize(entry.size);
  const displayPath = kind === "vault" && root ? joinPath(root, fullPath) : fullPath;

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card info-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="info-icon">
          <FileIcon entry={entry} customIcon={customIcon} />
        </div>
        <h3 className="info-name">{entry.name}</h3>
        {entry.is_dir && kind === "fs" && onChangeIcon && (
          <button className="btn-plain small" onClick={onChangeIcon}>
            Change Icon…
          </button>
        )}
        <div className="info-rows">
          <div className="info-row">
            <span>Type</span>
            <span>{kindLabel(entry)}</span>
          </div>
          <div className="info-row">
            <span>Size</span>
            <span>{sizeLabel}</span>
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
          {kind === "fs" && (
            <div className="info-row">
              <span>Created</span>
              <span>{formatDate(entry.created ?? 0)}</span>
            </div>
          )}
          {kind === "fs" && readonly !== null && (
            <label className="info-row checkbox-row">
              <span>Permissions</span>
              <span>
                <input
                  type="checkbox"
                  checked={readonly}
                  onChange={(e) => toggleReadonly(e.target.checked)}
                />{" "}
                Read-only
              </span>
            </label>
          )}
          {kind === "vault" && (
            <div className="info-row">
              <span>Encrypted</span>
              <span>🔒 Yes (inside the vault)</span>
            </div>
          )}
          {opensInEditor !== null && (
            <label className="info-row checkbox-row">
              <span>Opens with</span>
              <span>
                <input
                  type="checkbox"
                  checked={opensInEditor}
                  onChange={(e) => onSetOpensInEditor(e.target.checked)}
                />{" "}
                Text editor — all .{entry.name.toLowerCase().split(".").pop()} files
              </span>
            </label>
          )}
          {fileMeta.map(([label, value]) => (
            <div className="info-row" key={label}>
              <span>{label}</span>
              <span>{value}</span>
            </div>
          ))}
        </div>
        <div className="sheet-actions">
          <button className="btn-primary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

export function MultiInfoSheet({
  names,
  entries,
  curDir,
  kind,
  onClose,
}: {
  names: string[];
  entries: Entry[];
  curDir: string;
  kind: Loc["kind"];
  onClose: () => void;
}) {
  const [totalSize, setTotalSize] = useState<number | null>(null);
  const items = names.map((n) => entries.find((e) => e.name === n)).filter(Boolean) as Entry[];
  const folderCount = items.filter((e) => e.is_dir).length;
  const fileCount = items.length - folderCount;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      let total = 0;
      for (const entry of items) {
        const path = joinPath(curDir, entry.name);
        total += entry.is_dir
          ? await (kind === "vault" ? api.dirSize(path) : api.fsDirSize(path)).catch(() => 0)
          : entry.size;
      }
      if (!cancelled) setTotalSize(total);
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [names.join("|")]);

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card info-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3 className="info-name">{names.length} items selected</h3>
        <div className="info-rows">
          <div className="info-row">
            <span>Contains</span>
            <span>
              {fileCount} {fileCount === 1 ? "file" : "files"}, {folderCount}{" "}
              {folderCount === 1 ? "folder" : "folders"}
            </span>
          </div>
          <div className="info-row">
            <span>Total Size</span>
            <span>{totalSize === null ? "Calculating…" : formatSize(totalSize)}</span>
          </div>
        </div>
        <div className="sheet-actions">
          <button className="btn-primary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
