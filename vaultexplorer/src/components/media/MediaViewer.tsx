import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api } from "../../api";
import { ChevronLeft, ChevronRight, CloseGlyph, PencilGlyph, RotateGlyph, TrashGlyph } from "../../icons";
import { ContextMenu, MenuState } from "../../ContextMenu";
import { ImageStage } from "./ImageStage";
import { VideoStage } from "./VideoStage";
import { AudioStage } from "./AudioStage";
import { ImageEditor } from "./ImageEditor";
import "./MediaViewer.css";

// MediaViewer -- fullscreen photo/video/audio gallery shell ("swipe left or
// right" viewer). Owns navigation (index, keyboard, swipe, chevrons),
// src-resolution + caching, and delete; it renders exactly one of the three
// per-kind "stage" components below for whatever the current gallery item
// is, and layers ImageEditor on top of itself when editing an image.
//
// ---- Prop contracts this file was written against for its three sibling
// stage components + the editor overlay. All four live in this same
// src/components/media/ directory and are owned by other agents working in
// parallel; every one of them had already landed on disk by the time this
// file was finished, so all four contracts below are read directly from
// their real source (not guessed) -- listed here anyway so this file's
// call sites are self-documenting without having to cross-reference:
//
// ImageStage (src/components/media/ImageStage.tsx):
//   interface ImageStageProps {
//     src: string;             // resolved, directly displayable URL
//     fullPath: string;        // original (unresolved) entry path -- real
//                               // fs path if !inVault, vault-relative if
//                               // inVault -- used to persist a rotate
//     inVault: boolean;
//     rotateSignal?: number;   // bump to request a 90deg CW rotate+save;
//                               // initial value is not itself a request
//     onRotated?: () => void;  // fired after a rotate is persisted
//     onError?: (message: string) => void;
//   }
//   No `name` prop -- ImageStage renders only the image surface itself;
//   the filename lives in MediaViewer's own toolbar.
//
// VideoStage (src/components/media/VideoStage.tsx):
//   interface VideoStageProps {
//     src: string;
//     name: string; // used as the <video>'s accessible label
//   }
//
// AudioStage (src/components/media/AudioStage.tsx):
//   interface AudioStageProps {
//     src: string;
//     name: string;
//     hasNext: boolean;
//     hasPrev: boolean;
//     onNext: () => void;
//     onPrev: () => void;
//   }
//   AudioStage owns only playback state; MediaViewer owns the gallery
//   cursor and hands it a new src/name plus fresh hasNext/hasPrev on
//   navigation (including auto-advance when a track ends, via onNext).
//
// ImageEditor (src/components/media/ImageEditor.tsx):
//   interface ImageEditorProps {
//     src: string;          // same resolved src ImageStage gets
//     fullPath: string;     // real fs path or vault-relative path to save to
//     inVault: boolean;
//     onClose: () => void;  // cancel -- any edit only in its working
//                            // canvas (not yet Saved) is discarded
//     onSaved: () => void;  // fired once ImageEditor has *already* written
//                            // the edited bytes back to disk itself. Unlike
//                            // the original task note ("onSave just closes
//                            // the editor for now"), the real ImageEditor
//                            // does its own persisting -- so beyond
//                            // closing, MediaViewer also cache-busts this
//                            // item's resolved src here (see
//                            // handleImageSaved below) so the now-stale
//                            // cached URL doesn't keep showing pre-edit
//                            // pixels.
//   }

export interface GalleryEntry {
  name: string;
  fullPath: string;
  kind: "image" | "video" | "audio";
  inVault: boolean;
}

export interface MediaViewerProps {
  gallery: GalleryEntry[];
  startIndex: number;
  onClose: () => void;
  onDeleted: (fullPath: string) => void;
  // Fired after a rotate or an ImageEditor save actually lands new bytes
  // on disk -- MediaViewer's own resolvedSrc cache-busts itself fine (the
  // open photo shows the edit immediately), but the file *grid* behind it
  // has no way to know its thumbnail for this path just went stale until
  // something tells it to re-check. Reported directly: edits saved but
  // the background grid's thumbnail never updated until a manual refresh.
  onFileChanged: (fullPath: string) => void;
  mobile: boolean;
}

