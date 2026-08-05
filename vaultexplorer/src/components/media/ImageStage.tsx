import { useEffect, useRef, useState } from "react";
import { api } from "../../api";
import "./ImageStage.css";

// ImageStage -- the pannable/zoomable image surface shown inside the
// fullscreen MediaViewer when the current gallery item is an image.
// Sibling of AudioStage.tsx (same "self-contained stage the shell drops
// into its dark backdrop" role, just for the image case).
//
// ---------------------------------------------------------------------
// Prop contract (defined here since the MediaViewer shell is being built
// by a parallel agent -- reconcile the two at integration time):
//
//   src           Already-resolved, directly-displayable URL for the
//                 current image (i.e. the output of convertFileSrc,
//                 already resolved through api.openPath() for
//                 vault-internal files by the caller). ImageStage never
//                 resolves paths itself -- same division of labor as
//                 AudioStageProps.src. Changing this value means "the
//                 user navigated to a different image": zoom/pan/rotation
//                 state all reset.
//   fullPath      The real fs path (inVault === false) or vault-relative
//                 path (inVault === true) of the underlying file. Used
//                 only to (a) persist a rotate back to disk and (b) guess
//                 a mime type for the re-encoded file from its extension.
//   inVault       Selects which write API a rotate is saved through:
//                 api.fsWriteBytes(fullPath, bytes) when false,
//                 api.vaultWriteBytes(fullPath, bytes) when true.
//   rotateSignal  A number the parent increments each time it wants a
//                 90deg-clockwise rotate applied and persisted (the
//                 rotate *button* itself lives in the parent's toolbar,
//                 not here). The initial mount value is not treated as a
//                 rotate request -- only later *changes* trigger one.
//                 While a rotate is being saved, further increments are
//                 ignored (see isRotating below); the parent's button
//                 doesn't strictly need to disable itself, since a signal
//                 change during an in-flight rotate is just dropped, but
//                 doing so avoids a confusing "nothing happened" click.
//   onRotated     Optional -- fired once a rotate has been persisted
//                 successfully, in case the parent caches this file's
//                 size/orientation and needs to invalidate that.
//   onError       Optional -- fired with a message if a rotate fails to
//                 save (e.g. permission denied, disk full). ImageStage
//                 only stops its own spinner on failure; surfacing the
//                 error (toast, etc) is left to the parent.
// ---------------------------------------------------------------------
export interface ImageStageProps {
  src: string;
  fullPath: string;
  inVault: boolean;
  rotateSignal?: number;
  onRotated?: () => void;
  onError?: (message: string) => void;
}

const MIN_ZOOM = 1;
const MAX_ZOOM = 8;
const DOUBLE_TAP_MS = 320;
const DOUBLE_TAP_SLOP = 28;

function distance(a: Touch, b: Touch): number {
  return Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY);
}
function midpoint(a: Touch, b: Touch): { x: number; y: number } {
  return { x: (a.clientX + b.clientX) / 2, y: (a.clientY + b.clientY) / 2 };
}

// Best-effort mime type from the file's own extension, for the re-encoded
// rotated copy -- canvas.toBlob() needs one, and the file's original bytes
// aren't otherwise inspected here (that would mean re-reading the file,
// which the already-loaded <img> makes unnecessary).
function mimeFromPath(path: string): string {
  const ext = path.toLowerCase().split(".").pop() ?? "";
  switch (ext) {
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "webp":
      return "image/webp";
    case "gif":
      return "image/gif";
    case "bmp":
      return "image/bmp";
    case "png":
    default:
      return "image/png";
  }
}

