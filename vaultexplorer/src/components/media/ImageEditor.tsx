import { useEffect, useRef, useState } from "react";
import { api } from "../../api";
import {
  CloseGlyph,
  CropGlyph,
  FlipHorizontalGlyph,
  FlipVerticalGlyph,
  PencilGlyph,
  SaveGlyph,
  SlidersGlyph,
  UndoGlyph,
} from "../../icons";
import "./ImageEditor.css";

// ImageEditor -- fullscreen image-editing overlay opened from within the
// MediaViewer's "Edit" action on an image. Everything happens on one
// in-memory <canvas> ("working canvas") whose pixel data IS the current
// edit state: crop replaces its dimensions/content, draw strokes are
// painted directly onto it, adjust bakes a CSS-filter-equivalent into its
// pixels, flip re-draws it mirrored. Nothing touches disk until Save.
//
// Prop contract:
//   src:      currently-displayed, already-resolved image URL to load and
//             edit (a convertFileSrc()'d URL, per the app's usual
//             real-path-then-convert pattern -- see PreviewColumn.tsx's
//             toggle()). This component does not re-resolve it.
//   fullPath: the real fs path (inVault=false) or vault-relative path
//             (inVault=true) to write the edited result back to on Save.
//   inVault:  picks fsWriteBytes vs vaultWriteBytes for the save.
//   onClose:  user cancelled / closed without (further) saving -- any
//             edits already saved via a prior Save click stay saved;
//             whatever is only in the in-memory canvas right now is
//             discarded.
//   onSaved:  called once after a successful save so the parent can
//             re-fetch/refresh the now-edited image.
export interface ImageEditorProps {
  src: string;
  fullPath: string;
  inVault: boolean;
  onClose: () => void;
  onSaved: () => void;
}

type Mode = "crop" | "draw" | "adjust" | "flip";

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

type Handle = "new" | "move" | "nw" | "ne" | "sw" | "se" | "n" | "s" | "e" | "w" | null;

const PEN_COLORS = ["#ff2d2d", "#ffd60a", "#2d7bff", "#28c840", "#ffffff", "#000000"];
const PEN_WIDTHS: { label: string; value: number }[] = [
  { label: "Thin", value: 6 },
  { label: "Thick", value: 16 },
];
const HANDLE_HIT = 14; // px, in screen space, radius for grabbing a crop handle