function clampIndex(i: number, len: number): number {
  if (len <= 0) return 0;
  return Math.min(Math.max(i, 0), len - 1);
}

// How long an armed (but not yet confirmed) delete stays armed before
// silently disarming, so a stray later click on the same button doesn't
// delete something the user just wanted to look at again.
const DELETE_CONFIRM_WINDOW_MS = 4000;
// Minimum horizontal swipe distance (px) before it counts as prev/next,
// so an ordinary tap or vertical scroll gesture doesn't navigate.
const SWIPE_THRESHOLD_PX = 50;

export function MediaViewer({
  gallery: initialGallery,
  startIndex,
  onClose,
  onDeleted,
  onFileChanged,
  mobile,
}: MediaViewerProps): React.JSX.Element {
  const [gallery, setGallery] = useState<GalleryEntry[]>(initialGallery);
  const [index, setIndex] = useState(() => clampIndex(startIndex, initialGallery.length));
  // "left" = just moved to next (incoming pane slides in from the right),
  // "right" = just moved to prev (incoming pane slides in from the left),
  // null = initial mount, plain fade.
  const [direction, setDirection] = useState<"left" | "right" | null>(null);
  const [resolvedSrc, setResolvedSrc] = useState<string | null>(null);
  // The real filesystem path behind `resolvedSrc` -- same value, before
  // `convertFileSrc` turns it into an asset:// URL. Kept separately because
  // Share needs a real path (Android's FileProvider hands a `content://`
  // URI to the OS share sheet from an actual file, not from a URL an app
  // outside this one's sandbox has no way to fetch).
  const [resolvedAbsPath, setResolvedAbsPath] = useState<string | null>(null);
  const [sharing, setSharing] = useState(false);
  const [resolveError, setResolveError] = useState("");
  // Separate from resolveError: a rotate/save failure surfaced by
  // ImageStage's onError shouldn't hide the already-loaded stage the way a
  // path-resolution failure does (see the render guard below).
  const [stageError, setStageError] = useState("");
  const [editingOpen, setEditingOpen] = useState(false);
  const [rotateSignal, setRotateSignal] = useState(0);
  const [deleteArmed, setDeleteArmed] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [overflowMenu, setOverflowMenu] = useState<MenuState>(null);
  // While the current image is zoomed in, a one-finger pan is ImageStage's
  // gesture to interpret, not this shell's swipe-to-navigate -- see
  // ImageStage's `onZoomChange` doc comment for the bug this fixes.
  const [imageZoomed, setImageZoomed] = useState(false);
  // Audio-only playback modes -- `gallery` is already scoped to just the
  // audio siblings in the same folder when a track was opened (see
  // `openMediaViewer` in App.tsx), so that array doubles as the playlist
  // with no separate fetch needed. Shuffle history is real state (not a
  // ref) so `hasPrev` below reacts to it the same way it reacts to `index`.
  const [shuffle, setShuffle] = useState(false);
  const [repeatMode, setRepeatMode] = useState<"off" | "all" | "one">("off");
  const [shuffleHistory, setShuffleHistory] = useState<number[]>([]);
  // Bumped to tell AudioStage "replay the current track from 0", distinct
  // from a `src` change (which AudioStage treats as "this is a different
  // track" and would otherwise be indistinguishable from repeat-one).
  const [repeatOneSignal, setRepeatOneSignal] = useState(0);

  const srcCache = useRef(new Map<string, string>());
  const absPathCache = useRef(new Map<string, string>());
  const galleryLenRef = useRef(gallery.length);
  const disarmTimer = useRef<number | null>(null);
  const touchStartX = useRef<number | null>(null);
  const touchStartY = useRef<number | null>(null);

  const current: GalleryEntry | null = gallery[index] ?? null;

  useEffect(() => {
    galleryLenRef.current = gallery.length;
  }, [gallery.length]);

  // Resolve the current item's displayable src (real fs path -> convertFileSrc
  // directly; vault-internal path -> api.openPath() first, same pattern as
  // PreviewColumn.tsx's toggle()), cached per fullPath so flipping back and
  // forth between already-seen items is instant and doesn't re-resolve.
  useEffect(() => {
    if (!current) {
      setResolvedSrc(null);
      return;
    }
    setResolveError("");
    setStageError("");
    setImageZoomed(false);
    const cached = srcCache.current.get(current.fullPath);
    if (cached) {
      setResolvedSrc(cached);
      setResolvedAbsPath(absPathCache.current.get(current.fullPath) ?? null);
      return;
    }
    let cancelled = false;
    setResolvedSrc(null);
    setResolvedAbsPath(null);
    (async () => {
      try {
        const abs = current.inVault ? await api.openPath(current.fullPath) : current.fullPath;
        const url = convertFileSrc(abs);
        srcCache.current.set(current.fullPath, url);
        absPathCache.current.set(current.fullPath, abs);
        if (!cancelled) {
          setResolvedSrc(url);
          setResolvedAbsPath(abs);
        }
      } catch (e) {
        if (!cancelled) setResolveError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [current]);

  // Navigation helpers use a functional setIndex update + a length ref
  // (rather than closing over `index`/`gallery.length` directly) so the
  // keydown listener below can be attached once, without resubscribing on
  // every index change.
  function randomIndexExcluding(exclude: number, len: number): number {
    if (len <= 1) return exclude;
    let r = Math.floor(Math.random() * len);
    if (r === exclude) r = (r + 1) % len;
    return r;
  }
  function goPrev() {
    if (current?.kind === "audio" && shuffle) {
      setShuffleHistory((h) => {
        if (h.length === 0) return h;
        const prevIdx = h[h.length - 1];
        setDirection("right");
        setIndex(prevIdx);
        return h.slice(0, -1);
      });
      setDeleteArmed(false);
      return;
    }
    setIndex((i) => {
      if (i <= 0) return i;
      setDirection("right");
      return i - 1;
    });
    setDeleteArmed(false);
  }
  function goNext() {
    if (current?.kind === "audio" && shuffle) {
      setIndex((i) => {
        if (galleryLenRef.current <= 1) return i;
        setShuffleHistory((h) => [...h, i]);
        setDirection("left");
        return randomIndexExcluding(i, galleryLenRef.current);
      });
      setDeleteArmed(false);
      return;
    }
    setIndex((i) => {
      if (i >= galleryLenRef.current - 1) {
        if (current?.kind === "audio" && repeatMode === "all" && galleryLenRef.current > 1) {
          setDirection("left");
          return 0;
        }
        return i;
      }
      setDirection("left");
      return i + 1;
    });
    setDeleteArmed(false);
  }
  // Repeat-one restarts the same track instead of advancing -- exposed
  // separately from goNext so AudioStage's onEnded can special-case it
  // without goNext itself needing to know "did this fire because the
  // track ended" vs "the user tapped skip".
  function handleAudioEnded() {
    if (repeatMode === "one") {
      setRepeatOneSignal((n) => n + 1);
      return;
    }
    if (hasNext) {
      goNext();
    }
  }

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowLeft") goPrev();
      else if (e.key === "ArrowRight") goNext();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onClose]);

  useEffect(() => {
    return () => {
      if (disarmTimer.current !== null) window.clearTimeout(disarmTimer.current);
    };
  }, []);

  function onTouchStart(e: React.TouchEvent) {
    const t = e.touches[0];
    touchStartX.current = t.clientX;
    touchStartY.current = t.clientY;
  }
  function onTouchMove() {
    // No live drag-follow -- swipe is evaluated as a single gesture on
    // release, see onTouchEnd. Kept as a no-op handler so the root overlay
    // still receives touchmove (some browsers want all three handlers
    // present to treat this as an intentional gesture rather than a
    // page-level scroll).
  }
  function onTouchEnd(e: React.TouchEvent) {
    const startX = touchStartX.current;
    const startY = touchStartY.current;
    touchStartX.current = null;
    touchStartY.current = null;
    if (startX === null || startY === null) return;
    if (imageZoomed) return;
    const t = e.changedTouches[0];
    const dx = t.clientX - startX;
    const dy = t.clientY - startY;
    if (Math.abs(dx) < SWIPE_THRESHOLD_PX || Math.abs(dx) < Math.abs(dy)) return;
    if (dx < 0) goNext();
    else goPrev();
  }

  async function handleShare() {
    if (!resolvedAbsPath || sharing) return;
    setSharing(true);
    try {
      await api.androidSharePath(resolvedAbsPath);
    } catch (e) {
      setStageError(String(e));
    } finally {
      setSharing(false);
    }
  }

  function armOrConfirmDelete() {
    if (!current || deleting) return;
    if (!deleteArmed) {
      setDeleteArmed(true);
      if (disarmTimer.current !== null) window.clearTimeout(disarmTimer.current);
      disarmTimer.current = window.setTimeout(() => setDeleteArmed(false), DELETE_CONFIRM_WINDOW_MS);
      return;
    }
    void doDelete();
  }

  async function doDelete() {
    if (!current || deleting) return;
    const target = current;
    setDeleting(true);
    try {
      if (target.inVault) {
        await api.deleteFile(target.fullPath);
      } else {
        await api.fsTrash(target.fullPath);
      }
      srcCache.current.delete(target.fullPath);
      onDeleted(target.fullPath);
      setGallery((prev) => {
        const remaining = prev.filter((g) => g.fullPath !== target.fullPath);
        if (remaining.length === 0) {
          onClose();
        } else {
          setIndex((i) => clampIndex(i, remaining.length));
        }
        return remaining;
      });
    } catch (e) {
      setResolveError(String(e));
    } finally {
      setDeleting(false);
      setDeleteArmed(false);
      if (disarmTimer.current !== null) window.clearTimeout(disarmTimer.current);
    }
  }

  function handleRotate() {
    setStageError("");
    setRotateSignal((n) => n + 1);
  }

  // Appends/refreshes a `v=` query param so the browser treats this as a
  // different URL and re-fetches the file's bytes instead of serving a
  // stale cached copy -- same trick ImageStage already uses internally for
  // its own displaySrc after a rotate; this just applies it to *our* cache
  // too, so navigating away and back to this item later doesn't show
  // pre-edit pixels.
  function cacheBust(url: string): string {
    return `${url}${url.includes("?") ? "&" : "?"}v=${Date.now()}`;
  }

  // ImageStage already re-renders itself with the freshly-rotated bytes
  // (via its own internal cache-busted displaySrc); this only needs to
  // invalidate MediaViewer's own srcCache entry so a later re-visit to this
  // same gallery item doesn't hand ImageStage the pre-rotation URL again.
  function handleRotated() {
    if (!current || !resolvedSrc) return;
    srcCache.current.set(current.fullPath, cacheBust(resolvedSrc));
    onFileChanged(current.fullPath);
  }

  // ImageEditor persists its own edit to disk before calling this -- unlike
  // a rotate, there's no already-updated on-screen <img> to fall back on,
  // so this also swaps in a cache-busted resolvedSrc immediately so
  // ImageStage picks up the new pixels as soon as the editor closes.
  function handleImageSaved() {
    if (current && resolvedSrc) {
      const busted = cacheBust(resolvedSrc);
      srcCache.current.set(current.fullPath, busted);
      setResolvedSrc(busted);
      onFileChanged(current.fullPath);
    }
    setEditingOpen(false);
  }

  const isAudio = current?.kind === "audio";
  const hasPrev = isAudio && shuffle ? shuffleHistory.length > 0 : index > 0;
  const hasNext =
    isAudio && shuffle
      ? gallery.length > 1
      : index < gallery.length - 1 || (isAudio && repeatMode === "all" && gallery.length > 1);
  const isImage = current?.kind === "image";

  return (
    <div
      className="media-viewer-overlay"
      onMouseDown={onClose}
      onTouchStart={onTouchStart}
      onTouchMove={onTouchMove}
      onTouchEnd={onTouchEnd}
    >
      <div className="media-viewer-card" onMouseDown={(e) => e.stopPropagation()}>
        {/* Bottom-anchored is a mobile-only convention (thumb reach) --
            reported directly that it read as wrong on desktop, where the
            title belongs at the top like everything else here. */}
        <div className={`mv-toolbar ${isImage && mobile ? "mv-toolbar-bottom" : ""}`}>
          <div className="mv-title" title={current?.name ?? ""}>
            {current?.name ?? ""}
          </div>
          <div className="mv-toolbar-actions">
            {isImage && (
              <>
                <button className="mv-icon-btn" onClick={handleRotate} aria-label="Rotate" title="Rotate">
                  <RotateGlyph size={18} />
                </button>
                <button
                  className="mv-icon-btn"
                  onClick={() => setEditingOpen(true)}
                  aria-label="Edit"
                  title="Edit"
                >
                  <PencilGlyph size={17} />
                </button>
              </>
            )}
            {/* Delete lives in the "..." overflow now, not as its own
                always-visible icon -- reported directly. The armed
                confirm chip still shows inline (right next to the
                overflow button) once "Delete" has been picked once, so
                the existing tap-again-to-confirm safety window survives
                the menu closing after the first click. */}
            {deleteArmed && (
              <button
                className="mv-icon-btn mv-delete-btn armed"
                onClick={armOrConfirmDelete}
                disabled={deleting}
                aria-label="Confirm delete"
                title="Click again to confirm"
              >
                <TrashGlyph size={16} />
                <span className="mv-delete-label">Confirm</span>
              </button>
            )}
            <button
              className="mv-icon-btn"
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setOverflowMenu({
                  x: r.right,
                  y: r.bottom + 4,
                  items: [
                    ...(mobile
                      ? [
                          {
                            label: "Share",
                            disabled: !resolvedAbsPath || sharing,
                            onClick: handleShare,
                          },
                        ]
                      : []),
                    { label: "Delete", danger: true, onClick: armOrConfirmDelete },
                  ],
                });
              }}
              aria-label="More"
              title="More"
            >
              ⋯
            </button>
            <button className="mv-icon-btn" onClick={onClose} aria-label="Close" title="Close (Esc)">
              <CloseGlyph size={18} />
            </button>
          </div>
        </div>
        <ContextMenu state={overflowMenu} onClose={() => setOverflowMenu(null)} />

        <button
          className={`mv-nav-btn mv-nav-prev ${hasPrev ? "" : "hidden"}`}
          onClick={goPrev}
          disabled={!hasPrev}
          aria-label="Previous"
          title="Previous"
        >
          <ChevronLeft size={26} />
        </button>
        <button
          className={`mv-nav-btn mv-nav-next ${hasNext ? "" : "hidden"}`}
          onClick={goNext}
          disabled={!hasNext}
          aria-label="Next"
          title="Next"
        >
          <ChevronRight size={26} />
        </button>

        <div className="mv-stage-area">
          {resolveError && <div className="mv-error">{resolveError}</div>}
          {stageError && <div className="mv-error mv-error-toast">{stageError}</div>}
          {!resolveError && current && resolvedSrc && (
            <div className="mv-stage" key={current.fullPath} data-dir={direction ?? ""}>
              {current.kind === "image" && (
                <ImageStage
                  src={resolvedSrc}
                  fullPath={current.fullPath}
                  inVault={current.inVault}
                  rotateSignal={rotateSignal}
                  onRotated={handleRotated}
                  onError={setStageError}
                  onZoomChange={setImageZoomed}
                />
              )}
              {current.kind === "video" && <VideoStage src={resolvedSrc} name={current.name} />}
              {current.kind === "audio" && (
                <AudioStage
                  src={resolvedSrc}
                  name={current.name}
                  hasNext={hasNext}
                  hasPrev={hasPrev}
                  onNext={goNext}
                  onPrev={goPrev}
                  onEnded={handleAudioEnded}
                  repeatOneSignal={repeatOneSignal}
                  playlist={gallery.map((g) => g.name)}
                  currentIndex={index}
                  onSelectTrack={(i) => {
                    setDirection(i > index ? "left" : "right");
                    setIndex(i);
                  }}
                  shuffle={shuffle}
                  onToggleShuffle={() => {
                    setShuffle((s) => !s);
                    setShuffleHistory([]);
                  }}
                  repeatMode={repeatMode}
                  onCycleRepeat={() =>
                    setRepeatMode((m) => (m === "off" ? "all" : m === "all" ? "one" : "off"))
                  }
                />
              )}
            </div>
          )}
        </div>
      </div>

      {editingOpen && current && resolvedSrc && (
        <ImageEditor
          src={resolvedSrc}
          fullPath={current.fullPath}
          inVault={current.inVault}
          onClose={() => setEditingOpen(false)}
          onSaved={handleImageSaved}
        />
      )}
    </div>
  );
}
