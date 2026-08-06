import { useEffect, useRef, useState } from "react";
import {
  api,
  osOpen,
  YoutubeResult,
  ImageResult,
  ImageSearchFilters,
  BookResult,
  YoutubeSearchFilters,
  VideoProvider,
  ProviderVideoResult,
  AnimeflvEpisode,
  PlayerItem,
} from "../api";
import { ChevronLeft, SearchGlyph, SaveGlyph, FileIcon } from "../icons";
import { ContextMenu, Dropdown, MenuState } from "../ContextMenu";
import { useSelection } from "../hooks/useSelection";
import { AnimeflvEpisodeSheet } from "./sheets/animeflv-episode-sheet";
import folderVideosIcon from "../assets/foldericons/folder-videos.svg";
import folderImagesIcon from "../assets/foldericons/folder-images.svg";
import folderBookIcon from "../assets/foldericons/folder-book.svg";

type Mode = "root" | "videos" | "images" | "books";

// A result's real, directly-downloadable file -- filename already
// sanitized/extensioned, ready to hand to api.downloadWebResult as-is.
// Only images and books get one (see downloadItemsFor): a video result
// only ever has a page/watch URL, not an actual media file, so there's
// nothing honest to download for it (same reasoning as canPlayInApp).
export interface InternetDownloadItem {
  url: string;
  filename: string;
}

const DEFAULT_FILTERS: YoutubeSearchFilters = { sortByDate: false, uploadDate: null, duration: null };
const DEFAULT_IMAGE_FILTERS: ImageSearchFilters = { fileType: null, size: null, color: null, layout: null };

// A saved search's whole state, round-tripped through a `.ytsearch`/
// `.imgsearch`/`.booksearch` file's JSON content (see `activate()` in
// App.tsx for the open side, and `saveSearch` below for the write side) --
// deliberately small/flat so the file stays human-readable if someone
// opens it in a text editor.
export type SavedInternetSearch =
  | { kind: "videos"; query: string; filters: YoutubeSearchFilters }
  | { kind: "images"; query: string }
  | { kind: "books"; query: string };