export function ImageStage({
  src,
  fullPath,
  inVault,
  rotateSignal,
  onRotated,
  onError,
}: ImageStageProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  // The same double-click that opens a file (from the folder view behind
  // this viewer) can land its second click on *this* element once the
  // viewer has already mounted in response to the first -- the browser
  // dispatches the native dblclick to whatever's under the pointer at
  // that moment, not to the icon that used to be there. Confirmed live:
  // every image opened this way came up already zoomed to 2x, cropped
  // around wherever the file icon happened to sit in the grid, not a
  // sizing/CSS bug at all. Ignoring a double-click in the first moment
  // after mount closes that gap without needing any coordination from
  // the opener.
  const mountedAt = useRef(Date.now());

  // The URL actually handed to <img>. Equal to `src` except right after a
  // successful rotate-save, when a cache-busted variant is swapped in so
  // the browser re-fetches the file's new bytes instead of serving the
  // pre-rotation image back from cache.
  const [displaySrc, setDisplaySrc] = useState(src);

  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);

  // CSS-only spin applied to a wrapper around <img>, independent of the
  // zoom/pan transform on <img> itself -- see the rotate section below for
  // why these are kept on two separate elements.
  const [spinDeg, setSpinDeg] = useState(0);
  const [spinInstant, setSpinInstant] = useState(false);
  const [isRotating, setIsRotating] = useState(false);

  // Every prop-driven `src` change gets a new generation token; any
  // in-flight rotate save from a previous image is checked against this
  // before touching state, so a slow save for image A can't clobber image
  // B's freshly-reset view after the user has already navigated on.
  const genRef = useRef(0);
  const prevRotateSignal = useRef(rotateSignal);
  const pendingRotateLoad = useRef(false);

  // New image navigated to: drop zoom/pan/rotation and adopt the new src
  // as-is (no cache-bust needed, it's a different file).
  useEffect(() => {
    genRef.current += 1;
    setDisplaySrc(src);
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setSpinDeg(0);
    setSpinInstant(false);
    setIsRotating(false);
    pendingRotateLoad.current = false;
  }, [src]);

  // The image can never be panned fully out of view: at a given zoom the
  // rendered box overhangs the container by (zoom - 1) * size, so half of
  // that is the farthest either edge can travel from center. Same
  // technique as PreviewColumn's clampPan.
  function clampPan(p: { x: number; y: number }, z: number) {
    const el = containerRef.current;
    if (!el) return p;
    const maxX = (el.clientWidth * (z - 1)) / 2;
    const maxY = (el.clientHeight * (z - 1)) / 2;
    return {
      x: Math.min(maxX, Math.max(-maxX, p.x)),
      y: Math.min(maxY, Math.max(-maxY, p.y)),
    };
  }

  function toggleFitOrTwoX(anchor?: { x: number; y: number }) {
    setZoom((z) => {
      if (z > 1) {
        setPan({ x: 0, y: 0 });
        return 1;
      }
      const el = containerRef.current;
      if (anchor && el) {
        const rect = el.getBoundingClientRect();
        const cx = anchor.x - rect.left - rect.width / 2;
        const cy = anchor.y - rect.top - rect.height / 2;
        // Zoom toward the double-click/tap point rather than snapping to
        // dead-center, same reasoning as the wheel handler below.
        setPan(clampPan({ x: -cx, y: -cy }, 2));
      }
      return 2;
    });
  }

  // ---- desktop: wheel-zoom + mouse drag-to-pan -------------------------
  // Native (non-passive) wheel listener, like PreviewColumn's -- React's
  // onWheel is passive by default and can't reliably preventDefault.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setZoom((z) => {
        const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z * Math.exp(-e.deltaY * 0.002)));
        const rect = el.getBoundingClientRect();
        const cx = e.clientX - rect.left - rect.width / 2;
        const cy = e.clientY - rect.top - rect.height / 2;
        setPan((p) => clampPan({ x: cx - ((cx - p.x) * next) / z, y: cy - ((cy - p.y) * next) / z }, next));
        return next;
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const dragRef = useRef<{ startX: number; startY: number; panX: number; panY: number } | null>(null);
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

  function onMouseDown(e: React.MouseEvent) {
    if (zoom <= 1 || (e.button !== 0 && e.button !== 1)) return;
    e.preventDefault();
    dragRef.current = { startX: e.clientX, startY: e.clientY, panX: pan.x, panY: pan.y };
    setDragging(true);
  }
  function onDoubleClick(e: React.MouseEvent) {
    if (Date.now() - mountedAt.current < 500) return;
    toggleFitOrTwoX({ x: e.clientX, y: e.clientY });
  }

  // ---- touch: pinch-zoom + one-finger pan/drag + double-tap -----------
  const pinchRef = useRef<{
    dist0: number;
    zoom0: number;
    cx: number;
    cy: number;
    panX0: number;
    panY0: number;
  } | null>(null);
  const touchDragRef = useRef<{ startX: number; startY: number; panX: number; panY: number } | null>(null);
  const tapStartRef = useRef<{ x: number; y: number; time: number } | null>(null);
  const lastTapRef = useRef<{ x: number; y: number; time: number } | null>(null);

  // Mirrors of the latest zoom/pan, read inside the native touch listeners
  // below instead of closing over `zoom`/`pan` directly -- keeps that
  // effect mount-only (empty deps) so it isn't torn down and rebuilt on
  // every single pan update (which would otherwise happen on every
  // mousemove during a desktop drag too, since pan is shared state).
  const zoomRef = useRef(zoom);
  const panRef = useRef(pan);
  useEffect(() => {
    zoomRef.current = zoom;
    panRef.current = pan;
  }, [zoom, pan]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    // Function *expressions*, not declarations -- a hoisted `function`
    // declaration loses TS's narrowing of the `el` null-check above inside
    // its body (it's analyzed at its hoisted position, above the guard),
    // whereas these `const`-bound closures are analyzed at their actual
    // textual position, after it. Same reason `onWheel` above is `const`.
    const onTouchStart = (e: TouchEvent) => {
      if (e.touches.length === 2) {
        e.preventDefault();
        touchDragRef.current = null;
        tapStartRef.current = null;
        const [a, b] = [e.touches[0], e.touches[1]];
        const mid = midpoint(a, b);
        pinchRef.current = {
          dist0: distance(a, b),
          zoom0: zoomRef.current,
          cx: mid.x,
          cy: mid.y,
          panX0: panRef.current.x,
          panY0: panRef.current.y,
        };
      } else if (e.touches.length === 1) {
        const t = e.touches[0];
        tapStartRef.current = { x: t.clientX, y: t.clientY, time: Date.now() };
        if (zoomRef.current > 1) {
          touchDragRef.current = { startX: t.clientX, startY: t.clientY, panX: panRef.current.x, panY: panRef.current.y };
        }
      }
    };

    const onTouchMove = (e: TouchEvent) => {
      if (e.touches.length === 2 && pinchRef.current) {
        e.preventDefault();
        const [a, b] = [e.touches[0], e.touches[1]];
        const p = pinchRef.current;
        const ratio = distance(a, b) / (p.dist0 || 1);
        const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, p.zoom0 * ratio));
        const rect = el.getBoundingClientRect();
        const cx = p.cx - rect.left - rect.width / 2;
        const cy = p.cy - rect.top - rect.height / 2;
        setZoom(next);
        setPan(
          clampPan(
            { x: cx - ((cx - p.panX0) * next) / p.zoom0, y: cy - ((cy - p.panY0) * next) / p.zoom0 },
            next
          )
        );
      } else if (e.touches.length === 1 && touchDragRef.current) {
        e.preventDefault();
        const t = e.touches[0];
        const d = touchDragRef.current;
        setPan(clampPan({ x: d.panX + (t.clientX - d.startX), y: d.panY + (t.clientY - d.startY) }, zoomRef.current));
      }
    };

    const onTouchEnd = (e: TouchEvent) => {
      if (e.touches.length === 0) {
        pinchRef.current = null;
        touchDragRef.current = null;
        const start = tapStartRef.current;
        tapStartRef.current = null;
        // A "tap" is a touch that ended close to where it started (not a
        // pan/pinch gesture) -- only those count for double-tap detection.
        const changed = e.changedTouches[0];
        if (start && changed && Math.hypot(changed.clientX - start.x, changed.clientY - start.y) < DOUBLE_TAP_SLOP) {
          const now = Date.now();
          const last = lastTapRef.current;
          if (last && now - last.time < DOUBLE_TAP_MS && Math.hypot(changed.clientX - last.x, changed.clientY - last.y) < DOUBLE_TAP_SLOP) {
            lastTapRef.current = null;
            toggleFitOrTwoX({ x: changed.clientX, y: changed.clientY });
          } else {
            lastTapRef.current = { x: changed.clientX, y: changed.clientY, time: now };
          }
        }
      } else if (e.touches.length === 1) {
        // Dropped from pinch (two fingers) to one: restart as a plain pan
        // rather than jumping using stale two-finger anchor data.
        pinchRef.current = null;
        const t = e.touches[0];
        touchDragRef.current =
          zoomRef.current > 1
            ? { startX: t.clientX, startY: t.clientY, panX: panRef.current.x, panY: panRef.current.y }
            : null;
      }
    };

    el.addEventListener("touchstart", onTouchStart, { passive: false });
    el.addEventListener("touchmove", onTouchMove, { passive: false });
    el.addEventListener("touchend", onTouchEnd, { passive: false });
    el.addEventListener("touchcancel", onTouchEnd, { passive: false });
    return () => {
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchmove", onTouchMove);
      el.removeEventListener("touchend", onTouchEnd);
      el.removeEventListener("touchcancel", onTouchEnd);
    };
    // Mount-only: live zoom/pan are read from zoomRef/panRef above, not
    // closed over, so this doesn't need to (and shouldn't) re-subscribe
    // on every pan/zoom update -- see the comment on those refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---- rotate: CSS spin now, real pixels once the save lands -----------
  useEffect(() => {
    const prev = prevRotateSignal.current;
    prevRotateSignal.current = rotateSignal;
    if (rotateSignal === undefined || prev === undefined || rotateSignal === prev) return; // initial mount, or no real change
    if (isRotating) return; // drop -- a rotate is already in flight

    const img = imgRef.current;
    if (!img || !img.complete || !img.naturalWidth) {
      onError?.("Image isn't loaded yet");
      return;
    }

    const gen = genRef.current;
    setIsRotating(true);
    setSpinInstant(false);
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setSpinDeg((d) => d + 90);

    const w = img.naturalWidth;
    const h = img.naturalHeight;
    const canvas = document.createElement("canvas");
    canvas.width = h;
    canvas.height = w;
    const ctx = canvas.getContext("2d");
    if (!ctx) {
      setIsRotating(false);
      setSpinDeg((d) => d - 90);
      onError?.("Canvas unavailable");
      return;
    }
    ctx.translate(canvas.width, 0);
    ctx.rotate(Math.PI / 2);
    ctx.drawImage(img, 0, 0, w, h);

    const mime = mimeFromPath(fullPath);
    canvas.toBlob(
      (blob) => {
        if (!blob) {
          if (genRef.current === gen) {
            setIsRotating(false);
            setSpinDeg((d) => d - 90);
          }
          onError?.("Could not encode rotated image");
          return;
        }
        blob
          .arrayBuffer()
          .then((buf) => {
            const bytes = new Uint8Array(buf);
            return inVault ? api.vaultWriteBytes(fullPath, bytes) : api.fsWriteBytes(fullPath, bytes);
          })
          .then(() => {
            if (genRef.current !== gen) return; // navigated away meanwhile
            pendingRotateLoad.current = true;
            // Cache-bust so the browser re-fetches the file's new bytes
            // instead of reusing the pre-rotation image from cache.
            setDisplaySrc(`${src}${src.includes("?") ? "&" : "?"}v=${Date.now()}`);
            onRotated?.();
          })
          .catch((err) => {
            if (genRef.current === gen) {
              setIsRotating(false);
              setSpinDeg((d) => d - 90);
            }
            onError?.(String(err));
          });
      },
      mime,
      0.92
    );
    // Only `rotateSignal` should drive this effect -- src/inVault/fullPath
    // changing mid-rotate is handled via `gen`, re-running on them would
    // otherwise fire a second rotate off a stale prior signal value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rotateSignal]);

  // The freshly-rotated bytes have loaded: the CSS spin is now redundant
  // (the pixels are really rotated), drop it back to 0 with no transition
  // so nothing visibly moves, then re-arm the transition for next time.
  function onImgLoad() {
    if (!pendingRotateLoad.current) return;
    pendingRotateLoad.current = false;
    setSpinInstant(true);
    setSpinDeg(0);
    setIsRotating(false);
    requestAnimationFrame(() => setSpinInstant(false));
  }

  return (
    <div
      className="image-stage"
      ref={containerRef}
      onMouseDown={onMouseDown}
      onDoubleClick={onDoubleClick}
      style={{ cursor: zoom > 1 ? (dragging ? "grabbing" : "grab") : "default" }}
    >
      <div
        className="image-stage-spin"
        style={{
          transform: `rotate(${spinDeg}deg)`,
          transition: spinInstant ? "none" : "transform 300ms ease",
        }}
      >
        <img
          ref={imgRef}
          src={displaySrc}
          alt=""
          draggable={false}
          onLoad={onImgLoad}
          style={{
            transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
            transition: dragging ? "none" : "transform 120ms ease-out",
          }}
        />
      </div>
      {isRotating && (
        <div className="image-stage-spinner" role="status" aria-label="Saving rotated image">
          <span className="image-stage-spinner-ring" />
        </div>
      )}
    </div>
  );
}
