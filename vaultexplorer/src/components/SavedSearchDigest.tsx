import { useEffect, useState } from "react";
import { api, osOpen, Entry, YoutubeResult, ImageResult, BookResult, joinPath } from "../api";
import { FileIcon } from "../icons";
import { SavedInternetSearch } from "./InternetView";

type Section = {
  entry: Entry;
  loading: boolean;
  error: string;
  videos?: YoutubeResult[];
  images?: ImageResult[];
  books?: BookResult[];
};

// Auto-shown instead of the normal file view (see App.tsx) when a real
// folder holds nothing but .ytsearch/.imgsearch/.booksearch files -- each
// one reruns its saved search live, capped to 5 results, under a heading
// of its own filename. Meant for a "channel" of saved searches (e.g. one
// .ytsearch per topic) read at a glance rather than opened one at a time.
export function SavedSearchDigest({
  dir,
  entries,
  ext,
  onDismiss,
  onOpenFile,
}: {
  dir: string;
  entries: Entry[];
  ext: "ytsearch" | "imgsearch" | "booksearch";
  onDismiss: () => void;
  onOpenFile: (entry: Entry) => void;
}) {
  const [sections, setSections] = useState<Section[]>([]);

  useEffect(() => {
    let cancelled = false;
    setSections(entries.map((entry) => ({ entry, loading: true, error: "" })));
    entries.forEach(async (entry, i) => {
      try {
        const saved = JSON.parse(await api.fsReadText(joinPath(dir, entry.name))) as SavedInternetSearch;
        if (saved.kind === "videos") {
          const videos = (await api.searchYoutube(saved.query, saved.filters)).slice(0, 5);
          if (!cancelled) {
            setSections((prev) => prev.map((s, j) => (j === i ? { ...s, loading: false, videos } : s)));
          }
        } else if (saved.kind === "images") {
          const images = (await api.searchImages(saved.query)).slice(0, 5);
          if (!cancelled) {
            setSections((prev) => prev.map((s, j) => (j === i ? { ...s, loading: false, images } : s)));
          }
        } else {
          const books = (await api.searchBooks(saved.query)).slice(0, 5);
          if (!cancelled) {
            setSections((prev) => prev.map((s, j) => (j === i ? { ...s, loading: false, books } : s)));
          }
        }
      } catch (e) {
        if (!cancelled) {
          setSections((prev) => prev.map((s, j) => (j === i ? { ...s, loading: false, error: String(e) } : s)));
        }
      }
    });
    return () => {
      cancelled = true;
    };
    // Re-runs only when the folder or its saved-search filenames actually
    // change, not on every unrelated re-render or background poll refresh.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dir, entries.map((e) => e.name).join("\0")]);

  return (
    <div className="digest-view">
      <div className="digest-header">
        <span className="hint">Folder of saved searches -- showing live previews of each.</span>
        <button className="btn-plain small" onClick={onDismiss}>
          View as files
        </button>
      </div>
      {sections.map((s) => (
        <div className="digest-section" key={s.entry.name}>
          <div className="digest-section-title" onDoubleClick={() => onOpenFile(s.entry)} title="Double-click to open the full search">
            {s.entry.name.replace(/\.(ytsearch|imgsearch|booksearch)$/i, "")}
          </div>
          {s.loading && <div className="column-empty">Loading…</div>}
          {s.error && <p className="error">{s.error}</p>}
          {!s.loading && !s.error && (
            <div className="entries icon digest-strip">
              {ext === "ytsearch" &&
                s.videos?.map((v) => (
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
              {ext === "imgsearch" &&
                s.images?.map((img, i) => (
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
              {ext === "booksearch" &&
                s.books?.map((b, i) => (
                  <div key={i} className="entry icon" title={b.title} onDoubleClick={() => osOpen(b.url).catch(() => {})}>
                    <span className="entry-icon">
                      <FileIcon entry={{ name: "book.pdf", is_dir: false, size: 0, mtime: 0 }} />
                    </span>
                    <span className="entry-name">{b.title}</span>
                  </div>
                ))}
              {((ext === "ytsearch" && s.videos?.length === 0) ||
                (ext === "imgsearch" && s.images?.length === 0) ||
                (ext === "booksearch" && s.books?.length === 0)) && (
                <div className="column-empty">No results.</div>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
