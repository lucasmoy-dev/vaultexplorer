import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Entry, joinPath, parentPath } from "../../api";
import { FileIcon } from "../../icons";

// A folder chooser that doesn't go through the OS dialog.
//
// `@tauri-apps/plugin-dialog`'s `open({ directory: true })` has no Android
// implementation -- it simply never returns a path there, which is why
// "Move to…" appeared to do nothing on the phone. This browses folders
// with the app's own filesystem commands instead, so it works the same on
// every platform.
//
// What it browses is the caller's: `list` lists one folder, `root` is the
// top of that space ("/" on the desktop, shared storage on the phone, ""
// inside a vault). Nothing above `root` is offered. On the phone it used to
// open in the app's own sandbox dir and let "Up" walk into /data and
// /storage/emulated, which Android won't list -- hence "Up" erroring.
export function FolderPickerSheet({
  startPath,
  root,
  rootLabel,
  list,
  title,
  confirmLabel = "Move here",
  onPick,
  onClose,
}: {
  startPath: string;
  root: string;
  rootLabel: string;
  list: (dir: string) => Promise<Entry[]>;
  title: string;
  // The picker is no longer only used for "Move to…" (folder sync picks
  // the second folder of a pair with it), so the confirm button can't
  // keep saying "Move here" for everyone.
  confirmLabel?: string;
  onPick: (path: string) => void;
  onClose: () => void;
}) {
  const [dir, setDir] = useState<string | null>(null);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const seq = useRef(0);
  const crumbsRef = useRef<HTMLDivElement>(null);

  // Moves only once the target folder has actually listed: a folder that
  // can't be read leaves the picker where it was, with the error shown,
  // instead of showing a path whose contents are the previous folder's.
  function navigate(target: string, fallback?: string) {
    const id = ++seq.current;
    setLoading(true);
    list(target)
      .then((items) => {
        if (id !== seq.current) return;
        // `fs_list` hands back raw readdir order (inode order), which is
        // what the main window's own sort settings normally hide -- here
        // it meant the folder list came out in no order at all.
        setEntries(
          items
            .filter((e) => e.is_dir)
            .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }))
        );
        setDir(target);
        setError("");
        setLoading(false);
      })
      .catch((e) => {
        if (id !== seq.current) return;
        setLoading(false);
        if (fallback !== undefined && fallback !== target) {
          navigate(fallback);
          return;
        }
        setError(String(e));
      });
  }

  useEffect(() => {
    navigate(within(startPath, root) ? startPath : root, root);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The breadcrumb is scrolled to its end so the folder you just entered is
  // what's visible: an ellipsis at the end of one long string used to hide
  // exactly the part that changes, so the path looked stuck.
  useLayoutEffect(() => {
    const el = crumbsRef.current;
    if (el) el.scrollLeft = el.scrollWidth;
  }, [dir]);

  const crumbs = dir === null ? [] : breadcrumbs(dir, root, rootLabel);
  const atRoot = dir === null || dir === root;

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card folder-picker-sheet" onMouseDown={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        <div className="folder-picker-path" ref={crumbsRef}>
          {crumbs.map((c, i) => (
            <span key={c.path} className="folder-picker-crumb">
              {i > 0 && <span className="folder-picker-sep">/</span>}
              <button
                className={`btn-plain ${i === crumbs.length - 1 ? "current" : ""}`}
                onClick={() => c.path !== dir && navigate(c.path)}
              >
                {c.label}
              </button>
            </span>
          ))}
        </div>
        <div className="folder-picker-list">
          {!atRoot && dir !== null && (
            <button className="folder-picker-row" onClick={() => navigate(parentOf(dir, root))}>
              <span className="folder-picker-up">↑</span> Up one level
            </button>
          )}
          {dir !== null &&
            entries.map((e) => (
              <button key={e.name} className="folder-picker-row" onClick={() => navigate(joinPath(dir, e.name))}>
                <FileIcon entry={e} />
                <span>{e.name}</span>
              </button>
            ))}
          {dir !== null && entries.length === 0 && !loading && !error && <p className="hint">No folders in here.</p>}
        </div>
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Cancel
          </button>
          {/* Choosing is always the *current* folder, so an empty folder is
              still a valid destination -- which is usually exactly the one
              you just made for this. */}
          <button className="btn-primary" disabled={dir === null} onClick={() => dir !== null && onPick(dir)}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

function within(path: string, root: string): boolean {
  if (root === "" || root === "/") return root === "" ? !path.startsWith("/") : path.startsWith("/");
  return path === root || path.startsWith(root + "/");
}

function parentOf(path: string, root: string): string {
  const p = parentPath(path);
  return within(p, root) ? p : root;
}

export function breadcrumbs(dir: string, root: string, rootLabel: string): { label: string; path: string }[] {
  const out = [{ label: rootLabel, path: root }];
  if (dir === root) return out;
  const rest = root === "" ? dir : dir.slice(root === "/" ? 1 : root.length + 1);
  let acc = root;
  for (const part of rest.split("/").filter(Boolean)) {
    acc = joinPath(acc, part);
    out.push({ label: part, path: acc });
  }
  return out;
}
