import { useEffect, useState } from "react";
import { Entry, api, joinPath, parentPath } from "../../api";
import { FileIcon } from "../../icons";

// A folder chooser that doesn't go through the OS dialog.
//
// `@tauri-apps/plugin-dialog`'s `open({ directory: true })` has no Android
// implementation -- it simply never returns a path there, which is why
// "Move to…" appeared to do nothing on the phone. This browses folders
// with the app's own filesystem commands instead, so it works the same on
// every platform.
export function FolderPickerSheet({
  startPath,
  title,
  onPick,
  onClose,
}: {
  startPath: string;
  title: string;
  onPick: (path: string) => void;
  onClose: () => void;
}) {
  const [dir, setDir] = useState(startPath);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    api
      .fsList(dir, false)
      .then((list) => {
        if (!cancelled) {
          setEntries(list.filter((e) => e.is_dir));
          setError("");
        }
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [dir]);

  const atRoot = dir === "/" || dir === startPath;

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card folder-picker-sheet" onMouseDown={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        <div className="folder-picker-path" title={dir}>
          {dir}
        </div>
        <div className="folder-picker-list">
          {!atRoot && (
            <button className="folder-picker-row" onClick={() => setDir(parentPath(dir) || "/")}>
              <span className="folder-picker-up">↑</span> Up one level
            </button>
          )}
          {entries.map((e) => (
            <button key={e.name} className="folder-picker-row" onClick={() => setDir(joinPath(dir, e.name))}>
              <FileIcon entry={e} />
              <span>{e.name}</span>
            </button>
          ))}
          {entries.length === 0 && !error && <p className="hint">No folders in here.</p>}
        </div>
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Cancel
          </button>
          {/* Choosing is always the *current* folder, so an empty folder is
              still a valid destination -- which is usually exactly the one
              you just made for this. */}
          <button className="btn-primary" onClick={() => onPick(dir)}>
            Move here
          </button>
        </div>
      </div>
    </div>
  );
}
