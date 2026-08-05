import { useEffect, useState } from "react";
import { openPath as osOpen } from "@tauri-apps/plugin-opener";
import { api, YoutubeResult, ImageResult, BookResult, YoutubeSearchFilters } from "../api";
import { ChevronLeft, SearchGlyph, SaveGlyph, FileIcon } from "../icons";
import folderVideosIcon from "../assets/foldericons/folder-videos.svg";
import folderImagesIcon from "../assets/foldericons/folder-images.svg";
import folderBookIcon from "../assets/foldericons/folder-book.svg";

type Mode = "root" | "videos" | "images" | "books";

const DEFAULT_FILTERS: YoutubeSearchFilters = { sortByDate: false, uploadDate: null, duration: null };

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
}: {
  initial: SavedInternetSearch | null;
  onSave: (filename: string, content: string) => Promise<string>;
}) {
  const [mode, setMode] = useState<Mode>("root");
  const [query, setQuery] = useState("");
  const [filters, setFilters] = useState<YoutubeSearchFilters>(DEFAULT_FILTERS);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [videos, setVideos] = useState<YoutubeResult[]>([]);
  const [images, setImages] = useState<ImageResult[]>([]);
  const [books, setBooks] = useState<BookResult[]>([]);
  const [searched, setSearched] = useState(false);
  const [saveMsg, setSaveMsg] = useState("");
  // Click-to-select on a result tile, purely visual (no selection-driven
  // actions yet, same as the rest of this experiment) -- but without it,
  // clicking a tile before double-clicking gave no feedback at all, unlike
  // every real file entry elsewhere in the app.
  const [selectedKey, setSelectedKey] = useState<string | null>(null);

  async function runSearch(q: string, f: YoutubeSearchFilters, m: Mode) {
    if (!q.trim()) return;
    setLoading(true);
    setError("");
    setSearched(true);
    setSelectedKey(null);
    try {
      if (m === "videos") setVideos(await api.searchYoutube(q, f));
      else if (m === "images") setImages(await api.searchImages(q));
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
    setSearched(false);
    setError("");
    setSaveMsg("");
    setSelectedKey(null);
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

  const placeholder =
    mode === "videos" ? "Search YouTube…" : mode === "images" ? "Search images…" : "Search for a PDF…";

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
          <select
            className="settings-select"
            value={filters.uploadDate ?? ""}
            onChange={(e) =>
              setFilters((f) => ({ ...f, uploadDate: e.target.value ? (Number(e.target.value) as 1 | 2 | 3 | 4 | 5) : null }))
            }
          >
            <option value="">Any time</option>
            <option value="2">Today</option>
            <option value="3">This week</option>
            <option value="4">This month</option>
            <option value="5">This year</option>
          </select>
          <select
            className="settings-select"
            value={filters.duration ?? ""}
            onChange={(e) =>
              setFilters((f) => ({ ...f, duration: e.target.value ? (Number(e.target.value) as 1 | 2 | 3) : null }))
            }
          >
            <option value="">Any length</option>
            <option value="1">Under 4 minutes</option>
            <option value="3">4-20 minutes</option>
            <option value="2">Over 20 minutes</option>
          </select>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={filters.sortByDate}
              onChange={(e) => setFilters((f) => ({ ...f, sortByDate: e.target.checked }))}
            />
            Newest first
          </label>
        </div>
      )}
      {saveMsg && (
        <p className="hint" style={{ padding: "6px 14px 0" }}>
          {saveMsg}
        </p>
      )}
      {error && <p className="error">{error}</p>}
      {loading && <div className="column-empty">Searching…</div>}
      {!loading && searched && mode === "videos" && videos.length === 0 && !error && (
        <div className="column-empty">No results.</div>
      )}
      {!loading && searched && mode === "images" && images.length === 0 && !error && (
        <div className="column-empty">No results.</div>
      )}
      {!loading && searched && mode === "books" && books.length === 0 && !error && (
        <div className="column-empty">No results.</div>
      )}
      {mode === "videos" && videos.length > 0 && (
        <div className="entries icon internet-results">
          {videos.map((v) => (
            <div
              key={v.id}
              className={`entry icon ${selectedKey === v.id ? "selected" : ""}`}
              title={v.title}
              onClick={() => setSelectedKey(v.id)}
              onDoubleClick={() => osOpen(`https://www.youtube.com/watch?v=${v.id}`).catch(() => {})}
            >
              <span className="entry-icon">
                <img className="internet-thumb" src={v.thumbnail} draggable={false} />
                {v.duration && <span className="internet-badge">{v.duration}</span>}
              </span>
              <span className="entry-name">
                {v.title}
                {v.published && <span className="internet-published">{v.published}</span>}
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
              className={`entry icon ${selectedKey === `i${i}` ? "selected" : ""}`}
              title={img.title}
              onClick={() => setSelectedKey(`i${i}`)}
              onDoubleClick={() => osOpen(img.image).catch(() => {})}
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
              className={`entry icon ${selectedKey === `b${i}` ? "selected" : ""}`}
              title={b.snippet ? `${b.title}\n${b.snippet}` : b.title}
              onClick={() => setSelectedKey(`b${i}`)}
              onDoubleClick={() => osOpen(b.url).catch(() => {})}
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
  );
}
