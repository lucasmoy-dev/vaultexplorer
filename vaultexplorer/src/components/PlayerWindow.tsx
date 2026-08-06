import { useEffect, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { api, PlayerItem, PlayableSource } from "../api";
import { ChevronLeft, ChevronRight, CloseGlyph } from "../icons";
import "./PlayerWindow.css";

// PlayerWindow -- the standalone "just the video, nothing else" window
// double-clicking a video result in InternetView opens (see App.tsx's
// `?player=1` query-string routing, same convention as PickerView's
// `?picker=`). One browser tab's worth of chrome (a title bar with
// close/prev/next) over a single, full-bleed video -- no toolbar, no
// sidebar, no gallery grid around it.
//
// `kind` is the actual provider id ("youtube" | "xhamster" | "cuevana3")
// rather than a generic "youtube vs provider" split -- every non-YouTube
// provider needs its own resolveProviderPlayable call, so the window has
// to know which one regardless.
export interface PlayerWindowProps {
  kind: string;
  items: PlayerItem[];
  startIndex: number;
}

export function PlayerWindow({ kind, items, startIndex }: PlayerWindowProps): React.JSX.Element {
  const [index, setIndex] = useState(Math.min(Math.max(startIndex, 0), Math.max(items.length - 1, 0)));
  const [source, setSource] = useState<PlayableSource | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  const current = items[index] ?? null;
  const hasPrev = index > 0;
  const hasNext = index < items.length - 1;

  useEffect(() => {
    if (!current) return;
    let cancelled = false;
    setLoading(true);
    setError("");
    setSource(null);
    (async () => {
      try {
        const src: PlayableSource =
          kind === "youtube"
            ? {
                kind: "iframe",
                // YouTube's embed player rejects an unrecognized/missing
                // `origin` with error 153 ("video player configuration
                // error") -- a Tauri window's real origin is neither
                // http nor https (a custom asset:// / tauri:// scheme),
                // which is exactly the case that trips it. Passing the
                // *actual* runtime origin (whatever it really is here,
                // not a guessed one) plus enablejsapi is the documented
                // fix for embedding YouTube from a non-standard origin.
                url: `https://www.youtube.com/embed/${current.key}?autoplay=1&enablejsapi=1&origin=${encodeURIComponent(window.location.origin)}`,
              }
            : await api.resolveProviderPlayable(kind, current.key);
        if (!cancelled) setSource(src);
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current?.key]);

  function goPrev() {
    if (hasPrev) setIndex((i) => i - 1);
  }
  function goNext() {
    if (hasNext) setIndex((i) => i + 1);
  }
  function close() {
    getCurrentWebviewWindow().close().catch(() => {});
  }

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") close();
      else if (e.key === "ArrowLeft") goPrev();
      else if (e.key === "ArrowRight") goNext();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasPrev, hasNext]);

  return (
    <div className="player-window" data-tauri-drag-region>
      <div className="player-titlebar" data-tauri-drag-region>
        <button className="player-icon-btn" onClick={close} aria-label="Close" title="Close (Esc)">
          <CloseGlyph size={16} />
        </button>
        <div className="player-title" title={current?.title ?? ""}>
          {current?.title ?? ""}
        </div>
        <div className="player-titlebar-spacer" />
      </div>
      <div className="player-stage">
        {loading && <div className="player-status">Loading…</div>}
        {!loading && error && <div className="player-status player-error">{error}</div>}
        {!loading && !error && source?.kind === "iframe" && (
          <iframe
            className="player-frame"
            src={source.url}
            allow="autoplay; fullscreen; encrypted-media; picture-in-picture"
            allowFullScreen
            referrerPolicy="no-referrer"
          />
        )}
        {!loading && !error && source?.kind === "video" && (
          <video className="player-frame" src={source.url} controls autoPlay />
        )}
        <button
          className={`player-nav-btn player-nav-prev ${hasPrev ? "" : "hidden"}`}
          onClick={goPrev}
          disabled={!hasPrev}
          aria-label="Previous"
        >
          <ChevronLeft size={22} />
        </button>
        <button
          className={`player-nav-btn player-nav-next ${hasNext ? "" : "hidden"}`}
          onClick={goNext}
          disabled={!hasNext}
          aria-label="Next"
        >
          <ChevronRight size={22} />
        </button>
      </div>
    </div>
  );
}
