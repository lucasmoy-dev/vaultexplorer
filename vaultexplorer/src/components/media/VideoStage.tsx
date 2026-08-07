// VideoStage -- custom-controlled <video> player for the fullscreen media
// viewer. Deliberately *not* using the native `controls` attribute: the
// control bar built below is styled to match this dark fullscreen viewer's
// chrome instead of whatever the browser/OS draws natively.
//
// Prop contract:
//   export interface VideoStageProps {
//     src: string;   // directly-playable URL for the current video --
//                     already resolved by the caller (for a vault-internal
//                     file: api.openPath() + convertFileSrc(), same pattern
//                     as PreviewColumn.tsx's toggle()).
//     name: string;  // display name (filename) of the current video, used
//                     as the <video> element's accessible label.
//   }
//   export function VideoStage(props: VideoStageProps): JSX.Element
//   (written below as React.JSX.Element -- this file has no default React
//   import, and the bare global `JSX` namespace isn't ambient with this
//   @types/react version, so the namespace has to be qualified; same
//   convention as the sibling AudioStage.tsx.)
//
// VideoStage treats every new `src` as "a different video" and resets
// playback to paused/0:00 (see the `[src]` effect below) -- it owns no
// gallery-navigation state itself, same division of responsibility as
// AudioStage.
import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import {
  FullscreenExitGlyph,
  FullscreenGlyph,
  PauseGlyph,
  PlayGlyph,
  VolumeGlyph,
  VolumeMuteGlyph,
} from "../../icons";
import { useMediaBlobSrc } from "../../hooks/useMediaBlobSrc";
import "./VideoStage.css";

export interface VideoStageProps {
  src: string;
  name: string;
}

// Sensible spread around 1x -- covers the user-facing "0.5x, 1.5x, 2x etc."
// ask plus a couple of common in-between steps.
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];
// HTMLMediaElement.volume is spec-clamped to [0, 1] -- reaching the
// requested "up to 200%" needs a Web Audio GainNode downstream of the
// element instead, same as AudioStage's own volume boost.
const MAX_VOLUME = 2;

// How long the pointer has to sit idle while playing before the control
// bar fades away, same UX as YouTube/most video players.
const HIDE_DELAY_MS = 2500;

