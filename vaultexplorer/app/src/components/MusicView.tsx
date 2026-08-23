import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, MusicTrack } from "../api";
import {
  displaySubtitle,
  displayTitle,
  folderCounts,
  formatTime,
  mostPlayed,
  Order,
  orderTracks,
  stepIndex,
} from "../musicQueue";
import { useMediaBlobSrc } from "../hooks/useMediaBlobSrc";
import {
  MusicNoteGlyph,
  PauseGlyph,
  PlayGlyph,
  RepeatGlyph,
  ShuffleGlyph,
  SkipBackGlyph,
  SkipForwardGlyph,
} from "../icons";
import "./MusicView.css";

// MusicView -- the music half of a file manager: a folder full of audio,
// read as songs rather than as files.
//
// It exists because "open it with the system player" answers the wrong
// question for a phone full of downloads. The system player is handed one
// file and knows nothing about the next one, which is why playback stopped
// dead at the end of every track.
//
// Three details here are deliberate, and each one is a bug that was
// reported against the old player:
//
//   * **Auto-advance means calling play().** A media element handed a new
//     `src` loads it and *waits*. The queue therefore remembers that it was
//     playing and starts the next track itself (`playRequestRef`), instead
//     of assuming the element will.
//   * **One element, never remounted.** The <audio> lives here, above the
//     list, and only its `src` changes. Changing tracks used to rebuild the
//     list, which sent the scroll position back to the top of the library --
//     so the list is never keyed on the current track, and nothing scrolls
//     it on a track change.
//   * **No `controls` attribute.** Letting the WebView draw its own
//     transport on top of ours is where the duplicate next button came
//     from. The system-level controls people expect (the notification, the
//     lock screen, headset buttons) come from the Media Session API below,
//     which drives *this* queue rather than being a second player.
export interface MusicViewProps {
  /** Folder to read as a library. */
  root: string;
  /** Opens the tag updater ("Update song data") for this folder. */
  onUpdateTags: (root: string) => void;
}


