import { useEffect, useRef, useState } from "react";
import {
  MusicNoteGlyph,
  PauseGlyph,
  PlayGlyph,
  Repeat1Glyph,
  RepeatGlyph,
  ReverseGlyph,
  ShuffleGlyph,
  SkipBackGlyph,
  SkipForwardGlyph,
  VolumeGlyph,
  VolumeMuteGlyph,
} from "../../icons";
import { useMediaBlobSrc } from "../../hooks/useMediaBlobSrc";
import "./AudioStage.css";

// AudioStage -- the "now playing" mini music-player shown inside the
// fullscreen MediaViewer when the current gallery item is an audio file.
//
// Prop contract:
//   src:     a directly-playable URL for the current track (already
//            resolved -- for a vault-internal file the caller must have
//            done api.openPath() + convertFileSrc() first, same pattern as
//            PreviewColumn.tsx's toggle()). AudioStage treats every new
//            `src` value as "the user navigated to a different track" and
//            starts playback immediately (autoplay, best-effort -- see
//            below).
//   name:    display name of the track (filename), shown on the card.
//   hasPrev/hasNext: whether a previous/next track exists (already
//            shuffle/repeat-aware -- see MediaViewer's hasPrev/hasNext).
//   onPrev/onNext: callbacks the parent MediaViewer uses to move the
//            gallery cursor and hand AudioStage a new `src`/`name`.
//   onEnded: fired when the track finishes playing naturally -- the parent
//            owns repeat-mode semantics (repeat-one bumps
//            `repeatOneSignal` instead of changing `src`; anything else
//            either advances or does nothing), not this component.
//   repeatOneSignal: bumped by the parent to mean "replay the current
//            track from 0" -- distinct from a `src` change, which this
//            component would otherwise read as "this is a different
//            track" (wrong: it's the same one, on repeat).
//   playlist/currentIndex/onSelectTrack: the sibling tracks in this
//            folder (already how MediaViewer scopes `gallery` for an
//            audio open) shown as a tappable list below the player.
//   shuffle/onToggleShuffle, repeatMode/onCycleRepeat: playback-mode state
//            and toggles -- owned by the parent (it's the one doing the
//            actual index picking), this only renders the buttons.
//
// Autoplay note: browsers may reject a programmatic .play() call that
// isn't the direct result of a user gesture (autoplay policy). That
// rejection is caught silently here -- the track simply starts paused with
// the play button visible/enabled, rather than throwing.
export interface AudioStageProps {
  src: string;
  name: string;
  hasNext: boolean;
  hasPrev: boolean;
  onNext: () => void;
  onPrev: () => void;
  onEnded: () => void;
  repeatOneSignal: number;
  playlist: string[];
  currentIndex: number;
  onSelectTrack: (index: number) => void;
  shuffle: boolean;
  onToggleShuffle: () => void;
  repeatMode: "off" | "all" | "one";
  onCycleRepeat: () => void;
}

