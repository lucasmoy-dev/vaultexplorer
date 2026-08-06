import { useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { api, LargeFile, LargeFilesEvent, baseName, formatSize } from "../../api";
import { Dropdown } from "../../ContextMenu";

// Cap how many of the streamed candidates the list actually renders --
// `scan_large_files` already keeps a much bigger pool server-side (so a
// genuinely-larger file never gets evicted early), this is purely "how
// many rows is it reasonable to hand the user to review at once".
const DISPLAY_LIMIT = 100;

type Phase = "config" | "scanning" | "results";

// "Free up space" (Favorites sidebar): scans a favorited folder (or all of
// them) for its largest files via `scan_large_files`, which streams a
// running top-N snapshot back as it walks rather than blocking until the
// whole tree is done -- this sheet just renders whatever the latest
// snapshot says, live, and lets the scan be stopped early without losing
// what it's found so far. Selecting rows accumulates a running "size that
// would be freed" total; "Move to Trash" sends each selected file through
// the same reversible `fs_trash` every other delete in this app uses.
export function FreeUpSpaceSheet({
  favPaths,
  home,
  onClose,
  onDeleted,
}: {
  favPaths: string[];
  home: string;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const scopeOptions = useMemo(() => {
    const opts = [{ value: "__all__", label: "All Favorites" }];
    for (const p of favPaths) {
      opts.push({ value: p, label: p === home ? "All My Files" : baseName(p) || p });
    }
    return opts;
  }, [favPaths, home]);

  const [scope, setScope] = useState(scopeOptions[0]?.value ?? "__all__");
  const [phase, setPhase] = useState<Phase>("config");
  const [files, setFiles] = useState<LargeFile[]>([]);
  const [scanned, setScanned] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState("");
  const [deleting, setDeleting] = useState(false);
  const [deletedCount, setDeletedCount] = useState(0);
  const [freedBytes, setFreedBytes] = useState(0);
  const scanChannelId = useRef<number | null>(null);

  function roots(): string[] {
    return scope === "__all__" ? favPaths : [scope];
  }

  function startScan() {
    setError("");
    setFiles([]);
    setScanned(0);
    setSelected(new Set());
    setPhase("scanning");
    const channel = new Channel<LargeFilesEvent>();
    scanChannelId.current = (channel as unknown as { id: number }).id;
    channel.onmessage = (e) => {
      setFiles(e.files.slice(0, DISPLAY_LIMIT));
      setScanned(e.scanned);
      if (e.done) {
        scanChannelId.current = null;
        setPhase("results");
      }
    };
    api.scanLargeFiles(roots(), channel).catch((e) => {
      scanChannelId.current = null;
      setError(String(e));
      setPhase("results");
    });
  }

  function stopScan() {
    if (scanChannelId.current != null) {
      invoke("cancel_operation", { opId: scanChannelId.current }).catch(() => {});
    }
  }

  function toggle(path: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  const selectedBytes = files
    .filter((f) => selected.has(f.path))
    .reduce((sum, f) => sum + f.size, 0);

  async function deleteSelected() {
    const targets = files.filter((f) => selected.has(f.path));
    if (targets.length === 0) return;
    setDeleting(true);
    setError("");
    const removed = new Set<string>();
    let freed = 0;
    let failed = 0;
    for (const f of targets) {
      try {
        await api.fsTrash(f.path);
        removed.add(f.path);
        freed += f.size;
      } catch {
        failed++;
      }
    }
    setFiles((prev) => prev.filter((f) => !removed.has(f.path)));
    setSelected((prev) => {
      const next = new Set(prev);
      removed.forEach((p) => next.delete(p));
      return next;
    });
    if (removed.size > 0) {
      setDeletedCount((c) => c + removed.size);
      setFreedBytes((b) => b + freed);
      onDeleted();
    }
    if (failed > 0) {
      setError(`${failed} file${failed === 1 ? "" : "s"} couldn't be moved to trash.`);
    }
    setDeleting(false);
  }

  function close() {
    if (phase === "scanning") stopScan();
    onClose();
  }

  return (
    <div className="sheet-overlay" onMouseDown={close}>
      <div className="sheet-card free-space-sheet" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Free Up Space</h3>
        {phase === "config" ? (
          <>
            <p className="hint">
              Scans a folder for its largest files so you can review them and send the ones you
              don't need to the trash.
            </p>
            <label className="field-label">Scan</label>
            <Dropdown value={scope} options={scopeOptions} onChange={setScope} />
            <div className="sheet-actions" style={{ marginTop: 16 }}>
              <button className="btn-plain" onClick={onClose}>
                Cancel
              </button>
              <button className="btn-primary" onClick={startScan}>
                Scan
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="free-space-status">
              {phase === "scanning" && <span className="reorganize-spinner" aria-label="Scanning" />}
              <span>
                {phase === "scanning" ? "Scanning… " : "Scan finished — "}
                {scanned.toLocaleString()} file{scanned === 1 ? "" : "s"} checked
              </span>
            </div>
            {files.length === 0 ? (
              <p className="hint">
                {phase === "scanning" ? "Looking for large files…" : "No files found."}
              </p>
            ) : (
              <div className="free-space-list">
                {files.map((f) => (
                  <label key={f.path} className="checkbox-row free-space-row">
                    <input
                      type="checkbox"
                      checked={selected.has(f.path)}
                      onChange={() => toggle(f.path)}
                    />
                    <span className="free-space-info">
                      <span className="free-space-name">{f.name}</span>
                      <span className="free-space-path">{f.path}</span>
                    </span>
                    <span className="free-space-size">{formatSize(f.size)}</span>
                  </label>
                ))}
              </div>
            )}
            {error && <p className="error">{error}</p>}
            {deletedCount > 0 && (
              <p className="hint">
                Freed {formatSize(freedBytes)} so far ({deletedCount} file
                {deletedCount === 1 ? "" : "s"} moved to trash).
              </p>
            )}
            <div className="free-space-summary">
              <span>
                {selected.size > 0
                  ? `${selected.size} selected — ${formatSize(selectedBytes)} would be freed`
                  : "Select files to free up space"}
              </span>
              {files.length > 0 && (
                <button
                  className="btn-plain small"
                  onClick={() =>
                    setSelected(selected.size === files.length ? new Set() : new Set(files.map((f) => f.path)))
                  }
                >
                  {selected.size === files.length ? "Select None" : "Select All"}
                </button>
              )}
            </div>
            <div className="sheet-actions">
              {phase === "scanning" && (
                <button className="btn-plain" onClick={stopScan}>
                  Stop Scanning
                </button>
              )}
              <button
                className="btn-primary danger"
                disabled={selected.size === 0 || deleting}
                onClick={deleteSelected}
              >
                {deleting ? "Moving to Trash…" : "Move to Trash"}
              </button>
              <button className="btn-plain" onClick={close}>
                {phase === "results" ? "Done" : "Close"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