function fmtTime(totalSeconds: number): string {
  const sec = Number.isFinite(totalSeconds) && totalSeconds > 0 ? totalSeconds : 0;
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export function VideoStage({ src, name }: VideoStageProps): React.JSX.Element {
  const videoRef = useRef<HTMLVideoElement>(null);
  const resolvedSrc = useMediaBlobSrc(src);
  const stageRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [rate, setRate] = useState(1);
  const [ended, setEnded] = useState(false);
  const [controlsVisible, setControlsVisible] = useState(true);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [scrubbing, setScrubbing] = useState(false);
  const [playError, setPlayError] = useState("");

  // Web Audio graph: element -> GainNode -> speakers. Built *lazily*, only
  // once a volume above 100% is actually asked for.
  //
  // It used to be built eagerly on mount, and that alone stopped every
  // local file from playing: routing an element whose src is an `asset://`
  // URL through createMediaElementSource gives WebKit a media stream from
  // an opaque origin, and it then refuses to load the resource at all --
  // the reported "the video player doesn't work", reproduced as a black
  // stage stuck at 0:00/0:00 while the very same file plays fine straight
  // from the element (and the same asset:// URL renders fine as an image).
  // Below 100% the element's own `volume` does the job with no graph, so
  // the common case never touches Web Audio.
  const audioCtxRef = useRef<AudioContext | null>(null);
  const gainNodeRef = useRef<GainNode | null>(null);
  function ensureGain(): GainNode | null {
    if (gainNodeRef.current) return gainNodeRef.current;
    const el = videoRef.current;
    if (!el) return null;
    const AudioContextCtor =
      window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioContextCtor) return null;
    try {
      const ctx = new AudioContextCtor();
      const source = ctx.createMediaElementSource(el);
      const gain = ctx.createGain();
      source.connect(gain);
      gain.connect(ctx.destination);
      audioCtxRef.current = ctx;
      gainNodeRef.current = gain;
      void ctx.resume().catch(() => {});
      return gain;
    } catch {
      // createMediaElementSource throws if this element was already
      // routed once -- and there's nothing to recover, boost just stays
      // unavailable rather than breaking playback.
      return null;
    }
  }
  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    const wanted = muted ? 0 : volume;
    // Only the >1 case needs the graph; everything else stays on the
    // plain element so the boost feature can't cost normal playback.
    const gain = wanted > 1 ? ensureGain() : gainNodeRef.current;
    if (gain) {
      el.volume = 1;
      gain.gain.value = wanted;
    } else {
      el.volume = Math.min(wanted, 1);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [volume, muted]);
  useEffect(() => {
    return () => {
      audioCtxRef.current?.close().catch(() => {});
    };
  }, []);

  // A new src means a new video: snap state back to paused/0:00 rather
  // than showing the *previous* clip's progress for a frame while
  // loadedmetadata/timeupdate catch up. Playback speed is intentionally
  // preserved across files (re-applied to the new element) -- picking 1.5x
  // and then moving to the next video in a gallery shouldn't silently
  // reset it.
  useEffect(() => {
    const el = videoRef.current;
    setPlaying(false);
    setCurrentTime(0);
    setDuration(0);
    setEnded(false);
    if (el) {
      el.pause();
      el.currentTime = 0;
      el.playbackRate = rate;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedSrc]);

  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    const onTime = () => setCurrentTime(el.currentTime);
    const onLoaded = () => setDuration(el.duration || 0);
    const onPlay = () => {
      setPlaying(true);
      setEnded(false);
    };
    const onPause = () => setPlaying(false);
    const onEnded = () => {
      setPlaying(false);
      setEnded(true);
    };
    el.addEventListener("timeupdate", onTime);
    el.addEventListener("loadedmetadata", onLoaded);
    el.addEventListener("durationchange", onLoaded);
    el.addEventListener("play", onPlay);
    el.addEventListener("pause", onPause);
    el.addEventListener("ended", onEnded);
    return () => {
      el.removeEventListener("timeupdate", onTime);
      el.removeEventListener("loadedmetadata", onLoaded);
      el.removeEventListener("durationchange", onLoaded);
      el.removeEventListener("play", onPlay);
      el.removeEventListener("pause", onPause);
      el.removeEventListener("ended", onEnded);
    };
  }, [resolvedSrc]);

  // Fullscreen can be exited by the browser's own affordance (Escape, a
  // native "exit fullscreen" bar) as well as our own button -- listen for
  // the document event rather than only flipping state on click so the
  // icon/label stays truthful either way.
  useEffect(() => {
    const onFsChange = () => setIsFullscreen(document.fullscreenElement === stageRef.current);
    document.addEventListener("fullscreenchange", onFsChange);
    return () => document.removeEventListener("fullscreenchange", onFsChange);
  }, []);

  // Auto-hide the control bar after HIDE_DELAY_MS of no pointer activity,
  // but only while actually playing and not mid-scrub -- paused/scrubbing
  // always keeps it pinned visible.
  useEffect(() => {
    if (playing && !scrubbing) {
      hideTimerRef.current = setTimeout(() => setControlsVisible(false), HIDE_DELAY_MS);
    } else {
      setControlsVisible(true);
    }
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, [playing, scrubbing]);

  function wake() {
    setControlsVisible(true);
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    if (playing && !scrubbing) {
      hideTimerRef.current = setTimeout(() => setControlsVisible(false), HIDE_DELAY_MS);
    }
  }

  // A drag started on the seek input can end (pointerup) outside the
  // window/document; listening on window guarantees scrubbing always
  // clears instead of getting stuck "on" and pinning the bar visible.
  useEffect(() => {
    if (!scrubbing) return;
    const end = () => setScrubbing(false);
    window.addEventListener("pointerup", end);
    window.addEventListener("mouseup", end);
    return () => {
      window.removeEventListener("pointerup", end);
      window.removeEventListener("mouseup", end);
    };
  }, [scrubbing]);

  function togglePlay() {
    const el = videoRef.current;
    if (!el) return;
    if (el.paused || el.ended) {
      if (el.ended) el.currentTime = 0;
      audioCtxRef.current?.resume().catch(() => {});
      // Surfaced, not swallowed: a rejected play() (autoplay policy, a
      // decode failure, a source the engine won't touch) used to leave
      // the stage sitting silently at 0:00 with no clue why, which is
      // most of why "the player doesn't work" took so long to pin down.
      void el.play().then(
        () => setPlayError(""),
        (e: unknown) => setPlayError(String(e)),
      );
    } else {
      el.pause();
    }
  }

  function handleSeek(value: number) {
    const el = videoRef.current;
    if (!el) return;
    el.currentTime = value;
    setCurrentTime(value);
  }

  function toggleMute() {
    setMuted((m) => !m);
  }

  function handleVolume(value: number) {
    setVolume(value);
    if (value === 0) setMuted(true);
    else if (muted) setMuted(false);
  }

  function setSpeed(v: number) {
    const el = videoRef.current;
    if (el) el.playbackRate = v;
    setRate(v);
  }

  async function toggleFullscreen() {
    const stage = stageRef.current;
    if (!stage) return;
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen();
      } else {
        await stage.requestFullscreen();
      }
    } catch {
      // Fullscreen API can reject (e.g. not called from a trusted-enough
      // gesture in some embedders) -- nothing useful to surface, the
      // button just silently no-ops rather than throwing.
    }
  }

  const seekMax = duration || 0;
  const seekValue = Math.min(currentTime, seekMax);
  const progressPct = seekMax > 0 ? (seekValue / seekMax) * 100 : 0;
  const volumePct = muted ? 0 : Math.round((volume / MAX_VOLUME) * 100);

  return (
    <div
      ref={stageRef}
      className={`video-stage${controlsVisible ? "" : " controls-hidden"}`}
      onMouseMove={wake}
      onTouchStart={wake}
    >
      <video
        ref={videoRef}
        src={resolvedSrc}
        aria-label={name}
        className="video-stage-el"
        onClick={togglePlay}
        onError={() => {
          const err = videoRef.current?.error;
          // The src is part of the message on purpose: a media element
          // reports the same code 4 for "empty src" as for "codec I can't
          // handle", and telling those apart from a screenshot is
          // otherwise impossible.
          setPlayError(
            (err ? `Media error ${err.code}: ${err.message || "no detail"}` : "Media error") +
              ` — src: ${resolvedSrc || "(empty)"}`
          );
        }}
        playsInline
      />

      {playError && <p className="video-stage-error">{playError}</p>}

      {ended && (
        <button
          className="video-replay"
          onClick={togglePlay}
          aria-label="Replay"
          title="Replay"
        >
          <PlayGlyph size={30} />
        </button>
      )}

      <div className="video-controls">
        <div className="video-seek">
          <input
            type="range"
            min={0}
            max={seekMax}
            step={0.01}
            value={seekValue}
            onChange={(e) => handleSeek(Number(e.target.value))}
            onPointerDown={() => setScrubbing(true)}
            onPointerUp={() => setScrubbing(false)}
            style={{ "--progress": `${progressPct}%` } as CSSProperties}
            aria-label="Seek"
          />
        </div>

        <div className="video-controls-row">
          <button
            className="video-btn"
            onClick={togglePlay}
            title={playing ? "Pause" : "Play"}
            aria-label={playing ? "Pause" : "Play"}
          >
            {playing ? <PauseGlyph size={18} /> : <PlayGlyph size={18} />}
          </button>

          <div className="video-volume">
            <button
              className="video-btn"
              onClick={toggleMute}
              title={muted ? "Unmute" : "Mute"}
              aria-label={muted ? "Unmute" : "Mute"}
            >
              {muted || volume === 0 ? <VolumeMuteGlyph size={16} /> : <VolumeGlyph size={16} />}
            </button>
            <input
              type="range"
              className="video-volume-slider"
              min={0}
              max={MAX_VOLUME}
              step={0.01}
              value={muted ? 0 : volume}
              onChange={(e) => handleVolume(Number(e.target.value))}
              style={{ "--progress": `${volumePct}%` } as CSSProperties}
              aria-label="Volume"
            />
            <span className="video-volume-pct">{Math.round((muted ? 0 : volume) * 100)}%</span>
          </div>

          <div className="video-time">
            {fmtTime(currentTime)} / {fmtTime(duration)}
          </div>

          <div className="video-spacer" />

          <select
            className="video-speed"
            value={rate}
            onChange={(e) => setSpeed(Number(e.target.value))}
            title="Playback speed"
            aria-label="Playback speed"
          >
            {SPEEDS.map((s) => (
              <option key={s} value={s}>
                {s}x
              </option>
            ))}
          </select>

          <button
            className="video-btn"
            onClick={toggleFullscreen}
            title={isFullscreen ? "Exit fullscreen" : "Fullscreen"}
            aria-label={isFullscreen ? "Exit fullscreen" : "Fullscreen"}
          >
            {isFullscreen ? <FullscreenExitGlyph size={16} /> : <FullscreenGlyph size={16} />}
          </button>
        </div>
      </div>
    </div>
  );
}
