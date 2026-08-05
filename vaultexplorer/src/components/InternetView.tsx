import { useEffect, useState } from "react";
import { openPath as osOpen } from "@tauri-apps/plugin-opener";
import {
  api,
  YoutubeResult,
  ImageResult,
  BookResult,
  YoutubeSearchFilters,
  TorrentProvider,
  TorrentSearchResult,
  TorrentFile,
  formatSize,
} from "../api";
import { ChevronLeft, SearchGlyph, SaveGlyph, FileIcon } from "../icons";
import folderVideosIcon from "../assets/foldericons/folder-videos.svg";
import folderImagesIcon from "../assets/foldericons/folder-images.svg";
import folderBookIcon from "../assets/foldericons/folder-book.svg";
import folderDownloadIcon from "../assets/foldericons/folder-download.svg";

type Mode = "root" | "videos" | "images" | "books" | "torrents";

const DEFAULT_FILTERS: YoutubeSearchFilters = { sortByDate: false, uploadDate: null, duration: null };

// A saved search's whole state, round-tripped through a `.ytsearch`/
// `.imgsearch`/`.booksearch` file's JSON content (see `activate()` in
// App.tsx for the open side, and `saveSearch` below for the write side) --
// deliberately small/flat so the file stays human-readable if someone
// opens it in a text editor. Torrents aren't saveable this way (yet) --
// a torrent result is itself a folder of files, not a single thing to
// reopen.
export type SavedInternetSearch =
  | { kind: "videos"; query: string; filters: YoutubeSearchFilters }
  | { kind: "images"; query: string }
  | { kind: "books"; query: string };

