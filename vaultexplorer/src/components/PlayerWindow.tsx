import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { api, PlayerItem, PlayableSource } from "../api";
import { ChevronLeft, ChevronRight } from "../icons";
import { TrafficLights } from "../TitleBar";
import "./PlayerWindow.css";

// PlayerWindow -- the standalone "just the video, nothing else" window
// double-clicking a video result in InternetView opens (see App.tsx's
// `?player=1` query-string routing, same convention as PickerView's
// `?picker=`). Real window chrome (macOS traffic lights, with green mapped
// to fullscreen the way every mac video player does) over a single,
// full-bleed video -- no toolbar, no sidebar, no gallery grid around it.
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

// How long the chrome (titlebar + prev/next arrows) stays up after the
// last pointer movement. Matches the in-app VideoStage's own control
// timeout so both players feel like the same player.
const CHROME_HIDE_MS = 2600;

export function PlayerWindow({ kind, items, startIndex }: PlayerWindowProps): React.JSX.Element {
  const [index, setIndex] = useState(Math.min(Math.max(startIndex, 0), Math.max(items.length - 1, 0)));
  const [source, setSource] = useState<PlayableSource | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [chromeVisible, setChromeVisible] = useState(true);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

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
                // Not the embed URL directly: this window's origin is
                // `tauri://localhost`, and handing YouTube a non-http
                // origin is what error 153 ("video player configuration
                // error") *is*. youtube_embed_url returns a loopback page
                // (see ytembed.rs) that embeds YouTube from a real
                // `http://127.0.0.1:<port>` origin.
                url: await api.youtubeEmbedUrl(current.key),
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

  // Any pointer movement brings the chrome back and restarts its countdown.
  // Deliberately not gated on "is playing": this window has no play state
  // of its own for an <iframe> provider, and a still-visible titlebar over
  // a paused video is the macOS behaviour anyway.
  const wake = useCallback(() => {
    setChromeVisible(true);
    if (hideTimer.current) clearTimeout(hideTimer.current);
    hideTimer.current = setTimeout(() => setChromeVisible(false), CHROME_HIDE_MS);
  }, []);

  useEffect(() => {
    wake();
    return () => {
      if (hideTimer.current) clearTimeout(hideTimer.current);
    };
  }, [wake]);

  function goPrev() {
    if (hasPrev) setIndex((i) => i - 1);
  }
  function goNext() {
    if (hasNext) setIndex((i) => i + 1);
  }

  // Native window fullscreen rather than the DOM Fullscreen API: the video
  // lives in a cross-origin <iframe> for YouTube, so requesting DOM
  // fullscreen on our own container leaves the player letterboxed inside
  // it. Taking the whole window full-screen has no such problem and is
  // what the green button means on macOS.
  const toggleFullscreen = useCallback(async () => {
    const win = getCurrentWebviewWindow();
    try {
      const isFs = await win.isFullscreen();
      await win.setFullscreen(!isFs);
    } catch {
      // A window manager that refuses fullscreen shouldn't break playback.
    }
  }, []);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") getCurrentWebviewWindow().close().catch(() => {});
      else if (e.key === "ArrowLeft") goPrev();
      else if (e.key === "ArrowRight") goNext();
      else if (e.key === "f" || e.key === "F") void toggleFullscreen();
      wake();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasPrev, hasNext, toggleFullscreen, wake]);

  return (
    <div
      className={`player-window${chromeVisible ? "" : " chrome-hidden"}`}
      onMouseMove={wake}
      onTouchStart={wake}
    >
      <div className="player-titlebar" data-tauri-drag-region>
        <TrafficLights onMaximize={toggleFullscreen} />
        <div className="player-title" title={current?.title ?? ""} data-tauri-drag-region>
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