export function ImageEditor(props: ImageEditorProps): React.JSX.Element {
  const { src, fullPath, inVault, onClose, onSaved } = props;

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);

  const [mode, setMode] = useState<Mode>("crop");
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  // Bumped whenever the working canvas's pixels change, so screen-space
  // dependent state (crop rect, display size) can be recomputed / cleared.
  const [rev, setRev] = useState(0);

  // ---- crop state (canvas-pixel coordinates) ----
  const [cropRect, setCropRect] = useState<Rect | null>(null);
  const dragRef = useRef<{
    handle: Handle;
    startX: number;
    startY: number;
    orig: Rect;
  } | null>(null);

  // ---- draw state ----
  const [penColor, setPenColor] = useState(PEN_COLORS[0]);
  const [penWidth, setPenWidth] = useState(PEN_WIDTHS[1].value);
  const drawingRef = useRef(false);
  const lastPtRef = useRef<{ x: number; y: number } | null>(null);
  const drawSnapshotRef = useRef<ImageData | null>(null);
  const [canUndoDraw, setCanUndoDraw] = useState(false);

  // ---- adjust state ----
  const [saturation, setSaturation] = useState(100);
  const [brightness, setBrightness] = useState(100);
  const [contrast, setContrast] = useState(100);
  const adjustDirty = saturation !== 100 || brightness !== 100 || contrast !== 100;

  // ---- flip state (drives the transient CSS mirror animation) ----
  const [flipAnim, setFlipAnim] = useState<"h" | "v" | null>(null);

  // Load the source image once and paint it onto the working canvas at its
  // natural (full) resolution -- everything downstream operates in that
  // pixel space, independent of however big the on-screen element is.
  useEffect(() => {
    let cancelled = false;
    const img = new Image();
    img.onload = () => {
      if (cancelled) return;
      const canvas = canvasRef.current;
      if (!canvas) return;
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.drawImage(img, 0, 0);
      setReady(true);
      setRev((r) => r + 1);
    };
    img.onerror = () => {
      if (!cancelled) setError("Could not load the image for editing.");
    };
    img.src = src;
    return () => {
      cancelled = true;
    };
    // Intentionally only depends on `src` -- re-running on every `mode`
    // change would wipe out in-progress edits.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src]);

  function getCanvasCtx(): CanvasRenderingContext2D | null {
    const canvas = canvasRef.current;
    return canvas ? canvas.getContext("2d") : null;
  }

  // Maps a pointer event's client coordinates to working-canvas pixel
  // coordinates, accounting for the scale factor between the canvas's
  // rendered CSS box and its actual pixel dimensions.
  function toCanvasPoint(e: { clientX: number; clientY: number }): { x: number; y: number } {
    const canvas = canvasRef.current;
    if (!canvas) return { x: 0, y: 0 };
    const rect = canvas.getBoundingClientRect();
    const scaleX = canvas.width / rect.width;
    const scaleY = canvas.height / rect.height;
    const x = (e.clientX - rect.left) * scaleX;
    const y = (e.clientY - rect.top) * scaleY;
    return { x, y };
  }

  // Screen-space (CSS px) size of one working-canvas pixel, used to size
  // hit-test tolerances and handle knobs consistently regardless of zoom.
  function screenPxPerCanvasPx(): { x: number; y: number } {
    const canvas = canvasRef.current;
    if (!canvas) return { x: 1, y: 1 };
    const rect = canvas.getBoundingClientRect();
    return { x: rect.width / canvas.width, y: rect.height / canvas.height };
  }

  // ---------------------------------------------------------------------
  // Crop mode
  // ---------------------------------------------------------------------

  function clampRect(r: Rect): Rect {
    const canvas = canvasRef.current;
    const maxW = canvas?.width ?? r.w;
    const maxH = canvas?.height ?? r.h;
    let { x, y, w, h } = r;
    w = Math.max(8, w);
    h = Math.max(8, h);
    x = Math.min(Math.max(0, x), maxW - w);
    y = Math.min(Math.max(0, y), maxH - h);
    w = Math.min(w, maxW - x);
    h = Math.min(h, maxH - y);
    return { x, y, w, h };
  }

  function hitTestHandle(pt: { x: number; y: number }, r: Rect): Handle {
    const px = screenPxPerCanvasPx();
    const tolX = HANDLE_HIT / Math.max(px.x, 0.0001);
    const tolY = HANDLE_HIT / Math.max(px.y, 0.0001);
    const near = (ax: number, ay: number) => Math.abs(pt.x - ax) <= tolX && Math.abs(pt.y - ay) <= tolY;
    if (near(r.x, r.y)) return "nw";
    if (near(r.x + r.w, r.y)) return "ne";
    if (near(r.x, r.y + r.h)) return "sw";
    if (near(r.x + r.w, r.y + r.h)) return "se";
    if (near(r.x + r.w / 2, r.y)) return "n";
    if (near(r.x + r.w / 2, r.y + r.h)) return "s";
    if (near(r.x, r.y + r.h / 2)) return "w";
    if (near(r.x + r.w, r.y + r.h / 2)) return "e";
    if (pt.x >= r.x && pt.x <= r.x + r.w && pt.y >= r.y && pt.y <= r.y + r.h) return "move";
    return null;
  }

  function handleCropPointerDown(e: React.PointerEvent<HTMLCanvasElement>) {
    const pt = toCanvasPoint(e);
    if (cropRect) {
      const h = hitTestHandle(pt, cropRect);
      if (h) {
        (e.target as Element).setPointerCapture?.(e.pointerId);
        dragRef.current = { handle: h, startX: pt.x, startY: pt.y, orig: cropRect };
        return;
      }
    }
    // Start a brand-new selection -- anchored at the mousedown point, grown
    // toward wherever the pointer moves next (any of the four directions,
    // not just down-right).
    (e.target as Element).setPointerCapture?.(e.pointerId);
    const start = { x: pt.x, y: pt.y, w: 0, h: 0 };
    dragRef.current = { handle: "new", startX: pt.x, startY: pt.y, orig: start };
    setCropRect(start);
  }

  function handleCropPointerMove(e: React.PointerEvent<HTMLCanvasElement>) {
    const drag = dragRef.current;
    if (!drag) return;
    const pt = toCanvasPoint(e);
    if (drag.handle === "new") {
      // Anchor stays at the original mousedown point; the rect is the
      // min/max box between the anchor and the current pointer position so
      // dragging up/left works exactly like dragging down/right.
      const x = Math.min(drag.startX, pt.x);
      const y = Math.min(drag.startY, pt.y);
      const w = Math.abs(pt.x - drag.startX);
      const h = Math.abs(pt.y - drag.startY);
      setCropRect(clampRect({ x, y, w, h }));
      return;
    }
    const dx = pt.x - drag.startX;
    const dy = pt.y - drag.startY;
    const o = drag.orig;
    let next: Rect = o;
    switch (drag.handle) {
      case "move":
        next = { ...o, x: o.x + dx, y: o.y + dy };
        break;
      case "nw":
        next = { x: o.x + dx, y: o.y + dy, w: o.w - dx, h: o.h - dy };
        break;
      case "ne":
        next = { x: o.x, y: o.y + dy, w: o.w + dx, h: o.h - dy };
        break;
      case "sw":
        next = { x: o.x + dx, y: o.y, w: o.w - dx, h: o.h + dy };
        break;
      case "se":
        next = { x: o.x, y: o.y, w: o.w + dx, h: o.h + dy };
        break;
      case "n":
        next = { x: o.x, y: o.y + dy, w: o.w, h: o.h - dy };
        break;
      case "s":
        next = { x: o.x, y: o.y, w: o.w, h: o.h + dy };
        break;
      case "w":
        next = { x: o.x + dx, y: o.y, w: o.w - dx, h: o.h };
        break;
      case "e":
        next = { x: o.x, y: o.y, w: o.w + dx, h: o.h };
        break;
      default:
        return;
    }
    setCropRect(clampRect(next));
  }

  function endCropDrag() {
    dragRef.current = null;
  }

  function applyCrop() {
    const canvas = canvasRef.current;
    const ctx = getCanvasCtx();
    if (!canvas || !ctx || !cropRect) return;
    const { x, y, w, h } = cropRect;
    if (w < 1 || h < 1) return;
    const sx = Math.round(x);
    const sy = Math.round(y);
    const sw = Math.round(w);
    const sh = Math.round(h);
    const cropped = document.createElement("canvas");
    cropped.width = sw;
    cropped.height = sh;
    const cctx = cropped.getContext("2d");
    if (!cctx) return;
    cctx.drawImage(canvas, sx, sy, sw, sh, 0, 0, sw, sh);
    canvas.width = sw;
    canvas.height = sh;
    ctx.drawImage(cropped, 0, 0);
    setCropRect(null);
    setRev((r) => r + 1);
  }

  // ---------------------------------------------------------------------
  // Draw mode
  // ---------------------------------------------------------------------

  function enterDrawMode() {
    const canvas = canvasRef.current;
    const ctx = getCanvasCtx();
    if (canvas && ctx) {
      drawSnapshotRef.current = ctx.getImageData(0, 0, canvas.width, canvas.height);
      setCanUndoDraw(false);
    }
  }

  function handleDrawPointerDown(e: React.PointerEvent<HTMLCanvasElement>) {
    (e.target as Element).setPointerCapture?.(e.pointerId);
    drawingRef.current = true;
    const pt = toCanvasPoint(e);
    lastPtRef.current = pt;
    const ctx = getCanvasCtx();
    if (!ctx) return;
    // A dot for a plain tap/click (no drag).
    ctx.fillStyle = penColor;
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, penWidth / 2, 0, Math.PI * 2);
    ctx.fill();
    setCanUndoDraw(true);
  }

  function handleDrawPointerMove(e: React.PointerEvent<HTMLCanvasElement>) {
    if (!drawingRef.current) return;
    const ctx = getCanvasCtx();
    const last = lastPtRef.current;
    if (!ctx || !last) return;
    const pt = toCanvasPoint(e);
    ctx.strokeStyle = penColor;
    ctx.lineWidth = penWidth;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(last.x, last.y);
    ctx.lineTo(pt.x, pt.y);
    ctx.stroke();
    lastPtRef.current = pt;
  }

  function handleDrawPointerUp() {
    drawingRef.current = false;
    lastPtRef.current = null;
    setRev((r) => r + 1);
  }

  function undoDraw() {
    const canvas = canvasRef.current;
    const ctx = getCanvasCtx();
    const snap = drawSnapshotRef.current;
    if (!canvas || !ctx || !snap) return;
    ctx.putImageData(snap, 0, 0);
    setCanUndoDraw(false);
    setRev((r) => r + 1);
  }

  // ---------------------------------------------------------------------
  // Adjust mode -- live CSS-filter preview, baked into pixels on confirm.
  // ---------------------------------------------------------------------

  function filterString(): string {
    return `saturate(${saturation}%) brightness(${brightness}%) contrast(${contrast}%)`;
  }

  function applyAdjustments() {
    const canvas = canvasRef.current;
    const ctx = getCanvasCtx();
    if (!canvas || !ctx || !adjustDirty) return;
    const tmp = document.createElement("canvas");
    tmp.width = canvas.width;
    tmp.height = canvas.height;
    const tctx = tmp.getContext("2d");
    if (!tctx) return;
    tctx.filter = filterString();
    tctx.drawImage(canvas, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.filter = "none";
    ctx.drawImage(tmp, 0, 0);
    setSaturation(100);
    setBrightness(100);
    setContrast(100);
    setRev((r) => r + 1);
  }

  // ---------------------------------------------------------------------
  // Flip mode
  // ---------------------------------------------------------------------

  function flip(axis: "h" | "v") {
    const canvas = canvasRef.current;
    const ctx = getCanvasCtx();
    if (!canvas || !ctx) return;
    const tmp = document.createElement("canvas");
    tmp.width = canvas.width;
    tmp.height = canvas.height;
    const tctx = tmp.getContext("2d");
    if (!tctx) return;
    tctx.drawImage(canvas, 0, 0);
    ctx.save();
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (axis === "h") {
      ctx.translate(canvas.width, 0);
      ctx.scale(-1, 1);
    } else {
      ctx.translate(0, canvas.height);
      ctx.scale(1, -1);
    }
    ctx.drawImage(tmp, 0, 0);
    ctx.restore();
    setRev((r) => r + 1);
    // Transient mirror animation on the visible element, timed to match
    // the pixel flip that already happened above so it reads as one
    // motion rather than an instant jump-cut.
    setFlipAnim(axis);
    window.setTimeout(() => setFlipAnim(null), 300);
  }

  // ---------------------------------------------------------------------
  // Mode switching
  // ---------------------------------------------------------------------

  function selectMode(next: Mode) {
    if (mode === "adjust" && next !== "adjust") applyAdjustments();
    if (mode === "crop" && next !== "crop") setCropRect(null);
    if (next === "draw") enterDrawMode();
    setMode(next);
  }

  // ---------------------------------------------------------------------
  // Save
  // ---------------------------------------------------------------------

  async function handleSave() {
    const canvas = canvasRef.current;
    if (!canvas) return;
    setError("");
    setSaving(true);
    try {
      // Bake any still-pending live adjustment before export.
      if (mode === "adjust" && adjustDirty) applyAdjustments();
      const blob = await new Promise<Blob | null>((resolve) =>
        canvas.toBlob((b) => resolve(b), "image/png")
      );
      if (!blob) throw new Error("Could not encode the edited image.");
      const bytes = new Uint8Array(await blob.arrayBuffer());
      if (inVault) {
        await api.vaultWriteBytes(fullPath, bytes);
      } else {
        await api.fsWriteBytes(fullPath, bytes);
      }
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  // ---------------------------------------------------------------------
  // Keyboard: Escape closes (cancel), matching the app's sheet convention.
  // ---------------------------------------------------------------------

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  // Crop overlay geometry, in screen (CSS) px, derived from the canvas
  // pixel rect + current render scale -- recomputed on every render so it
  // tracks window resizes without extra listeners.
  function cropOverlayStyle(): React.CSSProperties {
    if (!cropRect) return { display: "none" };
    const px = screenPxPerCanvasPx();
    return {
      left: cropRect.x * px.x,
      top: cropRect.y * px.y,
      width: cropRect.w * px.x,
      height: cropRect.h * px.y,
    };
  }

  const canvasClassName = [
    "editor-canvas",
    mode === "crop" ? "mode-crop" : "",
    mode === "draw" ? "mode-draw" : "",
    flipAnim === "h" ? "flip-h" : "",
    flipAnim === "v" ? "flip-v" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className="image-editor-overlay" onMouseDown={(e) => e.stopPropagation()}>
      <div className="image-editor-toolbar">
        <button className="btn-plain small" onClick={onClose} title="Close">
          <CloseGlyph size={14} />
          <span>Close</span>
        </button>
        <div className="image-editor-modes">
          <button
            className={`mode-tab ${mode === "crop" ? "on" : ""}`}
            onClick={() => selectMode("crop")}
          >
            <CropGlyph size={14} />
            <span>Crop</span>
          </button>
          <button
            className={`mode-tab ${mode === "draw" ? "on" : ""}`}
            onClick={() => selectMode("draw")}
          >
            <PencilGlyph size={14} />
            <span>Draw</span>
          </button>
          <button
            className={`mode-tab ${mode === "adjust" ? "on" : ""}`}
            onClick={() => selectMode("adjust")}
          >
            <SlidersGlyph size={14} />
            <span>Adjust</span>
          </button>
          <button
            className={`mode-tab ${mode === "flip" ? "on" : ""}`}
            onClick={() => selectMode("flip")}
          >
            <FlipHorizontalGlyph size={14} />
            <span>Flip</span>
          </button>
        </div>
        <button className="btn-primary small" onClick={handleSave} disabled={saving || !ready}>
          <SaveGlyph size={13} />
          <span>{saving ? "Saving…" : "Save"}</span>
        </button>
      </div>

      <div className="image-editor-stage" ref={wrapRef} data-rev={rev}>
        {!ready && !error && <div className="image-editor-status">Loading…</div>}
        {error && <div className="image-editor-status error">{error}</div>}
        <div className="editor-canvas-wrap">
          <canvas
            ref={canvasRef}
            className={canvasClassName}
            style={mode === "adjust" ? { filter: filterString() } : undefined}
            onPointerDown={
              mode === "crop"
                ? handleCropPointerDown
                : mode === "draw"
                  ? handleDrawPointerDown
                  : undefined
            }
            onPointerMove={
              mode === "crop"
                ? handleCropPointerMove
                : mode === "draw"
                  ? handleDrawPointerMove
                  : undefined
            }
            onPointerUp={
              mode === "crop" ? endCropDrag : mode === "draw" ? handleDrawPointerUp : undefined
            }
            onPointerCancel={
              mode === "crop" ? endCropDrag : mode === "draw" ? handleDrawPointerUp : undefined
            }
          />
          {mode === "crop" && cropRect && (
            <>
              <div className="crop-mask" />
              <div className="crop-rect" style={cropOverlayStyle()}>
                <span className="crop-handle nw" />
                <span className="crop-handle ne" />
                <span className="crop-handle sw" />
                <span className="crop-handle se" />
                <span className="crop-handle n" />
                <span className="crop-handle s" />
                <span className="crop-handle e" />
                <span className="crop-handle w" />
              </div>
            </>
          )}
        </div>
      </div>

      <div className="image-editor-options" key={mode}>
        {mode === "crop" && (
          <div className="options-row">
            <p className="options-hint">
              Drag on the image to select an area, then drag the handles to adjust it.
            </p>
            <button className="btn-plain small" onClick={() => setCropRect(null)} disabled={!cropRect}>
              Clear Selection
            </button>
            <button className="btn-primary small" onClick={applyCrop} disabled={!cropRect}>
              Apply Crop
            </button>
          </div>
        )}

        {mode === "draw" && (
          <div className="options-row">
            <div className="pen-colors">
              {PEN_COLORS.map((c) => (
                <button
                  key={c}
                  className={`pen-swatch ${penColor === c ? "on" : ""}`}
                  style={{ background: c }}
                  title={c}
                  onClick={() => setPenColor(c)}
                />
              ))}
            </div>
            <div className="pen-widths">
              {PEN_WIDTHS.map((w) => (
                <button
                  key={w.value}
                  className={`btn-plain small ${penWidth === w.value ? "on" : ""}`}
                  onClick={() => setPenWidth(w.value)}
                >
                  {w.label}
                </button>
              ))}
            </div>
            <button className="btn-plain small" onClick={undoDraw} disabled={!canUndoDraw}>
              <UndoGlyph size={13} />
              <span>Undo</span>
            </button>
          </div>
        )}

        {mode === "adjust" && (
          <div className="options-row adjust-row">
            <label className="adjust-slider">
              <span>Saturation</span>
              <input
                type="range"
                min={0}
                max={200}
                value={saturation}
                onChange={(e) => setSaturation(Number(e.target.value))}
              />
              <span className="adjust-value">{saturation}%</span>
            </label>
            <label className="adjust-slider">
              <span>Brightness</span>
              <input
                type="range"
                min={0}
                max={200}
                value={brightness}
                onChange={(e) => setBrightness(Number(e.target.value))}
              />
              <span className="adjust-value">{brightness}%</span>
            </label>
            <label className="adjust-slider">
              <span>Contrast</span>
              <input
                type="range"
                min={0}
                max={200}
                value={contrast}
                onChange={(e) => setContrast(Number(e.target.value))}
              />
              <span className="adjust-value">{contrast}%</span>
            </label>
            <button className="btn-primary small" onClick={applyAdjustments} disabled={!adjustDirty}>
              Apply
            </button>
          </div>
        )}

        {mode === "flip" && (
          <div className="options-row">
            <button className="btn-plain" onClick={() => flip("h")}>
              <FlipHorizontalGlyph size={15} />
              <span>Flip Horizontal</span>
            </button>
            <button className="btn-plain" onClick={() => flip("v")}>
              <FlipVerticalGlyph size={15} />
              <span>Flip Vertical</span>
            </button>
          </div>
        )}
      </div>

      {error && mode !== "crop" && <div className="image-editor-error">{error}</div>}
    </div>
  );
}
