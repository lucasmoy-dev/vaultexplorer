import { useEffect, useRef, useState } from "react";
import { Entry, api, joinPath, parentPath, formatSize, formatDate } from "../api";
import { Loc } from "../types";
import { FileIcon, CopyGlyph, CheckGlyph, kindOf } from "../icons";
import { kindLabel } from "../entryHelpers";
import { useThumbnail } from "../hooks/useThumbnail";

// A small copy-to-clipboard button (checkmark swap on success, same
// pattern as the toolbar breadcrumb's own copy-path button) -- used next
// to both the file name and the Location value below, since "select the
// text to copy it" wasn't possible here until `user-select: text` was
// added (see the .preview-name / .info-path CSS), and a button is faster
// regardless.
function CopyButton({ text, title }: { text: string; title: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className={`preview-copy-btn ${copied ? "copied" : ""}`}
      title={title}
      onClick={async (e) => {
        e.stopPropagation();
        try {
          await navigator.clipboard.writeText(text);
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        } catch {
          /* ignore */
        }
      }}
    >
      {copied ? <CheckGlyph size={11} /> : <CopyGlyph size={11} />}
    </button>
  );
}

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

// Video and audio play in place instead of showing a dead poster frame:
// click the play badge and it plays right here, with the browser's own
// controls for pause/seek/volume once started.
//
// Nothing is created until that click -- no <video> element, and for a vault
// file no FUSE path resolution either, so merely *selecting* a 2GB video
// costs nothing. And since the whole preview is keyed by path, selecting
// another file tears this down: coming back gives a fresh element at 0:00
// rather than resuming mid-stream, which is what "start from the beginning,
// only if I press play" means. Playback also stops on its own when you
// select something else, for the same reason.
function MediaPreview({
  entry,
  fullPath,
  inVault,
  poster,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  poster: string | null;
}) {
  const isVideo = kindOf(entry) === "video";
  const [src, setSrc] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState("");
  const mediaRef = useRef<HTMLVideoElement & HTMLAudioElement>(null);

  async function toggle() {
    if (src) {
      const el = mediaRef.current;
      if (!el) return;
      if (el.paused) void el.play();
      else el.pause();
      return;
    }
    try {
      // A vault file's real bytes only exist behind its FUSE mount; the
      // vault-relative path means nothing to the webview.
      const abs = inVault ? await api.openPath(fullPath) : fullPath;
      // Same reason MediaViewer does this: a media element can't load an
      // asset:// URL under WebKitGTK (see mediaserver.rs).
      setSrc(await api.mediaUrl(abs));
    } catch (e) {
      setError(String(e));
    }
  }

  // Autoplay once the source lands (the click that set it *was* the play
  // request), and keep `playing` in sync with the element rather than
  // assuming -- pausing via the native controls has to swap the badge back.
  const showBadge = !src || !playing;
  return (
    <div className={`preview-media ${isVideo ? "video" : "audio"}`}>
      {src ? (
        isVideo ? (
          <video
            ref={mediaRef}
            src={src}
            poster={poster ?? undefined}
            autoPlay
            controls
            onPlay={() => setPlaying(true)}
            onPause={() => setPlaying(false)}
            onEnded={() => setPlaying(false)}
            onError={() => setError("Can't play this file")}
          />
        ) : (
          <>
            {poster ? <img src={poster} alt="" draggable={false} /> : <FileIcon entry={entry} />}
            <audio
              ref={mediaRef}
              src={src}
              autoPlay
              controls
              onPlay={() => setPlaying(true)}
              onPause={() => setPlaying(false)}
              onEnded={() => setPlaying(false)}
              onError={() => setError("Can't play this file")}
            />
          </>
        )
      ) : poster ? (
        <img src={poster} alt="" draggable={false} />
      ) : (
        <div className="preview-icon">
          <FileIcon entry={entry} />
        </div>
      )}
      {showBadge && (
        <button
          type="button"
          className="preview-play-badge"
          aria-label={src ? "Play" : `Play ${entry.name}`}
          onClick={toggle}
        >
          <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true">
            <path d="M8 5.5v13l11-6.5z" fill="currentColor" />
          </svg>
        </button>
      )}
      {error && <p className="preview-media-error">{error}</p>}
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
  onEdit,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  root?: string;
  onRename?: (newName: string) => void;
  // Given only where an editor can actually open (List with Preview's pane --
  // column view has no editor slot), and only for formats where editing as
  // text makes sense. Switches this format to the text editor from now on.
  onEdit?: () => void;
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
  // Video/audio get the inline player below instead of the zoom/pan image
  // viewer -- scroll-to-zoom over a playing video (and swallowing the wheel
  // event to do it) isn't a gesture anyone wants there.
  const isMedia = kindOf(entry) === "video" || kindOf(entry) === "audio";
  const thumbRef = useRef<HTMLDivElement>(null);
  // The image can never be dragged fully out of view: at a given zoom the
  // rendered box overhangs the container by (zoom - 1) * size, so half of
  // that is the farthest either edge can travel from center. Re-clamping
  // on zoom-out is what pulls a panned image back toward center as it
  // shrinks, until at 1x it's exactly centered again.
  function clampPan(p: { x: number; y: number }, z: number) {
    const el = thumbRef.current;
    if (!el) return p;
    const maxX = (el.clientWidth * (z - 1)) / 2;
    const maxY = (el.clientHeight * (z - 1)) / 2;
    return {
      x: Math.min(maxX, Math.max(-maxX, p.x)),
      y: Math.min(maxY, Math.max(-maxY, p.y)),
    };
  }
  useEffect(() => {
    const el = thumbRef.current;
    if (!el || isMedia) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setZoom((z) => {
        // Multiplicative steps feel uniform across the whole range
        // (additive ones crawl when zoomed out and jump when zoomed in).
        const next = Math.min(8, Math.max(1, z * Math.exp(-e.deltaY * 0.002)));
        // Zoom toward the cursor: keep the image point under the pointer
        // stationary by scaling its offset from center along with it.
        const rect = el.getBoundingClientRect();
        const cx = e.clientX - rect.left - rect.width / 2;
        const cy = e.clientY - rect.top - rect.height / 2;
        setPan((p) =>
          clampPan({ x: cx - ((cx - p.x) * next) / z, y: cy - ((cy - p.y) * next) / z }, next)
        );
        return next;
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [isMedia]);

  // Requesting a sharper render as the user zooms in, rather than just
  // CSS-stretching the same 480px thumbnail. Snapped to three fixed
  // buckets instead of a continuous size: a zoom session then re-decodes
  // the image at most twice, not once per settled scroll position.
  const [thumbSize, setThumbSize] = useState(480);
  useEffect(() => {
    const bucket = zoom > 3 ? 1920 : zoom > 1.2 ? 960 : 480;
    if (bucket <= thumbSize) return; // never downgrade mid-session; reset comes with the file change
    const t = setTimeout(() => setThumbSize(bucket), 250);
    return () => clearTimeout(t);
  }, [zoom, thumbSize]);
  useEffect(() => setThumbSize(480), [fullPath]);
  const thumb = useThumbnail(entry, fullPath, inVault, thumbSize);

  const dragRef = useRef<{ startX: number; startY: number; panX: number; panY: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  useEffect(() => {
    if (!dragging) return;
    function onMove(e: MouseEvent) {
      if (!dragRef.current) return;
      setPan(
        clampPan(
          {
            x: dragRef.current.panX + (e.clientX - dragRef.current.startX),
            y: dragRef.current.panY + (e.clientY - dragRef.current.startY),
          },
          zoom
        )
      );
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
  }, [dragging, zoom]);
  function onThumbMouseDown(e: React.MouseEvent) {
    if (zoom <= 1 || (e.button !== 0 && e.button !== 1)) return;
    e.preventDefault();
    dragRef.current = { startX: e.clientX, startY: e.clientY, panX: pan.x, panY: pan.y };
    setDragging(true);
  }
  // Double-click: quick in/out toggle, the gesture every image viewer has.
  function onThumbDoubleClick() {
    if (isMedia) return;
    if (zoom > 1) {
      setZoom(1);
      setPan({ x: 0, y: 0 });
    } else {
      setZoom(2);
    }
  }

  const displayPath = kind === "vault" && root ? joinPath(root, fullPath) : fullPath;
  // Just the containing folder -- the file's own name is already shown
  // right above (EditableFileName), repeating it here again was redundant.
  const displayDir = parentPath(displayPath);

  return (
    <div className="column preview-column">
      <div
        className="preview-thumb"
        ref={thumbRef}
        onMouseDown={onThumbMouseDown}
        onDoubleClick={onThumbDoubleClick}
        style={{ cursor: zoom > 1 ? (dragging ? "grabbing" : "grab") : "default" }}
      >
        {isMedia ? (
          <MediaPreview entry={entry} fullPath={fullPath} inVault={inVault} poster={thumb} />
        ) : thumb ? (
          <img
            src={thumb}
            alt=""
            draggable={false}
            style={{
              transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
              // The eased transform is what makes wheel-zoom feel smooth,
              // but during a drag it lags the cursor -- pan must be 1:1.
              transition: dragging ? "none" : undefined,
              willChange: zoom > 1 ? "transform" : undefined,
            }}
          />
        ) : (
          <div className="preview-icon">
            <FileIcon entry={entry} />
          </div>
        )}
      </div>
      <div className="preview-name-row">
        <EditableFileName name={entry.name} onRename={onRename} />
        <CopyButton text={entry.name} title="Copy name" />
      </div>
      {onEdit && (
        <button
          className="btn-plain small preview-edit-btn"
          title={`Open .${entry.name.toLowerCase().split(".").pop()} files in the text editor from now on (change it in Get Info)`}
          onClick={onEdit}
        >
          Edit as Text
        </button>
      )}
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
          <span className="info-path-group">
            <span className="info-path" title={displayDir}>
              {displayDir}
            </span>
            <CopyButton text={displayDir} title="Copy location" />
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
