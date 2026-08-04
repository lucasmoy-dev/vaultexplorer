import { useState } from "react";
import { openPath as osOpen } from "@tauri-apps/plugin-opener";
import { api, YoutubeResult, ImageResult } from "../api";
import { ChevronLeft, SearchGlyph } from "../icons";

type Mode = "root" | "videos" | "images";

// Desktop-only experiment: a sidebar entry that behaves like a folder but
// isn't backed by any real filesystem path -- "Videos" and "Images" inside
// it are YouTube/web-image search results wearing file icons, not
// anything downloaded or stored anywhere. Deliberately self-contained
// (its own mode/query/results state, no `Loc`, no fs/vault commands) so
// this stays a single component to delete if the experiment doesn't pan
// out, rather than something threaded through the app's real navigation
// model.
export function InternetView() {
  const [mode, setMode] = useState<Mode>("root");
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [videos, setVideos] = useState<YoutubeResult[]>([]);
  const [images, setImages] = useState<ImageResult[]>([]);
  const [searched, setSearched] = useState(false);

  async function runSearch() {
    if (!query.trim()) return;
    setLoading(true);
    setError("");
    setSearched(true);
    try {
      if (mode === "videos") setVideos(await api.searchYoutube(query));
      else if (mode === "images") setImages(await api.searchImages(query));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  function enterFolder(next: Mode) {
    setMode(next);
    setQuery("");
    setSearched(false);
    setError("");
  }

  if (mode === "root") {
    return (
      <div className="internet-view">
        <div className="entries icon">
          <div className="entry icon" onDoubleClick={() => enterFolder("videos")}>
            <span className="entry-icon">📹</span>
            <span className="entry-name">Videos</span>
          </div>
          <div className="entry icon" onDoubleClick={() => enterFolder("images")}>
            <span className="entry-icon">🖼️</span>
            <span className="entry-name">Images</span>
          </div>
        </div>
        <p className="hint" style={{ padding: "0 14px" }}>
          Experimental: "files" in here are YouTube/web search results, fetched live -- nothing is
          downloaded or stored until you open one.
        </p>
      </div>
    );
  }

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
            placeholder={mode === "videos" ? "Search YouTube…" : "Search images…"}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && runSearch()}
          />
        </div>
        <button className="tool-btn wide-btn" onClick={runSearch} disabled={loading}>
          Search
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {loading && <div className="column-empty">Searching…</div>}
      {!loading && searched && mode === "videos" && videos.length === 0 && !error && (
        <div className="column-empty">No results.</div>
      )}
      {!loading && searched && mode === "images" && images.length === 0 && !error && (
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
              <img className="internet-thumb" src={v.thumbnail} draggable={false} />
              <span className="entry-name">{v.title}</span>
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
    </div>
  );
}
