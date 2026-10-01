import { convertFileSrc, invoke } from "@tauri-apps/api/core";

// ---------------------------------------------------------------------------
// The thumbnail pipeline's frontend half: shared visibility tracking, a
// batched cache check, and a priority render queue.
//
// What it replaces was one IntersectionObserver per tile and one FIFO
// semaphore of ~6 slots shared by everything. In a folder of photos and
// heavy videos that meant: six 4K videos at ~1s of ffmpeg each holding
// every slot while photos -- and thumbnails already sitting in the disk
// cache, which only needed a file read -- waited behind them; tiles that
// had scrolled away still rendered before the ones now on screen, because
// the queue was in arrival order and nothing was ever dropped from it.
//
// Now:
//   * a cache hit never queues: every tile that comes near the viewport in
//     the same frame is checked in ONE `thumb_lookup` IPC, and a hit is a
//     file path the webview loads itself (asset protocol, no base64);
//   * renders queue by priority -- on screen first, top to bottom, then the
//     prefetch margin -- and a tile that leaves the margin is dropped from
//     the queue before it ever starts;
//   * videos/PDFs (an external process each) get their own small lane, so
//     they can never starve the photos.
// ---------------------------------------------------------------------------

export interface LookupResult {
  path: string | null;
  failed: boolean;
}

// ---- batched cache lookups ----

type PendingLookup = { path: string; resolve: (r: LookupResult) => void };
const pendingBySize = new Map<number, PendingLookup[]>();
let lookupFlushScheduled = false;

function flushLookups() {
  lookupFlushScheduled = false;
  const batches = [...pendingBySize.entries()];
  pendingBySize.clear();
  for (const [size, items] of batches) {
    invoke<LookupResult[]>("thumb_lookup", { paths: items.map((i) => i.path), maxSize: size })
      .then((results) => items.forEach((it, i) => it.resolve(results[i] ?? { path: null, failed: false })))
      .catch(() => items.forEach((it) => it.resolve({ path: null, failed: false })));
  }
}

export function lookupThumb(path: string, size: number): Promise<LookupResult> {
  return new Promise((resolve) => {
    const list = pendingBySize.get(size);
    if (list) list.push({ path, resolve });
    else pendingBySize.set(size, [{ path, resolve }]);
    if (!lookupFlushScheduled) {
      lookupFlushScheduled = true;
      // A macrotask, not a microtask: IntersectionObserver delivers a
      // whole screenful of tiles in one callback, and each tile's effect
      // runs in its own commit -- this lets all of them land in one batch.
      setTimeout(flushLookups, 0);
    }
  });
}

// A cached thumbnail's URL, served by the backend's `vxthumb://` scheme
// (thumbcache::serve). The query string never reaches the disk -- it is
// there so the webview's own image cache can't keep showing an old picture
// after the file changed and its thumbnail was rewritten at the same path.
export function thumbUrl(cachePath: string, mtime: number): string {
  return `${convertFileSrc(cachePath, "vxthumb")}?m=${mtime}`;
}

// ---- render queue ----

export type Lane = "fast" | "slow";
const cores = navigator.hardwareConcurrency || 4;
const LIMITS: Record<Lane, number> = {
  // Each image render is a scaled-down decode (~1MB of pixels, not the
  // ~72MB a full 24MP decode used to cost), so this can be wide.
  fast: Math.max(3, Math.min(8, cores - 2)),
  // ffmpeg/pdftoppm are whole processes, often multi-threaded themselves.
  slow: cores >= 8 ? 3 : 2,
};
const running: Record<Lane, number> = { fast: 0, slow: 0 };

export interface Job {
  lane: Lane;
  inView: boolean;
  // Screen position at the time it was (re)prioritized: top-to-bottom,
  // left-to-right, the order a person reads the grid in.
  order: number;
  run: () => Promise<void>;
}
const queued = new Set<Job>();

function better(a: Job, b: Job): boolean {
  if (a.inView !== b.inView) return a.inView;
  return a.order < b.order;
}

function pump() {
  for (const lane of ["fast", "slow"] as Lane[]) {
    while (running[lane] < LIMITS[lane]) {
      let best: Job | null = null;
      for (const j of queued) if (j.lane === lane && (!best || better(j, best))) best = j;
      if (!best) break;
      queued.delete(best);
      running[lane]++;
      best
        .run()
        .catch(() => {})
        .finally(() => {
          running[lane]--;
          pump();
        });
    }
  }
}

export function enqueue(job: Job): () => void {
  queued.add(job);
  // Deferred like the lookups, so a screenful of tiles is all queued (and
  // sorted) before the first slot is handed out.
  setTimeout(pump, 0);
  return () => {
    queued.delete(job);
  };
}

// ---- shared visibility observers ----
//
// Two observers for the whole app instead of one (or two) per tile: "near"
// (a generous prefetch margin -- start before the tile scrolls in) and
// "view" (actually on screen, which is what gets rendered first).

type VisCallback = (near: boolean, inView: boolean, order: number) => void;
const callbacks = new WeakMap<Element, VisCallback>();
const nearState = new WeakMap<Element, boolean>();
const viewState = new WeakMap<Element, boolean>();
let nearObs: IntersectionObserver | null = null;
let viewObs: IntersectionObserver | null = null;

function orderOf(rect: DOMRectReadOnly): number {
  return Math.round(rect.top) * 10000 + Math.round(rect.left);
}

function observers(): [IntersectionObserver, IntersectionObserver] {
  if (!nearObs || !viewObs) {
    const fire = (el: Element, rect: DOMRectReadOnly) =>
      callbacks.get(el)?.(nearState.get(el) ?? false, viewState.get(el) ?? false, orderOf(rect));
    nearObs = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          nearState.set(e.target, e.isIntersecting);
          fire(e.target, e.boundingClientRect);
        }
      },
      { rootMargin: "900px" }
    );
    viewObs = new IntersectionObserver((entries) => {
      for (const e of entries) {
        viewState.set(e.target, e.isIntersecting);
        fire(e.target, e.boundingClientRect);
      }
    });
  }
  return [nearObs, viewObs];
}

export function observeVisibility(el: Element, cb: VisCallback): () => void {
  const [near, view] = observers();
  callbacks.set(el, cb);
  near.observe(el);
  view.observe(el);
  return () => {
    near.unobserve(el);
    view.unobserve(el);
    callbacks.delete(el);
    nearState.delete(el);
    viewState.delete(el);
  };
}
