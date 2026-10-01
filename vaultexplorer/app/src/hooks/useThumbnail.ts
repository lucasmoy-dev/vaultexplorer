import { useEffect, useRef, useState } from "react";
import { Entry, api } from "../api";
import { kindOf } from "../icons";
import { Job, Lane, enqueue, lookupThumb, observeVisibility, thumbUrl } from "./thumbQueue";

// Returns a thumbnail URL for `entry` (or null while loading / not
// thumbnailable / on error, in which case callers show the generic
// FileIcon glyph).
//
// `elRef`, when given, ties the work to the element's visibility: nothing
// happens until it comes within the prefetch margin, it renders ahead of
// off-screen tiles while it is actually on screen, and a render still
// queued when it scrolls back out of the margin is dropped. Callers that
// always show exactly one thing (the preview pane) skip the ref and load
// eagerly. See thumbQueue.ts for the queue itself.
//
// Real files at up to 1024px go through the shared freedesktop cache
// (thumbcache.rs): a hit is a PNG path the webview loads directly, so a
// folder that this app -- or Nautilus -- has seen before paints at once.
// Vault files never touch disk (see thumbnail.rs) and keep the in-memory
// data-URI path, as do sizes past the cache's largest class.

// Module-level, cross-component cache of resolved thumbnail URLs, keyed by
// path+mtime+size. Survives tile unmount/remount and folder navigation, so
// going back to an already-seen folder needs no IPC at all. Bounded so it
// can't grow without limit over a long session.
const memCache = new Map<string, string>();
const MEM_CACHE_MAX = 3000;
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

// Android keeps its thumbnails in the app's own cache dir through the
// data-URI path; the freedesktop cache is a desktop-Linux convention.
const SHARED_CACHE_OK = !/Android/i.test(navigator.userAgent);
const SHARED_CACHE_MAX = 1024;

type Vis = { near: boolean; inView: boolean; order: number };

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
  const shared = thumbable && !inVault && maxSize <= SHARED_CACHE_MAX && SHARED_CACHE_OK;
  const lane: Lane = kind === "image" ? "fast" : "slow";
  const cacheKey = `${inVault ? "v" : "f"}|${fullPath}|${entry.mtime}|${maxSize}`;
  // Seed initial state straight from the cache so an already-resolved
  // thumbnail paints on first render with no flicker-to-null.
  const [thumb, setThumb] = useState<string | null>(() => (thumbable ? cacheGet(cacheKey) ?? null : null));
  const [vis, setVis] = useState<Vis>(() =>
    elRef ? { near: false, inView: false, order: 0 } : { near: true, inView: true, order: 0 }
  );
  const visRef = useRef(vis);
  const jobRef = useRef<Job | null>(null);

  useEffect(() => {
    if (!elRef) return;
    const el = elRef.current;
    if (!el) return;
    return observeVisibility(el, (near, inView, order) =>
      setVis((v) => (v.near === near && v.inView === inView ? v : { near, inView, order }))
    );
  }, [elRef]);

  // Re-prioritize a queued render in place when the tile scrolls on or
  // off screen, without restarting it.
  useEffect(() => {
    visRef.current = vis;
    const job = jobRef.current;
    if (job) {
      job.inView = vis.inView;
      job.order = vis.order;
    }
  }, [vis]);

  useEffect(() => {
    if (!thumbable) {
      setThumb(null);
      return;
    }
    const hit = cacheGet(cacheKey);
    if (hit) {
      setThumb(hit);
      return;
    }
    // A new key with nothing cached: don't keep showing the previous
    // file's (or previous version's) picture while this one loads.
    setThumb(null);
    if (!vis.near) return;

    let cancelled = false;
    let dequeue: (() => void) | null = null;
    const done = (uri: string) => {
      cacheSet(cacheKey, uri);
      if (!cancelled) setThumb(uri);
    };
    const queueRender = (render: () => Promise<string>) => {
      if (cancelled) return;
      const job: Job = {
        lane,
        inView: visRef.current.inView,
        order: visRef.current.order,
        run: async () => {
          if (cancelled) return;
          try {
            done(await render());
          } catch {
            /* fall back to the generic icon */
          }
        },
      };
      jobRef.current = job;
      dequeue = enqueue(job);
    };

    // A cached PNG is shown only once the webview has actually loaded and
    // decoded it (off the main thread): no half-painted tile, and a file
    // the asset protocol refuses falls back to the icon instead of
    // showing a broken-image glyph forever.
    const showFile = (url: string) => {
      const img = new Image();
      img.src = url;
      return img.decode().then(() => done(url));
    };
    if (shared) {
      lookupThumb(fullPath, maxSize).then((r) => {
        if (cancelled) return;
        if (r.path) {
          showFile(thumbUrl(r.path, entry.mtime)).catch(() => {});
          return;
        }
        if (r.failed) return;
        queueRender(async () => {
          try {
            const url = thumbUrl(await api.fsThumb(fullPath, maxSize), entry.mtime);
            const img = new Image();
            img.src = url;
            await img.decode();
            return url;
          } catch (e) {
            // A cache dir that can't be written (read-only $HOME, a full
            // disk) shouldn't cost the user their thumbnails: the
            // in-memory path still works. Images only -- for a video the
            // second attempt would just re-run the same failing ffmpeg.
            if (kind !== "image") throw e;
            return api.fsThumbnail(fullPath, maxSize);
          }
        });
      });
    } else {
      queueRender(() => (inVault ? api.vaultThumbnail(fullPath, maxSize) : api.fsThumbnail(fullPath, maxSize)));
    }
    return () => {
      cancelled = true;
      dequeue?.();
      jobRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [thumbable, cacheKey, vis.near]);
  return thumb;
}
