import { useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { api, LargeFile, LargeFilesEvent, ProgressEvent, baseName, formatSize } from "../api";
import { Dropdown } from "../ContextMenu";
import { FileIcon, kindOf } from "../icons";
import "./FreeUpSpaceView.css";

// "Free Up Space" -- a full view, not a sheet. Reviewing what to delete is
// a browsing task: you scan a tree, sort through a few hundred candidates,
// compare what each category is costing you, and pick. A modal card with a
// scrolling list inside it was the wrong shape for that (reported as
// exactly that), so this takes the whole content pane, shows where the
// space actually went before asking anything, and keeps its actions
// pinned at the bottom while the list scrolls.
//
// The scan itself is unchanged: `scan_large_files` streams a running top-N
// snapshot as it walks, so results appear immediately and stopping early
// keeps whatever it has found.

const DISPLAY_LIMIT = 200;

type Phase = "config" | "scanning" | "results";

// Buckets are the ones that actually explain a full disk. `kindOf` already
// classifies by extension for the file grid, so the same names show up
// here as in the rest of the app rather than a second vocabulary.
const BUCKETS = [
  // `sample` drives the icon: FileIcon classifies by extension, so a
  // throwaway name gives each bucket the very same icon the file grid
  // uses for that kind -- one iconography, not a second private set.
  { key: "video", label: "Video", hue: 265, sample: "a.mp4" },
  { key: "audio", label: "Audio", hue: 200, sample: "a.mp3" },
  { key: "image", label: "Images", hue: 145, sample: "a.jpg" },
  { key: "archive", label: "Archives", hue: 35, sample: "a.zip" },
  { key: "pdf", label: "Documents", hue: 5, sample: "a.pdf" },
  { key: "other", label: "Other", hue: 220, sample: "a.bin" },
] as const;
type BucketKey = (typeof BUCKETS)[number]["key"];

function bucketOf(file: LargeFile): BucketKey {
  const kind = kindOf({ name: file.name, is_dir: false, size: file.size, mtime: 0 });
  switch (kind) {
    case "video":
    case "audio":
    case "image":
    case "archive":
    case "pdf":
      return kind;
    default:
      return "other";
  }
}

export function FreeUpSpaceView({
  favPaths,
  home,
  onDeleted,
  beginProgress,
}: {
  favPaths: string[];
  home: string;
  onDeleted: () => void;
  beginProgress: (label: string) => Channel<ProgressEvent>;
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
  const [bucketFilter, setBucketFilter] = useState<BucketKey | null>(null);
  const [error, setError] = useState("");
  const [deleting, setDeleting] = useState(false);
  const [deletedCount, setDeletedCount] = useState(0);
  const [freedBytes, setFreedBytes] = useState(0);
  const scanChannelId = useRef<number | null>(null);

  function startScan() {
    setError("");
    setFiles([]);
    setScanned(0);
    setSelected(new Set());
    setBucketFilter(null);
    setPhase("scanning");
    const channel = new Channel<LargeFilesEvent>();
    scanChannelId.current = (channel as unknown as { id: number }).id;
    channel.onmessage = (e) => {
      setFiles(e.files.slice(0, DISPLAY_LIMIT));
      setScanned(e.scanned);
      if (e.done) setPhase("results");
    };
    api
      .scanLargeFiles(scope === "__all__" ? favPaths : [scope], channel)
      .catch((err) => {
        setError(String(err));
        setPhase("results");
      });
  }

  function stopScan() {
    if (scanChannelId.current != null) {
      invoke("cancel_operation", { opId: scanChannelId.current }).catch(() => {});
    }
    setPhase("results");
  }

  const totals = useMemo(() => {
    const byBucket = new Map<BucketKey, { bytes: number; count: number }>();
    let bytes = 0;
    for (const f of files) {
      const b = bucketOf(f);
      const cur = byBucket.get(b) ?? { bytes: 0, count: 0 };
      cur.bytes += f.size;
      cur.count += 1;
      byBucket.set(b, cur);
      bytes += f.size;
    }
    return { byBucket, bytes };
  }, [files]);

  // "Which folder is eating the disk" is the other half of the question
  // the chart answers by type -- same found-files pool, grouped by parent
  // directory, biggest first.
  const topFolders = useMemo(() => {
    const byDir = new Map<string, { bytes: number; count: number }>();
    for (const f of files) {
      const dir = f.path.slice(0, f.path.lastIndexOf("/")) || "/";
      const cur = byDir.get(dir) ?? { bytes: 0, count: 0 };
      cur.bytes += f.size;
      cur.count += 1;
      byDir.set(dir, cur);
    }
    return [...byDir.entries()]
      .map(([dir, stat]) => ({ dir, ...stat }))
      .sort((a, b) => b.bytes - a.bytes)
      .slice(0, 5);
  }, [files]);

  const shown = bucketFilter ? files.filter((f) => bucketOf(f) === bucketFilter) : files;
  const largest = files[0]?.size ?? 1;
  const selectedFiles = files.filter((f) => selected.has(f.path));
  const selectedBytes = selectedFiles.reduce((sum, f) => sum + f.size, 0);

  function toggle(path: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(path) ? next.delete(path) : next.add(path);
      return next;
    });
  }

  async function deleteSelected() {
    if (selectedFiles.length === 0) return;
    setDeleting(true);
    setError("");
    const paths = selectedFiles.map((f) => f.path);
    const freed = selectedBytes;
    try {
      // One batched call with an Actions row, same path the file grid's
      // own delete uses -- sending a few hundred files one at a time is
      // what made this feel like it had hung.
      await api.fsTrashMany(
        paths,
        beginProgress(paths.length === 1 ? `Deleting "${selectedFiles[0].name}"` : `Deleting ${paths.length} files`)
      );
      const removed = new Set(paths);
      setFiles((prev) => prev.filter((f) => !removed.has(f.path)));
      setSelected(new Set());
      setDeletedCount((c) => c + paths.length);
      setFreedBytes((b) => b + freed);
      onDeleted();
    } catch (e) {
      setError(String(e));
    }
    setDeleting(false);
  }

  return (
    <div className="freespace-view">
      <div className="freespace-head">
        <div className="freespace-head-text">
          <h2>Free Up Space</h2>
          <p>
            Finds the largest files under a folder so you can see where the space went and send what
            you don't need to the trash. Nothing is deleted until you choose it.
          </p>
        </div>
        <div className="freespace-head-controls">
          <Dropdown value={scope} options={scopeOptions} onChange={setScope} />
          {phase === "scanning" ? (
            <button className="btn-plain" onClick={stopScan}>
              Stop
            </button>
          ) : (
            <button className="btn-primary" onClick={startScan}>
              {phase === "config" ? "Scan" : "Rescan"}
            </button>
          )}
        </div>
      </div>

      {phase !== "config" && (
        <div className="freespace-summary-cards">
          <div className="freespace-card">
            <span className="freespace-card-value">{formatSize(totals.bytes)}</span>
            <span className="freespace-card-label">in the {files.length} largest files</span>
          </div>
          <div className="freespace-card">
            <span className="freespace-card-value">{scanned.toLocaleString()}</span>
            <span className="freespace-card-label">
              files checked{phase === "scanning" ? " so far" : ""}
            </span>
          </div>
          <div className="freespace-card accent">
            <span className="freespace-card-value">{formatSize(selectedBytes)}</span>
            <span className="freespace-card-label">
              selected to free ({selected.size} file{selected.size === 1 ? "" : "s"})
            </span>
          </div>
          {deletedCount > 0 && (
            <div className="freespace-card done">
              <span className="freespace-card-value">{formatSize(freedBytes)}</span>
              <span className="freespace-card-label">freed this session</span>
            </div>
          )}
        </div>
      )}

      {phase !== "config" && totals.bytes > 0 && (
        <>
          {/* A single stacked bar rather than a pie: the question here is
              "which category is eating the disk", which is a part-to-whole
              comparison people read far more accurately off one bar than
              off angles. Clicking a segment filters the list to it. */}
          <div className="freespace-bar" role="img" aria-label="Space by file type">
            {BUCKETS.map((b) => {
              const stat = totals.byBucket.get(b.key);
              if (!stat || stat.bytes === 0) return null;
              const pct = (stat.bytes / totals.bytes) * 100;
              return (
                <button
                  key={b.key}
                  className={`freespace-seg${bucketFilter === b.key ? " active" : ""}`}
                  style={{ width: `${pct}%`, background: `hsl(${b.hue} 62% 55%)` }}
                  title={`${b.label} — ${formatSize(stat.bytes)} (${stat.count} files)`}
                  onClick={() => setBucketFilter((f) => (f === b.key ? null : b.key))}
                  aria-label={`${b.label}, ${formatSize(stat.bytes)}`}
                />
              );
            })}
          </div>
          <div className="freespace-legend">
            {BUCKETS.map((b) => {
              const stat = totals.byBucket.get(b.key);
              if (!stat || stat.bytes === 0) return null;
              const share = Math.round((stat.bytes / totals.bytes) * 100);
              return (
                <button
                  key={b.key}
                  className={`freespace-kind${bucketFilter === b.key ? " active" : ""}`}
                  onClick={() => setBucketFilter((f) => (f === b.key ? null : b.key))}
                  style={{ "--kind": `hsl(${b.hue} 62% 55%)` } as React.CSSProperties}
                >
                  <span className="freespace-kind-icon">
                    <FileIcon entry={{ name: b.sample, is_dir: false, size: 0, mtime: 0 }} />
                  </span>
                  <span className="freespace-kind-text">
                    <span className="freespace-kind-label">{b.label}</span>
                    <span className="freespace-kind-sub">
                      {stat.count} file{stat.count === 1 ? "" : "s"} · {share}%
                    </span>
                  </span>
                  <span className="freespace-kind-size">{formatSize(stat.bytes)}</span>
                </button>
              );
            })}
            {bucketFilter && (
              <button className="freespace-legend-clear" onClick={() => setBucketFilter(null)}>
                Show all
              </button>
            )}
          </div>

          {topFolders.length > 1 && (
            <div className="freespace-folders">
              <h3>Where it's going</h3>
              {topFolders.map((f) => (
                <div className="freespace-folder" key={f.dir}>
                  <span className="freespace-folder-name" title={f.dir}>
                    {f.dir.split("/").slice(-2).join("/")}
                  </span>
                  <span className="freespace-folder-bar">
                    <span style={{ width: `${(f.bytes / (topFolders[0]?.bytes || 1)) * 100}%` }} />
                  </span>
                  <span className="freespace-folder-size">{formatSize(f.bytes)}</span>
                </div>
              ))}
            </div>
          )}
        </>
      )}

      {phase === "config" ? (
        <div className="freespace-empty">
          <p>Pick what to scan, then press Scan. Results appear as they're found.</p>
        </div>
      ) : (
        <div className="freespace-list">
          {shown.length === 0 && (
            <p className="freespace-empty-line">
              {phase === "scanning" ? "Looking for large files…" : "Nothing found here."}
            </p>
          )}
          {shown.map((f) => {
            const bucket = BUCKETS.find((b) => b.key === bucketOf(f))!;
            return (
              <label key={f.path} className={`freespace-row${selected.has(f.path) ? " selected" : ""}`}>
                <input type="checkbox" checked={selected.has(f.path)} onChange={() => toggle(f.path)} />
                {/* The bar behind each row is that file's size relative to
                    the biggest one found, so a list sorted by size still
                    shows *how much* bigger the top entries are. */}
                <span className="freespace-row-bar" style={{ width: `${(f.size / largest) * 100}%`, background: `hsl(${bucket.hue} 62% 55% / 0.16)` }} />
                <span className="freespace-row-main">
                  <span className="freespace-row-name">{f.name}</span>
                  <span className="freespace-row-path">{f.path}</span>
                </span>
                <span className="freespace-row-size">{formatSize(f.size)}</span>
              </label>
            );
          })}
        </div>
      )}

      {error && <p className="error freespace-error">{error}</p>}

      {phase !== "config" && (
        <div className="freespace-actions">
          <button
            className="btn-plain small"
            onClick={() =>
              setSelected(selected.size === shown.length ? new Set() : new Set(shown.map((f) => f.path)))
            }
            disabled={shown.length === 0}
          >
            {selected.size === shown.length && shown.length > 0 ? "Select none" : "Select all shown"}
          </button>
          <span className="freespace-actions-summary">
            {selected.size > 0
              ? `${selected.size} selected — ${formatSize(selectedBytes)} would be freed`
              : "Select files to free up space"}
          </span>
          <button
            className="btn-primary danger"
            disabled={selected.size === 0 || deleting}
            onClick={deleteSelected}
          >
            {deleting ? "Moving to Trash…" : "Move to Trash"}
          </button>
        </div>
      )}
    </div>
  );
}