// Desktop-only experiment: a sidebar entry that behaves like a folder but
// isn't backed by any real filesystem path -- "Videos"/"Images"/"Books"
// inside it are live search results wearing file icons, not anything
// downloaded or stored anywhere. A search can be saved to a real file
// though (see SaveGlyph button below) -- that file IS a normal filesystem
// citizen, movable/copyable anywhere, and reopening it comes back here
// via `initial` instead of this component inventing its own separate
// storage.
export function InternetView({
  initial,
  onSave,
  mobile,
  onDragResults,
  onSaveToFolder,
}: {
  initial: SavedInternetSearch | null;
  onSave: (filename: string, content: string) => Promise<string>;
  mobile: boolean;
  // Drag-to-a-folder: called with the dragged tile(s)' downloadable items
  // the moment a drag starts, so whichever folder target it's eventually
  // dropped on (a sidebar favorite -- see App.tsx's beginDrag/dropInto for
  // the real-file equivalent) can read them back out and start the real
  // download. Not a native OS-level drag like real files get: there's no
  // file on disk yet to hand another process, so a plain in-window HTML5
  // drag (App.tsx's own vault-entry drag path) is all this needs.
  onDragResults: (items: InternetDownloadItem[]) => void;
  onSaveToFolder: (items: InternetDownloadItem[]) => void;
}) {
  const [mode, setMode] = useState<Mode>("root");
  const [query, setQuery] = useState("");
  const [filters, setFilters] = useState<YoutubeSearchFilters>(DEFAULT_FILTERS);
  const [imageFilters, setImageFilters] = useState<ImageSearchFilters>(DEFAULT_IMAGE_FILTERS);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [videos, setVideos] = useState<YoutubeResult[]>([]);
  const [providerVideos, setProviderVideos] = useState<ProviderVideoResult[]>([]);
  const [images, setImages] = useState<ImageResult[]>([]);
  const [books, setBooks] = useState<BookResult[]>([]);
  const [searched, setSearched] = useState(false);
  const [saveMsg, setSaveMsg] = useState("");
  // Every provider gets inline results (see webfind.rs's module doc
  // comment for how each one is scraped) -- double-clicking a result
  // opens its real page in the system browser, same as a YouTube result.
  const [videoProviders, setVideoProviders] = useState<VideoProvider[]>([{ id: "youtube", label: "YouTube" }]);
  const [provider, setProvider] = useState("youtube");
  useEffect(() => {
    api.listVideoProviders().then(setVideoProviders).catch(() => {});
  }, []);
  // Selection/keyboard-nav behavior matching the real file grid (multi-
  // select, shift-range, arrow keys) -- these results wear file icons and
  // read as files everywhere else in this experiment, so they should
  // behave like a file grid too, not a plain click-only tile list.
  const { selected, setSelected, lastClicked, selectOnly, toggle, selectRange } = useSelection();
  const resultsRef = useRef<HTMLDivElement>(null);
  const arrowAnchorRef = useRef<string | null>(null);
  const arrowFocusRef = useRef<string | null>(null);
  // Drag-to-select rectangle -- same technique as the real file grid's
  // own marquee (App.tsx's onContentMouseDown), a self-contained local
  // copy since this grid's items/container are entirely separate from
  // the real filesystem one.
  const [marquee, setMarquee] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  const [contextMenu, setContextMenu] = useState<MenuState>(null);
  // Opening an AnimeFLV result goes through this picker instead of
  // straight to the external browser -- see AnimeflvEpisodeSheet.
  const [episodePicker, setEpisodePicker] = useState<{ title: string; pageUrl: string } | null>(null);

  // The ordered key list for whatever's currently rendered -- same key
  // format each tile below already uses (v.id / `p${i}` / `i${i}` /
  // `b${i}`), just centralized so keyboard nav and "select all" don't
  // have to duplicate the per-mode branching four times.
  function currentKeys(): string[] {
    if (mode === "videos" && provider === "youtube") return videos.map((v) => v.id);
    if (mode === "videos") return providerVideos.map((_, i) => `p${i}`);
    if (mode === "images") return images.map((_, i) => `i${i}`);
    if (mode === "books") return books.map((_, i) => `b${i}`);
    return [];
  }

  // AnimeFLV's episode player is filled in by client-side JS this app's
  // scraper has no way to run, cuevana3's static scrape turned out to be
  // a decoy clip (not the real movie), and xhamster's own embed player
  // registers the click (its play button visibly changes state) but
  // never actually starts decoding/playing -- confirmed live, on more
  // than one video, and unaffected by a no-referrer attempt -- a real
  // WebKitGTK/player incompatibility, not something fixable through the
  // embed URL. Opening externally is the honest fallback for all three
  // (see resolve_provider_playable's own doc comment). Only YouTube is
  // confirmed to actually play in-app.
  function canPlayInApp(): boolean {
    return !mobile && mode === "videos" && provider === "youtube";
  }

  function openResult(key: string) {
    if (mode === "videos" && canPlayInApp()) {
      const keys = currentKeys();
      const playerItems: PlayerItem[] =
        provider === "youtube"
          ? videos.map((v) => ({ title: v.title, key: v.id }))
          : providerVideos.map((v) => ({ title: v.title, key: v.page_url }));
      api.openPlayerWindow(provider, playerItems, keys.indexOf(key)).catch(() => {});
    } else if (mode === "videos" && provider === "youtube") {
      osOpen(`https://www.youtube.com/watch?v=${key}`).catch(() => {});
    } else if (mode === "videos" && provider === "animeflv") {
      const v = providerVideos[Number(key.slice(1))];
      if (v) setEpisodePicker({ title: v.title, pageUrl: v.page_url });
    } else if (mode === "videos") {
      const v = providerVideos[Number(key.slice(1))];
      if (v) osOpen(v.page_url).catch(() => {});
    } else if (mode === "images") {
      const img = images[Number(key.slice(1))];
      if (img) osOpen(img.image).catch(() => {});
    } else if (mode === "books") {
      const b = books[Number(key.slice(1))];
      if (b) osOpen(b.url).catch(() => {});
    }
  }

  // The real, external page/file URL for a result -- distinct from
  // openResult, which for playable videos opens the in-app player
  // instead. "Open in Browser" always means the real external URL,
  // even for a video that would otherwise open in-app.
  function resultUrl(key: string): string | null {
    if (mode === "videos" && provider === "youtube") return `https://www.youtube.com/watch?v=${key}`;
    if (mode === "videos") return providerVideos[Number(key.slice(1))]?.page_url ?? null;
    if (mode === "images") return images[Number(key.slice(1))]?.image ?? null;
    if (mode === "books") return books[Number(key.slice(1))]?.url ?? null;
    return null;
  }

  function sanitizeFilename(name: string): string {
    return name.trim().replace(/[/\\:*?"<>|]/g, "_").slice(0, 80) || "download";
  }

  // The image URL itself is the real downloadable file -- just needs an
  // extension guessed from it (DDG's own thumbnail/image URLs almost
  // always end in one; "jpg" is the fallback for the rare one that doesn't).
  function imageDownloadItem(img: ImageResult): InternetDownloadItem {
    const m = /\.(jpe?g|png|gif|webp|bmp|svg)(?:[?#]|$)/i.exec(img.image);
    const ext = m ? m[1].toLowerCase() : "jpg";
    return { url: img.image, filename: `${sanitizeFilename(img.title)}.${ext}` };
  }

  // A book result's `url` is archive.org's *details* page, not a file --
  // rewritten to that item's real `/download/<id>/<id>.pdf`, the same
  // filename convention archive.org's own file server uses for a texts
  // item's primary derivative (confirmed live: it 302s to the real CDN
  // mirror hosting the actual PDF). Not guaranteed for every item (some
  // are djvu/OCR-text only, no pdf) -- a missing one surfaces as a real
  // download error rather than silently saving the wrong thing.
  function bookDownloadItem(b: BookResult): InternetDownloadItem | null {
    const id = b.url.split("/details/")[1];
    if (!id) return null;
    return { url: `https://archive.org/download/${id}/${id}.pdf`, filename: `${sanitizeFilename(b.title)}.pdf` };
  }

  function downloadItemsFor(keys: string[]): InternetDownloadItem[] {
    if (mode === "images") {
      return keys.map((k) => images[Number(k.slice(1))]).filter((x): x is ImageResult => !!x).map(imageDownloadItem);
    }
    if (mode === "books") {
      return keys
        .map((k) => books[Number(k.slice(1))])
        .filter((x): x is BookResult => !!x)
        .map(bookDownloadItem)
        .filter((x): x is InternetDownloadItem => !!x);
    }
    return [];
  }

  function handleTileDragStart(key: string, e: React.DragEvent) {
    if (!selected.has(key)) selectOnly(key);
    const keys = selected.has(key) && selected.size > 1 ? [...selected] : [key];
    const items = downloadItemsFor(keys);
    if (items.length === 0) {
      e.preventDefault();
      return;
    }
    e.dataTransfer.effectAllowed = "copy";
    e.dataTransfer.setData("text/plain", items.map((i) => i.url).join("\n"));
    onDragResults(items);
  }

  // Right-click on a result: "Open in Browser" (always the real
  // external URL, regardless of in-app playback) + "Copy Link" -- the
  // same two actions available for every result type. Reported
  // directly as missing on every kind (videos/images/books alike).
  // "Save to Folder…" is a third, non-drag way to reach the same
  // download drag-and-drop now offers -- only for images/books, same
  // gating as handleTileDragStart above (no genuine downloadable file
  // for a video result).
  function onTileContextMenu(key: string, e: React.MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!selected.has(key)) selectOnly(key);
    const keys = selected.has(key) && selected.size > 1 ? [...selected] : [key];
    const downloadItems = downloadItemsFor(keys);
    setContextMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        {
          label: keys.length > 1 ? `Open ${keys.length} in Browser` : "Open in Browser",
          onClick: () => {
            for (const k of keys) {
              const u = resultUrl(k);
              if (u) osOpen(u).catch(() => {});
            }
          },
        },
        {
          label: "Copy Link",
          onClick: () => {
            const links = keys.map(resultUrl).filter((u): u is string => !!u);
            if (links.length) navigator.clipboard.writeText(links.join("\n")).catch(() => {});
          },
        },
        ...(downloadItems.length > 0
          ? [{ label: "Save to Folder…", onClick: () => onSaveToFolder(downloadItems) }]
          : []),
      ],
    });
  }

  // Same geometric row/column measurement App.tsx's own file grid uses
  // for arrow-key nav (see computeArrowTarget there) -- kept as a local,
  // self-contained copy rather than reaching into that one, since this
  // grid's items (data-key, not data-name) and container are entirely
  // separate from the real filesystem grid.
  function computeArrowTarget(direction: "up" | "down" | "left" | "right", fromKey: string): string | null {
    const keys = currentKeys();
    const container = resultsRef.current;
    if (!container) return keys[0] ?? null;
    const tiles = Array.from(container.querySelectorAll<HTMLElement>(".entry.icon"));
    const rows: string[][] = [];
    let lastTop = -1;
    for (const tile of tiles) {
      const key = tile.dataset.key;
      if (!key) continue;
      const top = tile.offsetTop;
      if (rows.length === 0 || Math.abs(top - lastTop) > 4) {
        rows.push([key]);
        lastTop = top;
      } else {
        rows[rows.length - 1].push(key);
      }
    }
    let r = -1;
    let c = -1;
    for (let i = 0; i < rows.length; i++) {
      const j = rows[i].indexOf(fromKey);
      if (j !== -1) {
        r = i;
        c = j;
        break;
      }
    }
    if (r === -1) return keys[0] ?? null;
    if (direction === "left") return rows[r][c - 1] ?? fromKey;
    if (direction === "right") return rows[r][c + 1] ?? fromKey;
    if (direction === "up") return rows[r - 1]?.[Math.min(c, rows[r - 1].length - 1)] ?? fromKey;
    return rows[r + 1]?.[Math.min(c, rows[r + 1].length - 1)] ?? fromKey;
  }

  // Click semantics matching the real file grid: plain click selects
  // only this tile, ctrl/cmd toggles it into/out of the selection,
  // shift extends the range from the last-clicked anchor.
  function handleTileClick(key: string, e: React.MouseEvent) {
    if (e.shiftKey) selectRange(key, currentKeys());
    else if (e.ctrlKey || e.metaKey) toggle(key);
    else selectOnly(key);
    arrowAnchorRef.current = key;
    arrowFocusRef.current = key;
  }

  function onResultsMouseDown(e: React.MouseEvent) {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest(".entry")) return;
    if (!(e.metaKey || e.ctrlKey || e.shiftKey)) setSelected(new Set());
    setMarquee({ x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY });
  }
  useEffect(() => {
    if (!marquee) return;
    const origin = { x: marquee.x0, y: marquee.y0 };
    const rects = Array.from(resultsRef.current?.querySelectorAll<HTMLElement>(".entry.icon") ?? []).map((el) => ({
      key: el.dataset.key,
      rect: el.getBoundingClientRect(),
    }));
    const move = (e: MouseEvent) => {
      setMarquee((m) => (m ? { ...m, x1: e.clientX, y1: e.clientY } : m));
      const left = Math.min(origin.x, e.clientX);
      const right = Math.max(origin.x, e.clientX);
      const top = Math.min(origin.y, e.clientY);
      const bottom = Math.max(origin.y, e.clientY);
      const hit = new Set<string>();
      for (const { key, rect: r } of rects) {
        if (key && r.left < right && r.right > left && r.top < bottom && r.bottom > top) hit.add(key);
      }
      setSelected(hit);
    };
    const up = () => setMarquee(null);
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [marquee !== null]);

  function handleResultsKeyDown(e: React.KeyboardEvent) {
    const keys = currentKeys();
    if (keys.length === 0) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      e.stopPropagation();
      setSelected(new Set(keys));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      for (const key of selected) openResult(key);
      return;
    }
    const isArrow = e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "ArrowLeft" || e.key === "ArrowRight";
    if (!isArrow) return;
    e.preventDefault();
    e.stopPropagation();
    const direction =
      e.key === "ArrowUp" ? "up" : e.key === "ArrowDown" ? "down" : e.key === "ArrowLeft" ? "left" : "right";
    if (e.shiftKey) {
      if (!arrowAnchorRef.current || !keys.includes(arrowAnchorRef.current)) {
        arrowAnchorRef.current = (lastClicked && keys.includes(lastClicked) ? lastClicked : null) ?? keys[0];
      }
      const currentFocus =
        arrowFocusRef.current && keys.includes(arrowFocusRef.current) ? arrowFocusRef.current : arrowAnchorRef.current;
      const target = computeArrowTarget(direction, currentFocus);
      if (!target) return;
      arrowFocusRef.current = target;
      const anchorIdx = keys.indexOf(arrowAnchorRef.current);
      const targetIdx = keys.indexOf(target);
      const [lo, hi] = anchorIdx < targetIdx ? [anchorIdx, targetIdx] : [targetIdx, anchorIdx];
      setSelected(new Set(keys.slice(lo, hi + 1)));
      return;
    }
    const from = (lastClicked && keys.includes(lastClicked) ? lastClicked : null) ?? [...selected][0] ?? keys[0];
    const target = selected.size === 0 ? from : computeArrowTarget(direction, from) ?? from;
    selectOnly(target);
    arrowAnchorRef.current = target;
    arrowFocusRef.current = target;
  }

  async function runSearch(q: string, f: YoutubeSearchFilters, m: Mode) {
    if (!q.trim()) return;
    setLoading(true);
    setError("");
    setSearched(true);
    setSelected(new Set());
    try {
      if (m === "videos" && provider !== "youtube") setProviderVideos(await api.searchProviderVideos(provider, q));
      else if (m === "videos") setVideos(await api.searchYoutube(q, f));
      else if (m === "images") setImages(await api.searchImages(q, imageFilters));
      else if (m === "books") setBooks(await api.searchBooks(q));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  // Reopening a saved search (double-clicked from anywhere in the real
  // filesystem) -- jump straight past the root tiles into results, same
  // query/filters as when it was saved, fetched fresh (never cached).
  useEffect(() => {
    if (!initial) return;
    setMode(initial.kind);
    setQuery(initial.query);
    const f = initial.kind === "videos" ? initial.filters : DEFAULT_FILTERS;
    setFilters(f);
    setSaveMsg("");
    runSearch(initial.query, f, initial.kind);
    // Only meant to react to a *new* file being opened, not every
    // keystroke afterward -- see the eslint-disable-next-line below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initial]);

  function enterFolder(next: Mode) {
    setMode(next);
    setQuery("");
    setFilters(DEFAULT_FILTERS);
    setImageFilters(DEFAULT_IMAGE_FILTERS);
    setSearched(false);
    setError("");
    setSaveMsg("");
    setSelected(new Set());
  }

  async function saveSearch() {
    if (!query.trim()) return;
    const saved: SavedInternetSearch =
      mode === "videos"
        ? { kind: "videos", query, filters }
        : mode === "images"
        ? { kind: "images", query }
        : { kind: "books", query };
    const ext = mode === "videos" ? "ytsearch" : mode === "images" ? "imgsearch" : "booksearch";
    const safeName = query.trim().replace(/[/\\:*?"<>|]/g, "_").slice(0, 60);
    try {
      const path = await onSave(`${safeName}.${ext}`, JSON.stringify(saved));
      setSaveMsg(`Saved as "${path.split("/").pop()}" -- double-click it anytime to rerun this search.`);
    } catch (e) {
      setSaveMsg(String(e));
    }
  }

  if (mode === "root") {
    return (
      <div className="internet-view">
        <div className="entries icon">
          <div className="entry icon" onDoubleClick={() => enterFolder("videos")}>
            <span className="entry-icon">
              <img className="fileicon-img" src={folderVideosIcon} alt="" draggable={false} />
            </span>
            <span className="entry-name">Videos</span>
          </div>
          <div className="entry icon" onDoubleClick={() => enterFolder("images")}>
            <span className="entry-icon">
              <img className="fileicon-img" src={folderImagesIcon} alt="" draggable={false} />
            </span>
            <span className="entry-name">Images</span>
          </div>
          <div className="entry icon" onDoubleClick={() => enterFolder("books")}>
            <span className="entry-icon">
              <img className="fileicon-img" src={folderBookIcon} alt="" draggable={false} />
            </span>
            <span className="entry-name">Books</span>
          </div>
        </div>
        <p className="hint" style={{ padding: "0 14px" }}>
          Experimental: "files" in here are YouTube/web-image/PDF search results, fetched live --
          nothing is downloaded or stored until you open one. Save a search (once you've run one) to
          drop a real file anywhere in your filesystem that reruns it on double-click.
        </p>
      </div>
    );
  }

  const activeProviderLabel = videoProviders.find((p) => p.id === provider)?.label ?? "YouTube";
  const placeholder =
    mode === "videos"
      ? `Search ${activeProviderLabel}…`
      : mode === "images"
      ? "Search images…"
      : "Search for a PDF…";

  return (
    <div className="internet-view">
      <div className="internet-search-bar">
        <button className="tool-btn" onClick={() => enterFolder("root")} aria-label="Back">
          <ChevronLeft size={16} />
        </button>
        <div className="search-field internet-search-field">
          <SearchGlyph />
          <input
            autoFocus
            placeholder={placeholder}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && runSearch(query, filters, mode)}
          />
        </div>
        <button className="tool-btn wide-btn" onClick={() => runSearch(query, filters, mode)} disabled={loading}>
          Search
        </button>
        <button
          className="tool-btn"
          onClick={saveSearch}
          disabled={!query.trim()}
          aria-label="Save search"
          title="Save this search as a file"
        >
          <SaveGlyph size={16} />
        </button>
      </div>
      {mode === "videos" && (
        <div className="internet-filters">
          <Dropdown
            value={provider}
            options={videoProviders.map((p) => ({ value: p.id, label: p.label }))}
            onChange={setProvider}
          />
          <Dropdown
            disabled={provider !== "youtube"}
            value={String(filters.uploadDate ?? "")}
            options={[
              { value: "", label: "Any time" },
              { value: "2", label: "Today" },
              { value: "3", label: "This week" },
              { value: "4", label: "This month" },
              { value: "5", label: "This year" },
            ]}
            onChange={(v) =>
              setFilters((f) => ({ ...f, uploadDate: v ? (Number(v) as 1 | 2 | 3 | 4 | 5) : null }))
            }
          />
          <Dropdown
            disabled={provider !== "youtube"}
            value={String(filters.duration ?? "")}
            options={[
              { value: "", label: "Any length" },
              { value: "1", label: "Under 4 minutes" },
              { value: "3", label: "4-20 minutes" },
              { value: "2", label: "Over 20 minutes" },
            ]}
            onChange={(v) => setFilters((f) => ({ ...f, duration: v ? (Number(v) as 1 | 2 | 3) : null }))}
          />
          <label className="checkbox-row">
            <input
              type="checkbox"
              disabled={provider !== "youtube"}
              checked={filters.sortByDate}
              onChange={(e) => setFilters((f) => ({ ...f, sortByDate: e.target.checked }))}
            />
            Newest first
          </label>
        </div>
      )}
      {mode === "images" && (
        <div className="internet-filters">
          <Dropdown
            value={imageFilters.fileType ?? ""}
            options={[
              { value: "", label: "Any type" },
              { value: "photo", label: "Photo" },
              { value: "clipart", label: "Clipart" },
              { value: "gif", label: "GIF" },
              { value: "transparent", label: "Transparent" },
              { value: "line", label: "Line drawing" },
            ]}
            onChange={(v) => setImageFilters((f) => ({ ...f, fileType: (v || null) as ImageSearchFilters["fileType"] }))}
          />
          <Dropdown
            value={imageFilters.size ?? ""}
            options={[
              { value: "", label: "Any size" },
              { value: "Small", label: "Small" },
              { value: "Medium", label: "Medium" },
              { value: "Large", label: "Large" },
              { value: "Wallpaper", label: "Wallpaper" },
            ]}
            onChange={(v) => setImageFilters((f) => ({ ...f, size: (v || null) as ImageSearchFilters["size"] }))}
          />
          <Dropdown
            value={imageFilters.color ?? ""}
            options={[
              { value: "", label: "Any color" },
              { value: "color", label: "Color" },
              { value: "Monochrome", label: "Black & white" },
              { value: "Red", label: "Red" },
              { value: "Orange", label: "Orange" },
              { value: "Yellow", label: "Yellow" },
              { value: "Green", label: "Green" },
              { value: "Blue", label: "Blue" },
              { value: "Purple", label: "Purple" },
              { value: "Pink", label: "Pink" },
              { value: "Brown", label: "Brown" },
              { value: "Black", label: "Black" },
              { value: "Gray", label: "Gray" },
              { value: "Teal", label: "Teal" },
              { value: "White", label: "White" },
            ]}
            onChange={(v) => setImageFilters((f) => ({ ...f, color: (v || null) as ImageSearchFilters["color"] }))}
          />
          <Dropdown
            value={imageFilters.layout ?? ""}
            options={[
              { value: "", label: "Any layout" },
              { value: "Square", label: "Square" },
              { value: "Tall", label: "Tall" },
              { value: "Wide", label: "Wide" },
            ]}
            onChange={(v) => setImageFilters((f) => ({ ...f, layout: (v || null) as ImageSearchFilters["layout"] }))}
          />
        </div>
      )}
      {saveMsg && (
        <p className="hint" style={{ padding: "6px 14px 0" }}>
          {saveMsg}
        </p>
      )}
      {error && <p className="error">{error}</p>}
      {loading && <div className="column-empty">Searching…</div>}
      {!loading &&
        searched &&
        mode === "videos" &&
        (provider === "youtube" ? videos.length === 0 : providerVideos.length === 0) &&
        !error && <div className="column-empty">No results.</div>}
      {!loading && searched && mode === "images" && images.length === 0 && !error && (
        <div className="column-empty">No results.</div>
      )}
      {!loading && searched && mode === "books" && books.length === 0 && !error && (
        <div className="column-empty">No results.</div>
      )}
      <div
        ref={resultsRef}
        className="internet-results-wrap"
        tabIndex={0}
        onKeyDown={handleResultsKeyDown}
        onMouseDown={onResultsMouseDown}
      >
        {mode === "videos" && provider === "youtube" && videos.length > 0 && (
          <div className="entries icon internet-results">
            {videos.map((v) => (
              <div
                key={v.id}
                data-key={v.id}
                className={`entry icon ${selected.has(v.id) ? "selected" : ""}`}
                title={v.title}
                onClick={(e) => {
                  handleTileClick(v.id, e);
                  // Touch has no double-click -- a single tap both selects
                  // and opens, same convention as the regular file browser
                  // (confirmed live: these results were unopenable on mobile
                  // before this, since dblclick never fires from a touch tap).
                  if (mobile) osOpen(`https://www.youtube.com/watch?v=${v.id}`).catch(() => {});
                }}
                onDoubleClick={() => openResult(v.id)}
                onContextMenu={(e) => onTileContextMenu(v.id, e)}
              >
                <span className="entry-icon">
                  <img className="internet-thumb" src={v.thumbnail} draggable={false} />
                  {v.duration && <span className="internet-badge">{v.duration}</span>}
                </span>
                <span className="entry-name">
                  <span className="internet-title">{v.title}</span>
                  {v.published && <span className="internet-published">{v.published}</span>}
                </span>
              </div>
            ))}
          </div>
        )}
        {mode === "videos" && provider !== "youtube" && providerVideos.length > 0 && (
          <div className="entries icon internet-results">
            {providerVideos.map((v, i) => (
              <div
                key={i}
                data-key={`p${i}`}
                className={`entry icon ${selected.has(`p${i}`) ? "selected" : ""}`}
                title={v.title}
                onClick={(e) => {
                  handleTileClick(`p${i}`, e);
                  if (!mobile) return;
                  if (provider === "animeflv") setEpisodePicker({ title: v.title, pageUrl: v.page_url });
                  else osOpen(v.page_url).catch(() => {});
                }}
                onDoubleClick={() => openResult(`p${i}`)}
                onContextMenu={(e) => onTileContextMenu(`p${i}`, e)}
              >
                <span className="entry-icon">
                  {v.thumbnail ? (
                    <img className="internet-thumb" src={v.thumbnail} draggable={false} />
                  ) : (
                    <FileIcon entry={{ name: "video.mp4", is_dir: false, size: 0, mtime: 0 }} />
                  )}
                  {v.duration && <span className="internet-badge">{v.duration}</span>}
                </span>
                <span className="entry-name">
                  <span className="internet-title">{v.title}</span>
                </span>
              </div>
            ))}
          </div>
        )}
        {mode === "images" && images.length > 0 && (
          <div className="entries icon internet-results">
            {images.map((img, i) => (
              <div
                key={i}
                data-key={`i${i}`}
                className={`entry icon ${selected.has(`i${i}`) ? "selected" : ""}`}
                title={img.title}
                draggable
                onDragStart={(e) => handleTileDragStart(`i${i}`, e)}
                onClick={(e) => {
                  handleTileClick(`i${i}`, e);
                  if (mobile) osOpen(img.image).catch(() => {});
                }}
                onDoubleClick={() => osOpen(img.image).catch(() => {})}
                onContextMenu={(e) => onTileContextMenu(`i${i}`, e)}
              >
                <img className="internet-thumb" src={img.thumbnail} draggable={false} />
                <span className="entry-name">{img.title}</span>
              </div>
            ))}
          </div>
        )}
        {mode === "books" && books.length > 0 && (
          <div className="entries icon internet-results">
            {books.map((b, i) => (
              <div
                key={i}
                data-key={`b${i}`}
                className={`entry icon ${selected.has(`b${i}`) ? "selected" : ""}`}
                title={b.snippet ? `${b.title}\n${b.snippet}` : b.title}
                draggable
                onDragStart={(e) => handleTileDragStart(`b${i}`, e)}
                onClick={(e) => {
                  handleTileClick(`b${i}`, e);
                  if (mobile) osOpen(b.url).catch(() => {});
                }}
                onDoubleClick={() => osOpen(b.url).catch(() => {})}
                onContextMenu={(e) => onTileContextMenu(`b${i}`, e)}
              >
                <span className="entry-icon">
                  <FileIcon entry={{ name: "book.pdf", is_dir: false, size: 0, mtime: 0 }} />
                </span>
                <span className="entry-name">{b.title}</span>
              </div>
            ))}
          </div>
        )}
      </div>
      {marquee && (
        <div
          className="marquee"
          style={{
            left: Math.min(marquee.x0, marquee.x1),
            top: Math.min(marquee.y0, marquee.y1),
            width: Math.abs(marquee.x1 - marquee.x0),
            height: Math.abs(marquee.y1 - marquee.y0),
          }}
        />
      )}
      <ContextMenu state={contextMenu} onClose={() => setContextMenu(null)} />
      {episodePicker && (
        <AnimeflvEpisodeSheet
          title={episodePicker.title}
          pageUrl={episodePicker.pageUrl}
          onClose={() => setEpisodePicker(null)}
          onPick={(ep: AnimeflvEpisode) => {
            osOpen(ep.page_url).catch(() => {});
            setEpisodePicker(null);
          }}
        />
      )}
    </div>
  );
}
