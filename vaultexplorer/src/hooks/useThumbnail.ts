import { useEffect, useState } from "react";
import { Entry, api } from "../api";
import { kindOf } from "../icons";

// Fires off fs_thumbnail/vault_thumbnail for image-kind entries and
// returns the resolved data-URI (or null while loading/not an image/on
// error, in which case callers fall back to the generic FileIcon glyph).
// `elRef`, when given, gates the actual thumbnail request behind the
// element scrolling into view -- a folder with a few hundred images used
// to fire that many concurrent decode/ffmpeg thumbnail requests the
// instant it opened, all at once, regardless of how many tiles were
// actually visible. Callers that always show exactly one thing (the
// column-view preview pane) skip a ref and load eagerly.

// Module-level, cross-component cache of resolved thumbnails, keyed by
// path+mtime+size (same key the Rust disk cache uses). Survives tile
// unmount/remount and folder navigation, so scrolling back to an
// already-seen folder is instant with no IPC round-trip at all. Bounded so
// it can't grow without limit over a long session.
const memCache = new Map<string, string>();
const MEM_CACHE_MAX = 1500;
function cacheGet(key: string): string | undefined {
  return memCache.get(key);
}
function cacheSet(key: string, uri: string) {
  if (memCache.size >= MEM_CACHE_MAX) {
    // drop the oldest ~10% (Map preserves insertion order)
    const drop = Math.ceil(MEM_CACHE_MAX * 0.1);
    let i = 0;
    for (const k of memCache.keys()) {
      memCache.delete(k);
      if (++i >= drop) break;
    }
  }
  memCache.set(key, uri);
}

// Client-side concurrency cap: even though the Rust side now decodes on a
// blocking threadpool, firing hundreds of `invoke`s at once still floods
// the IPC channel and the pool. A small semaphore keeps only a handful in
// flight; the rest queue and drain as slots free.
const MAX_INFLIGHT = 10;
let inflight = 0;
const waiters: Array<() => void> = [];
function acquire(): Promise<void> {
  if (inflight < MAX_INFLIGHT) {
    inflight++;
    return Promise.resolve();
  }
  return new Promise((resolve) => waiters.push(resolve));
}
function release() {
  const next = waiters.shift();
  if (next) {
    // hand the slot straight to the next waiter (inflight stays the same)
    next();
  } else {
    inflight--;
  }
}

export function useThumbnail(
  entry: Entry,
  fullPath: string,
  inVault: boolean,
  maxSize: number,
  elRef?: React.RefObject<HTMLElement | null>
): string | null {
  const kind = kindOf(entry);
  // Video/PDF thumbnails are real-fs only -- ffmpeg/pdftoppm need a real
  // path (see thumbnail.rs), so either kind inside a vault just keeps the
  // generic icon.
  const thumbable = kind === "image" || ((kind === "video" || kind === "pdf") && !inVault);
  const cacheKey = `${inVault ? "v" : "f"}|${fullPath}|${entry.mtime}|${maxSize}`;
  // Seed initial state straight from the cache so an already-resolved
  // thumbnail paints on first render with no flicker-to-null.
  const [thumb, setThumb] = useState<string | null>(() =>
    thumbable ? cacheGet(cacheKey) ?? null : null
  );
  const [visible, setVisible] = useState(!elRef);

  useEffect(() => {
    if (!elRef || visible) return;
    const el = elRef.current;
    if (!el) return;
    // A generous prefetch margin -- thumbnails should already be decoded
    // by the time a tile scrolls into view, not start decoding right as
    // it crosses the viewport edge (visible pop-in while scrolling).
    const obs = new IntersectionObserver(
      (obsEntries) => {
        if (obsEntries.some((e) => e.isIntersecting)) setVisible(true);
      },
      { rootMargin: "900px" }
    );
    obs.observe(el);
    return () => obs.disconnect();
  }, [elRef, visible]);

  useEffect(() => {
    if (!thumbable || !visible) {
      setThumb(null);
      return;
    }
    const hit = cacheGet(cacheKey);
    if (hit) {
      setThumb(hit);
      return;
    }
    setThumb(null);
    let cancelled = false;
    // Every acquire() is paired with exactly one release(): either the
    // early "cancelled before the slot was granted" branch, or the
    // request's finally. The cleanup only flips `cancelled` -- it must not
    // release, or a slot granted later (or the finally) would double-free.
    acquire().then(() => {
      if (cancelled) {
        release();
        return;
      }
      const req = inVault ? api.vaultThumbnail(fullPath, maxSize) : api.fsThumbnail(fullPath, maxSize);
      req
        .then((uri) => {
          cacheSet(cacheKey, uri);
          if (!cancelled) setThumb(uri);
        })
        .catch(() => {
          /* fall back to the generic icon */
        })
        .finally(release);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [thumbable, visible, cacheKey]);
  return thumb;
}
