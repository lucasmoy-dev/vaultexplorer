import { useEffect, useRef, useState } from "react";
import {
  MusicNoteGlyph,
  PauseGlyph,
  PlayGlyph,
  SkipBackGlyph,
  SkipForwardGlyph,
  VolumeGlyph,
  VolumeMuteGlyph,
} from "../../icons";
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
//   hasPrev/hasNext: whether a previous/next track exists in the current
//            gallery -- controls whether the skip buttons are enabled and
//            whether playback auto-advances when the track ends.
//   onPrev/onNext: callbacks the parent MediaViewer uses to move the
//            gallery cursor and hand AudioStage a new `src`/`name`. This
//            component owns no navigation state itself, only playback.
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
}

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
}: AudioStageProps): React.JSX.Element {
  const audioRef = useRef<HTMLAudioElement>(null);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  // Guards the seek <input> against being clobbered by the audio element's
  // own timeupdate events while the user has the thumb grabbed.
  const [seeking, setSeeking] = useState(false);

  // Treat every new `src` as a navigation to a different track: reset the
  // displayed position/duration immediately (don't show the previous
  // track's numbers for a frame) and try to autoplay it.
  useEffect(() => {
    setCurrentTime(0);
    setDuration(0);
    setSeeking(false);
    const el = audioRef.current;
    if (!el) return;
    el.currentTime = 0;
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
  }, [src]);

  function togglePlay() {
    const el = audioRef.current;
    if (!el) return;
    if (el.paused) {
      el.play().catch(() => setPlaying(false));
    } else {
      el.pause();
    }
  }

  function handleEnded() {
    if (hasNext) {
      onNext();
    } else {
      setPlaying(false);
    }
  }

  function handleSeekChange(e: React.ChangeEvent<HTMLInputElement>) {
    const value = Number(e.target.value);
    setCurrentTime(value);
    const el = audioRef.current;
    if (el) el.currentTime = value;
  }

  function handleVolumeChange(e: React.ChangeEvent<HTMLInputElement>) {
    const value = Number(e.target.value);
    setVolume(value);
    const el = audioRef.current;
    if (el) {
      el.volume = value;
      if (value > 0 && muted) {
        el.muted = false;
        setMuted(false);
      }
    }
  }

  function toggleMute() {
    const el = audioRef.current;
    if (!el) return;
    const next = !muted;
    el.muted = next;
    setMuted(next);
  }

  const progressPct = duration > 0 ? (currentTime / duration) * 100 : 0;

  return (
    <div className="audio-stage">
      <audio
        ref={audioRef}
        src={src}
        preload="metadata"
        onLoadedMetadata={(e) => setDuration(e.currentTarget.duration || 0)}
        onDurationChange={(e) => setDuration(e.currentTarget.duration || 0)}
        onTimeUpdate={(e) => {
          if (!seeking) setCurrentTime(e.currentTarget.currentTime);
        }}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={handleEnded}
      />

      <div className="audio-stage-card">
        <div className={`audio-art ${playing ? "playing" : ""}`}>
          <div className="audio-art-ring" />
          <MusicNoteGlyph size={46} />
        </div>

        <div className="audio-track-name" title={name}>
          {name}
        </div>

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
            max={1}
            step={0.01}
            value={muted ? 0 : volume}
            onChange={handleVolumeChange}
            aria-label="Volume"
          />
        </div>
      </div>
    </div>
  );
}