// Desktop-only experiment: a sidebar entry that behaves like a folder but
// isn't backed by any real filesystem path -- "Videos"/"Images"/"Books"/
// "Torrents" inside it are live search results wearing file icons, not
// anything downloaded or stored anywhere. A search can be saved to a real
// file though (see SaveGlyph button below) -- that file IS a normal
// filesystem citizen, movable/copyable anywhere, and reopening it comes
// back here via `initial` instead of this component inventing its own
// separate storage.
export function InternetView({
  initial,
  onSave,
  onDownloadTorrentFile,
}: {
  initial: SavedInternetSearch | null;
  onSave: (filename: string, content: string) => Promise<string>;
  onDownloadTorrentFile: (sourceUrl: string, fileIndex: number, fileName: string) => void;
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

  // ---- Torrents (see torrents.rs) ----
  const [providers, setProviders] = useState<TorrentProvider[]>([]);
  const [providerId, setProviderId] = useState("internet_archive");
  const [torrentResults, setTorrentResults] = useState<TorrentSearchResult[]>([]);
  const [showAddProvider, setShowAddProvider] = useState(false);
  const [newProviderName, setNewProviderName] = useState("");
  const [newProviderUrl, setNewProviderUrl] = useState("");
  const [openTorrent, setOpenTorrent] = useState<TorrentSearchResult | null>(null);
  const [torrentFiles, setTorrentFiles] = useState<TorrentFile[]>([]);

  useEffect(() => {
    if (mode === "torrents") api.torrentProvidersList().then(setProviders).catch(() => {});
  }, [mode]);

  async function runSearch(q: string, f: YoutubeSearchFilters, m: Mode) {
    if (!q.trim()) return;
    setLoading(true);
    setError("");
    setSearched(true);
    try {
      if (m === "videos") setVideos(await api.searchYoutube(q, f));
      else if (m === "images") setImages(await api.searchImages(q));
      else if (m === "books") setBooks(await api.searchBooks(q));
      else if (m === "torrents") setTorrentResults(await api.torrentSearch(providerId, q));
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
    setOpenTorrent(null);
    setTorrentFiles([]);
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

  async function openTorrentResult(t: TorrentSearchResult) {
    setOpenTorrent(t);
    setTorrentFiles([]);
    setLoading(true);
    setError("");
    try {
      setTorrentFiles(await api.torrentListFiles(t.source_url));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function playTorrentFile(sourceUrl: string, file: TorrentFile) {
    try {
      const url = await api.torrentStreamUrl(sourceUrl, file.index);
      await osOpen(url);
    } catch (e) {
      setError(String(e));
    }
  }

  async function addProvider() {
    if (!newProviderName.trim() || !newProviderUrl.includes("{query}")) return;
    try {
      setProviders(await api.torrentProviderAdd(newProviderName.trim(), newProviderUrl.trim()));
      setNewProviderName("");
      setNewProviderUrl("");
      setShowAddProvider(false);
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeProvider(id: string) {
    setProviders(await api.torrentProviderRemove(id));
    if (providerId === id) setProviderId("internet_archive");
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
          <div className="entry icon" onDoubleClick={() => enterFolder("torrents")}>
            <span className="entry-icon">
              <img className="fileicon-img" src={folderDownloadIcon} alt="" draggable={false} />
            </span>
            <span className="entry-name">Torrents</span>
          </div>
        </div>
        <p className="hint" style={{ padding: "0 14px" }}>
          Experimental: "files" in here are YouTube/web-image/Internet-Archive/torrent search results,
          fetched live -- nothing is downloaded or stored until you open one. Save a search (once you've
          run one) to drop a real file anywhere in your filesystem that reruns it on double-click.
        </p>
      </div>
    );
  }

  // ---- Torrents: browsing inside one result's file list ----
  if (mode === "torrents" && openTorrent) {
    return (
      <div className="internet-view">
        <div className="internet-search-bar">
          <button className="tool-btn" onClick={() => setOpenTorrent(null)} aria-label="Back">
            <ChevronLeft size={16} />
          </button>
          <span className="entry-name" style={{ fontWeight: 600 }}>
            {openTorrent.title}
          </span>
        </div>
        {error && <p className="error">{error}</p>}
        {loading && <div className="column-empty">Loading file list…</div>}
        {!loading && torrentFiles.length === 0 && !error && (
          <div className="column-empty">No files found.</div>
        )}
        {!loading && torrentFiles.length > 0 && (
          <div className="entries icon internet-results">
            {torrentFiles.map((f) => (
              <div
                key={f.index}
                className="entry icon"
                title={`${f.name} -- ${formatSize(f.length)}`}
                onDoubleClick={() => playTorrentFile(openTorrent.source_url, f)}
              >
                <span className="entry-icon">
                  <FileIcon entry={{ name: f.name, is_dir: false, size: f.length, mtime: 0 }} />
                </span>
                <span className="entry-name">
                  {f.name.split("/").pop()}
                  <span className="internet-published">{formatSize(f.length)}</span>
                </span>
                <button
                  className="tool-btn internet-download-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDownloadTorrentFile(openTorrent.source_url, f.index, f.name.split("/").pop() || f.name);
                  }}
                  title="Download to the current folder"
                  aria-label="Download"
                >
                  <SaveGlyph size={13} />
                </button>
              </div>
            ))}
          </div>
        )}
        <p className="hint" style={{ padding: "6px 14px" }}>
          Double-click plays it directly (streams while it downloads). The download icon saves it into
          the folder you had open, showing up in Tasks like any other download.
        </p>
      </div>
    );
  }

  const placeholder =
    mode === "videos"
      ? "Search YouTube…"
      : mode === "images"
      ? "Search images…"
      : mode === "torrents"
      ? "Search torrents…"
      : "Search books…";

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
        {mode !== "torrents" && (
          <button
            className="tool-btn"
            onClick={saveSearch}
            disabled={!query.trim()}
            aria-label="Save search"
            title="Save this search as a file"
          >
            <SaveGlyph size={16} />
          </button>
        )}
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
          <label className="internet-sort-toggle">
            <input
              type="checkbox"
              checked={filters.sortByDate}
              onChange={(e) => setFilters((f) => ({ ...f, sortByDate: e.target.checked }))}
            />
            Newest first
          </label>
        </div>
      )}
      {mode === "torrents" && (
        <div className="internet-filters">
          <select className="settings-select" value={providerId} onChange={(e) => setProviderId(e.target.value)}>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <button className="btn-plain small" onClick={() => setShowAddProvider((s) => !s)}>
            {showAddProvider ? "Cancel" : "+ Add provider"}
          </button>
          {!providers.find((p) => p.id === providerId)?.builtin && (
            <button className="btn-plain small" onClick={() => removeProvider(providerId)}>
              Remove this provider
            </button>
          )}
        </div>
      )}
      {showAddProvider && (
        <div className="internet-filters">
          <input
            placeholder="Provider name"
            value={newProviderName}
            onChange={(e) => setNewProviderName(e.target.value)}
            style={{ width: 140 }}
          />
          <input
            placeholder="Search URL with {query} in it"
            value={newProviderUrl}
            onChange={(e) => setNewProviderUrl(e.target.value)}
            style={{ flex: 1 }}
          />
          <button className="btn-primary small" onClick={addProvider}>
            Save
          </button>
        </div>
      )}
      {mode === "torrents" && (
        <p className="hint" style={{ padding: "6px 14px 0" }}>
          Internet Archive only searches public-domain/openly-licensed content. A custom provider is any
          page whose search results contain magnet links -- same idea as a browser's custom search
          engines; what it points to is on you, same as any real torrent client's search plugins.
        </p>
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
      {!loading && searched && mode === "torrents" && torrentResults.length === 0 && !error && (
        <div className="column-empty">No results.</div>
      )}
      {mode === "videos" && videos.length > 0 && (
        <div className="entries icon internet-results">
          {videos.map((v) => (
            <div
              key={v.id}
              className="entry icon"
              title={v.title}
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
              className="entry icon"
              title={img.title}
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
          {books.map((b) => (
            <div
              key={b.identifier}
              className="entry icon"
              title={b.title}
              onDoubleClick={() => osOpen(b.details_url).catch(() => {})}
            >
              <img className="internet-thumb" src={b.thumbnail} draggable={false} />
              <span className="entry-name">
                {b.title}
                {(b.creator || b.year) && (
                  <span className="internet-published">
                    {[b.creator, b.year].filter(Boolean).join(" -- ")}
                  </span>
                )}
              </span>
            </div>
          ))}
        </div>
      )}
      {mode === "torrents" && torrentResults.length > 0 && (
        <div className="entries icon internet-results">
          {torrentResults.map((t, i) => (
            <div key={i} className="entry icon" title={t.title} onDoubleClick={() => openTorrentResult(t)}>
              <span className="entry-icon">
                <img className="fileicon-img" src={folderDownloadIcon} alt="" draggable={false} />
              </span>
              <span className="entry-name">{t.title}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