// Sensible spread around 1x -- same convention as VideoStage's SPEEDS.
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];
// HTMLMediaElement.volume is spec-clamped to [0, 1] -- reaching the
// requested "up to 200%" needs a Web Audio GainNode downstream of the
// element instead of the element's own volume property.
const MAX_VOLUME = 2;

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s < 10 ? "0" : ""}${s}`;
}

export function AudioStage({
  src,
  name,
  hasNext,
  hasPrev,
  onNext,
  onPrev,
  onEnded,
  repeatOneSignal,
  playlist,
  currentIndex,
  onSelectTrack,
  shuffle,
  onToggleShuffle,
  repeatMode,
  onCycleRepeat,
}: AudioStageProps): React.JSX.Element {
  const audioRef = useRef<HTMLAudioElement>(null);
  const resolvedSrc = useMediaBlobSrc(src);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [rate, setRate] = useState(1);
  const [reversed, setReversed] = useState(false);
  // Guards the seek <input> against being clobbered by the audio element's
  // own timeupdate events while the user has the thumb grabbed.
  const [seeking, setSeeking] = useState(false);

  // ---- Web Audio graph: element -> GainNode -> speakers -----------------
  // Built once and reused across every track (the same <audio> element is
  // reused for the whole gallery, only its `src` changes) --
  // createMediaElementSource throws if called twice on the same element.
  const audioCtxRef = useRef<AudioContext | null>(null);
  const gainNodeRef = useRef<GainNode | null>(null);
  useEffect(() => {
    const el = audioRef.current;
    if (!el) return;
    const AudioContextCtor = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioContextCtor) return;
    const ctx = new AudioContextCtor();
    const source = ctx.createMediaElementSource(el);
    const gain = ctx.createGain();
    source.connect(gain);
    gain.connect(ctx.destination);
    audioCtxRef.current = ctx;
    gainNodeRef.current = gain;
    return () => {
      ctx.close().catch(() => {});
    };
  }, []);
  useEffect(() => {
    if (gainNodeRef.current) gainNodeRef.current.gain.value = muted ? 0 : volume;
  }, [volume, muted]);

  // ---- reverse playback: decode -> reverse each channel -> AudioBufferSourceNode ----
  // <audio>/<video> have no real reverse-playback support (a negative
  // playbackRate is unreliable-to-unsupported for audio in every engine
  // this app targets) -- the only way to genuinely "listen backwards" is
  // decoding the file to PCM, reversing the sample arrays, and playing
  // that back through Web Audio instead of the element.
  const reverseBufferRef = useRef<AudioBuffer | null>(null);
  const reverseNodeRef = useRef<AudioBufferSourceNode | null>(null);
  const reverseStartCtxTimeRef = useRef(0);
  const reverseStartOffsetRef = useRef(0);
  const [reverseLoading, setReverseLoading] = useState(false);
  const [reverseError, setReverseError] = useState("");

  function stopReverseNode() {
    if (reverseNodeRef.current) {
      reverseNodeRef.current.onended = null;
      try {
        reverseNodeRef.current.stop();
      } catch {
        // already stopped
      }
      reverseNodeRef.current = null;
    }
  }

  function startReverseFrom(offsetSeconds: number) {
    const ctx = audioCtxRef.current;
    const gain = gainNodeRef.current;
    const buffer = reverseBufferRef.current;
    if (!ctx || !gain || !buffer) return;
    stopReverseNode();
    const node = ctx.createBufferSource();
    node.buffer = buffer;
    node.playbackRate.value = rate;
    node.connect(gain);
    const clamped = Math.min(Math.max(offsetSeconds, 0), buffer.duration);
    node.start(0, clamped);
    node.onended = () => {
      if (reverseNodeRef.current === node) {
        reverseNodeRef.current = null;
        setPlaying(false);
        onEnded();
      }
    };
    reverseNodeRef.current = node;
    reverseStartCtxTimeRef.current = ctx.currentTime;
    reverseStartOffsetRef.current = clamped;
    setPlaying(true);
  }

  async function toggleReverse() {
    const ctx = audioCtxRef.current;
    if (!ctx) return;
    if (reversed) {
      // Back to normal playback, resuming roughly where the reversed
      // clip left off (mirrored: reverse-position -> forward-position).
      const elapsed = ctx.currentTime - reverseStartCtxTimeRef.current;
      const reversePos = Math.min(reverseStartOffsetRef.current + elapsed * rate, duration || 0);
      stopReverseNode();
      setReversed(false);
      const el = audioRef.current;
      if (el) {
        el.currentTime = Math.max(0, duration - reversePos);
        if (playing) el.play().catch(() => {});
      }
      return;
    }
    setReverseError("");
    setReverseLoading(true);
    try {
      audioRef.current?.pause();
      const res = await fetch(resolvedSrc);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const bytes = await res.arrayBuffer();
      const decoded = await ctx.decodeAudioData(bytes);
      const reversedBuf = ctx.createBuffer(decoded.numberOfChannels, decoded.length, decoded.sampleRate);
      for (let ch = 0; ch < decoded.numberOfChannels; ch++) {
        const data = decoded.getChannelData(ch).slice();
        data.reverse();
        reversedBuf.copyToChannel(data, ch);
      }
      reverseBufferRef.current = reversedBuf;
      setDuration(reversedBuf.duration);
      setReversed(true);
      // Mirror the forward position: reverse-buffer time 0 corresponds
      // to the *end* of the original track.
      const startAt = Math.max(0, (duration || reversedBuf.duration) - currentTime);
      startReverseFrom(startAt);
    } catch (e) {
      setReverseError(String(e));
    } finally {
      setReverseLoading(false);
    }
  }

  // Polls the reversed buffer's elapsed time into the same
  // currentTime/duration state the seek bar already renders from, instead
  // of duplicating that UI for a second playback path.
  useEffect(() => {
    if (!reversed || !playing) return;
    const ctx = audioCtxRef.current;
    if (!ctx) return;
    let raf = 0;
    const tick = () => {
      const elapsed = (ctx.currentTime - reverseStartCtxTimeRef.current) * rate;
      setCurrentTime(Math.min(reverseStartOffsetRef.current + elapsed, duration));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [reversed, playing, rate, duration]);

  // Treat every new `src` as a navigation to a different track: reset the
  // displayed position/duration immediately (don't show the previous
  // track's numbers for a frame) and try to autoplay it.
  useEffect(() => {
    setCurrentTime(0);
    setDuration(0);
    setSeeking(false);
    setReversed(false);
    setReverseError("");
    stopReverseNode();
    reverseBufferRef.current = null;
    const el = audioRef.current;
    if (!el) return;
    el.currentTime = 0;
    el.playbackRate = rate;
    audioCtxRef.current?.resume().catch(() => {});
    const playPromise = el.play();
    if (playPromise && typeof playPromise.then === "function") {
      playPromise
        .then(() => setPlaying(true))
        .catch(() => {
          // Autoplay blocked by the browser -- leave paused, play button
          // stays visible/enabled so the user can start it with one click.
          setPlaying(false);
        });
    } else {
      setPlaying(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedSrc]);

  // Parent asked for a replay-from-0 of the *same* track (repeat-one) --
  // distinct from the `src`-keyed effect above, which would otherwise
  // never re-fire for an unchanged src.
  useEffect(() => {
    if (repeatOneSignal === 0) return;
    const el = audioRef.current;
    if (reversed) {
      startReverseFrom(0);
      return;
    }
    if (!el) return;
    el.currentTime = 0;
    el.play().catch(() => setPlaying(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repeatOneSignal]);

  function togglePlay() {
    if (reversed) {
      const ctx = audioCtxRef.current;
      if (!ctx) return;
      if (playing) {
        stopReverseNode();
        setPlaying(false);
      } else {
        startReverseFrom(currentTime);
      }
      return;
    }
    const el = audioRef.current;
    if (!el) return;
    if (el.paused) {
      audioCtxRef.current?.resume().catch(() => {});
      el.play().catch(() => setPlaying(false));
    } else {
      el.pause();
    }
  }

  function handleSeekChange(e: React.ChangeEvent<HTMLInputElement>) {
    const value = Number(e.target.value);
    setCurrentTime(value);
    if (reversed) {
      const buffer = reverseBufferRef.current;
      if (buffer) startReverseFrom(Math.max(0, duration - value));
      return;
    }
    const el = audioRef.current;
    if (el) el.currentTime = value;
  }

  function handleVolumeChange(e: React.ChangeEvent<HTMLInputElement>) {
    const value = Number(e.target.value);
    setVolume(value);
    if (value > 0 && muted) setMuted(false);
  }

  function toggleMute() {
    setMuted((m) => !m);
  }

  function setSpeed(v: number) {
    setRate(v);
    const el = audioRef.current;
    if (el) el.playbackRate = v;
    if (reverseNodeRef.current) reverseNodeRef.current.playbackRate.value = v;
  }

  const progressPct = duration > 0 ? (currentTime / duration) * 100 : 0;
  const volumePct = muted ? 0 : Math.round((volume / MAX_VOLUME) * 100);

  return (
    <div className="audio-stage">
      <audio
        ref={audioRef}
        src={resolvedSrc}
        preload="metadata"
        onLoadedMetadata={(e) => {
          if (!reversed) setDuration(e.currentTarget.duration || 0);
        }}
        onDurationChange={(e) => {
          if (!reversed) setDuration(e.currentTarget.duration || 0);
        }}
        onTimeUpdate={(e) => {
          if (!seeking && !reversed) setCurrentTime(e.currentTarget.currentTime);
        }}
        onPlay={() => !reversed && setPlaying(true)}
        onPause={() => !reversed && setPlaying(false)}
        onEnded={() => !reversed && onEnded()}
      />

      <div className="audio-stage-card">
        <div className={`audio-art ${playing ? "playing" : ""}`}>
          <div className="audio-art-ring" />
          <MusicNoteGlyph size={46} />
        </div>

        <div className="audio-track-name" title={name}>
          {name}
        </div>
        {reverseError && <div className="audio-error">{reverseError}</div>}

        <div className="audio-seek-row">
          <span className="audio-time">{formatTime(currentTime)}</span>
          <input
            className="audio-seek"
            type="range"
            min={0}
            max={duration || 0}
            step={0.01}
            value={Math.min(currentTime, duration || 0)}
            style={{ "--audio-progress": `${progressPct}%` } as React.CSSProperties}
            onMouseDown={() => setSeeking(true)}
            onTouchStart={() => setSeeking(true)}
            onMouseUp={() => setSeeking(false)}
            onTouchEnd={() => setSeeking(false)}
            onChange={handleSeekChange}
            aria-label="Seek"
          />
          <span className="audio-time">{formatTime(duration)}</span>
        </div>

        <div className="audio-controls-row">
          <button
            className="audio-btn audio-btn-skip"
            onClick={onPrev}
            disabled={!hasPrev}
            aria-label="Previous track"
            title="Previous"
          >
            <SkipBackGlyph size={20} />
          </button>
          <button
            className="audio-btn audio-btn-play"
            onClick={togglePlay}
            disabled={reverseLoading}
            aria-label={playing ? "Pause" : "Play"}
            title={playing ? "Pause" : "Play"}
          >
            {playing ? <PauseGlyph size={24} /> : <PlayGlyph size={24} />}
          </button>
          <button
            className="audio-btn audio-btn-skip"
            onClick={onNext}
            disabled={!hasNext}
            aria-label="Next track"
            title="Next"
          >
            <SkipForwardGlyph size={20} />
          </button>
        </div>

        <div className="audio-mode-row">
          <button
            className={`audio-btn audio-btn-mode ${shuffle ? "on" : ""}`}
            onClick={onToggleShuffle}
            aria-label="Shuffle"
            title="Shuffle"
          >
            <ShuffleGlyph size={16} />
          </button>
          <button
            className={`audio-btn audio-btn-mode ${repeatMode !== "off" ? "on" : ""}`}
            onClick={onCycleRepeat}
            aria-label={`Repeat: ${repeatMode}`}
            title={`Repeat: ${repeatMode === "off" ? "off" : repeatMode === "all" ? "all" : "one"}`}
          >
            {repeatMode === "one" ? <Repeat1Glyph size={16} /> : <RepeatGlyph size={16} />}
          </button>
          <button
            className={`audio-btn audio-btn-mode ${reversed ? "on" : ""}`}
            onClick={toggleReverse}
            disabled={reverseLoading}
            aria-label="Play in reverse"
            title={reverseLoading ? "Decoding…" : "Play in reverse"}
          >
            <ReverseGlyph size={16} />
          </button>
          <select
            className="audio-speed"
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
        </div>

        <div className="audio-volume-row">
          <button
            className="audio-btn audio-btn-mute"
            onClick={toggleMute}
            aria-label={muted ? "Unmute" : "Mute"}
            title={muted ? "Unmute" : "Mute"}
          >
            {muted || volume === 0 ? <VolumeMuteGlyph size={15} /> : <VolumeGlyph size={15} />}
          </button>
          <input
            className="audio-volume"
            type="range"
            min={0}
            max={MAX_VOLUME}
            step={0.01}
            value={muted ? 0 : volume}
            onChange={handleVolumeChange}
            style={{ "--audio-progress": `${volumePct}%` } as React.CSSProperties}
            aria-label="Volume"
          />
          <span className="audio-volume-pct">{Math.round((muted ? 0 : volume) * 100)}%</span>
        </div>

        {playlist.length > 1 && (
          <div className="audio-playlist">
            {playlist.map((trackName, i) => (
              <button
                key={i}
                className={`audio-playlist-item ${i === currentIndex ? "current" : ""}`}
                onClick={() => onSelectTrack(i)}
                title={trackName}
              >
                {i === currentIndex && playing ? (
                  <span className="audio-playlist-now">♪</span>
                ) : (
                  <span className="audio-playlist-index">{i + 1}</span>
                )}
                <span className="audio-playlist-name">{trackName}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