export function MusicView({ root, onUpdateTags }: MusicViewProps): React.JSX.Element {
  const [tracks, setTracks] = useState<MusicTrack[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [folder, setFolder] = useState<string | null>(null);
  const [order, setOrder] = useState<Order>("folder");
  const [queue, setQueue] = useState<MusicTrack[]>([]);
  const [index, setIndex] = useState(-1);
  const [playing, setPlaying] = useState(false);
  const [shuffle, setShuffle] = useState(false);
  const [repeat, setRepeat] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [seeking, setSeeking] = useState(false);
  const [art, setArt] = useState<string | null>(null);
  const [src, setSrc] = useState("");

  const audioRef = useRef<HTMLAudioElement | null>(null);
  // Set when a track change should start playing: the `src` swap is
  // asynchronous, so the intent has to outlive it.
  const playRequestRef = useRef(false);

  const current = index >= 0 ? (queue[index] ?? null) : null;

  const reload = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const found = await api.musicLibrary(root);
      setTracks(found);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [root]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Folders that actually contain music, with counts -- the switcher only
  // offers somewhere there is something to play.
  const folders = useMemo(() => folderCounts(tracks), [tracks]);
  const visible = useMemo(() => orderTracks(tracks, folder, order), [tracks, folder, order]);

  // ---- playback -------------------------------------------------------

  const countPlay = useCallback((track: MusicTrack) => {
    // Counted when a track starts, not when it is queued: "most played"
    // should mean what it says.
    void api
      .musicPlayed(track.path)
      .then((plays) => {
        setTracks((all) => all.map((t) => (t.path === track.path ? { ...t, plays } : t)));
      })
      .catch(() => {
        // A count that fails to save is not worth interrupting music for.
      });
  }, []);

  const start = useCallback(
    (list: MusicTrack[], at: number) => {
      if (list.length === 0) return;
      const bounded = Math.min(Math.max(at, 0), list.length - 1);
      // Tapping the track that is already playing starts it over -- the
      // src does not change, so the effect below would never fire and the
      // tap would do nothing at all.
      if (queue[index]?.path === list[bounded]?.path && audioRef.current) {
        audioRef.current.currentTime = 0;
        void audioRef.current.play();
        return;
      }
      setQueue(list);
      setIndex(bounded);
      playRequestRef.current = true;
    },
    [index, queue],
  );

  const step = useCallback(
    (delta: number) => {
      const next = stepIndex({ index, length: queue.length, delta, shuffle, repeat });
      if (next === null) {
        // End of the queue with no repeat: stop, rather than wrapping round
        // to track one on its own.
        setPlaying(false);
        return;
      }
      setIndex(next);
      // Going back to the start of the current track is not a track change,
      // so it must not ask for a fresh play.
      if (next !== index) playRequestRef.current = true;
    },
    [index, queue.length, repeat, shuffle],
  );

  // Resolve the current track to a URL the element can play. The local media
  // server (with byte ranges) rather than asset://, which stalls on Android.
  useEffect(() => {
    let cancelled = false;
    if (!current) {
      setSrc("");
      return;
    }
    void api
      .mediaUrl(current.path)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [current]);

  const resolvedSrc = useMediaBlobSrc(src);

  // Cover art, per track, and only when the file says it has some.
  useEffect(() => {
    let cancelled = false;
    setArt(null);
    if (!current?.has_art) return;
    void api
      .musicArt(current.path)
      .then((data) => {
        if (!cancelled) setArt(data);
      })
      .catch(() => {
        // No art is a normal state, not an error worth showing.
      });
    return () => {
      cancelled = true;
    };
  }, [current]);

  // The play that auto-advance depends on. Autoplay policies allow this
  // because the queue only ever starts from a tap.
  useEffect(() => {
    const element = audioRef.current;
    if (!element || !resolvedSrc || !playRequestRef.current) return;
    playRequestRef.current = false;
    const attempt = element.play();
    if (attempt) {
      attempt.catch(() => {
        // Blocked (no gesture yet, or the file will not decode): show it
        // paused rather than pretending.
        setPlaying(false);
      });
    }
    if (current) countPlay(current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedSrc]);

  // System transport: the notification, lock screen and headset buttons.
  // Wired to this queue so there is one player, not two.
  useEffect(() => {
    const session = navigator.mediaSession;
    if (!session || !current) return;
    session.metadata = new MediaMetadata({
      title: displayTitle(current),
      artist: current.artist ?? "",
      album: current.album ?? "",
      artwork: art ? [{ src: art }] : [],
    });
    session.playbackState = playing ? "playing" : "paused";
    const bind = (action: MediaSessionAction, handler: () => void) => {
      try {
        session.setActionHandler(action, handler);
      } catch {
        // Older WebViews reject unknown actions; the rest still work.
      }
    };
    bind("play", () => void audioRef.current?.play());
    bind("pause", () => audioRef.current?.pause());
    bind("nexttrack", () => step(1));
    bind("previoustrack", () => step(-1));
    return () => {
      bind("play", () => {});
      bind("pause", () => {});
    };
  }, [current, art, playing, step]);

  function togglePlay() {
    const element = audioRef.current;
    if (!element) return;
    if (element.paused) void element.play();
    else element.pause();
  }

  const playAll = () => start(visible, 0);
  const playMostPlayed = () => {
    const byPlays = mostPlayed(tracks);
    if (byPlays.length === 0) return;
    setOrder("most-played");
    setFolder(null);
    start(byPlays, 0);
  };

  return (
    <div className="music-view">
      <div className="music-toolbar">
        <button className="music-action primary" onClick={playAll} disabled={visible.length === 0}>
          <PlayGlyph size={13} /> Reproducir todo
          {visible.length > 0 && <span className="music-count">{visible.length}</span>}
        </button>
        <button
          className={`music-action ${order === "most-played" ? "on" : ""}`}
          onClick={playMostPlayed}
          disabled={tracks.every((t) => t.plays === 0)}
          title="Las que más escuchás, primero"
        >
          Más escuchadas
        </button>
        <button
          className={`music-action ${shuffle ? "on" : ""}`}
          onClick={() => setShuffle((s) => !s)}
          title="Aleatorio"
        >
          <ShuffleGlyph size={13} />
        </button>
        <button
          className={`music-action ${repeat ? "on" : ""}`}
          onClick={() => setRepeat((r) => !r)}
          title="Repetir la lista"
        >
          <RepeatGlyph size={13} />
        </button>
        <div className="music-toolbar-spacer" />
        <button className="music-action" onClick={() => onUpdateTags(root)} title="Buscar títulos, álbum, año y portada en internet">
          Actualizar datos
        </button>
        <button className="music-action" onClick={() => void reload()} title="Volver a leer la carpeta">
          Recargar
        </button>
      </div>

      {folders.length > 1 && (
        <div className="music-folders">
          <button
            className={`music-folder ${folder === null ? "on" : ""}`}
            onClick={() => setFolder(null)}
          >
            Todas <span className="music-count">{tracks.length}</span>
          </button>
          {folders.map(([name, count]) => (
            <button
              key={name || "."}
              className={`music-folder ${folder === name ? "on" : ""}`}
              onClick={() => setFolder(name)}
              title={name || "Esta carpeta"}
            >
              {name === "" ? "Esta carpeta" : name.split("/").slice(-1)[0]}
              <span className="music-count">{count}</span>
            </button>
          ))}
        </div>
      )}

      {loading && <div className="music-note">Leyendo la carpeta…</div>}
      {error && <div className="music-error">{error}</div>}
      {!loading && !error && tracks.length === 0 && (
        <div className="music-note">No hay música en esta carpeta.</div>
      )}

      {/* The list is its own scroll container and is never rebuilt when the
          track changes, which is what keeps the scroll position where the
          user left it. */}
      <div className="music-list">
        {visible.map((track, position) => {
          const isCurrent = current?.path === track.path;
          return (
            <button
              key={track.path}
              className={`music-row ${isCurrent ? "current" : ""}`}
              onClick={() => start(visible, position)}
            >
              <span className="music-row-index">
                {isCurrent && playing ? <PlayGlyph size={11} /> : position + 1}
              </span>
              <span className="music-row-text">
                <span className="music-row-title">{displayTitle(track)}</span>
                <span className="music-row-sub">{displaySubtitle(track) || track.folder}</span>
              </span>
              {track.plays > 0 && <span className="music-row-plays">{track.plays}×</span>}
              <span className="music-row-time">
                {track.duration_secs ? formatTime(track.duration_secs) : ""}
              </span>
            </button>
          );
        })}
      </div>

      {/* One element for the whole session. */}
      <audio
        ref={audioRef}
        src={resolvedSrc || undefined}
        preload="metadata"
        onLoadedMetadata={(e) => setDuration(e.currentTarget.duration || 0)}
        onTimeUpdate={(e) => {
          if (!seeking) setCurrentTime(e.currentTarget.currentTime);
        }}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => step(1)}
        onError={() => {
          // A file that will not play must not stop the queue -- and it
          // stops *before* `playing` is ever true, so that flag is the wrong
          // thing to ask.
          if (queue.length > 1) step(1);
        }}
      />

      {current && (
        <div className="music-bar">
          <div className="music-bar-art">
            {art ? <img src={art} alt="" /> : <MusicNoteGlyph size={20} />}
          </div>
          <div className="music-bar-text">
            <div className="music-bar-title">{displayTitle(current)}</div>
            <div className="music-bar-sub">{displaySubtitle(current) || current.folder}</div>
          </div>
          <div className="music-bar-controls">
            <button onClick={() => step(-1)} aria-label="Anterior" disabled={index <= 0 && !shuffle}>
              <SkipBackGlyph size={16} />
            </button>
            <button className="music-bar-play" onClick={togglePlay} aria-label={playing ? "Pausa" : "Reproducir"}>
              {playing ? <PauseGlyph size={17} /> : <PlayGlyph size={17} />}
            </button>
            <button onClick={() => step(1)} aria-label="Siguiente">
              <SkipForwardGlyph size={16} />
            </button>
          </div>
          <div className="music-bar-seek">
            <span>{formatTime(currentTime)}</span>
            <input
              type="range"
              min={0}
              max={duration || 0}
              step={0.5}
              value={Math.min(currentTime, duration || 0)}
              onMouseDown={() => setSeeking(true)}
              onTouchStart={() => setSeeking(true)}
              onMouseUp={() => setSeeking(false)}
              onTouchEnd={() => setSeeking(false)}
              onChange={(e) => {
                const to = Number(e.currentTarget.value);
                setCurrentTime(to);
                if (audioRef.current) audioRef.current.currentTime = to;
              }}
              aria-label="Posición"
            />
            <span>{formatTime(duration)}</span>
          </div>
        </div>
      )}
    </div>
  );
}
