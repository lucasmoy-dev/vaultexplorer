// gallery.go turns hcshare's directory listing from a bare list of blue links
// into something a relative can actually recognise things in: an icon per
// kind of file, a real thumbnail for photos, and a size and date under each
// name. Read-only browsing is the entire interface a visitor sees, so this is
// not a nice-to-have skin — it is the whole product for whoever opens the
// link.
package main

import (
	"fmt"
	"html"
	"image"
	"image/color"
	_ "image/gif"
	"image/jpeg"
	_ "image/jpeg"
	_ "image/png"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

// How large the longest edge of a generated thumbnail is. Big enough to look
// sharp in a phone-width grid, small enough that a folder of a thousand
// photos does not turn a page load into a thousand full-size downloads.
const thumbMaxSide = 320

// Skips thumbnailing anything heavier than this. A giant image costs real CPU
// and memory to decode just to shrink for a preview nobody is going to
// scrutinise at full resolution anyway; the icon for its kind is shown instead
// and the full file is still one tap away.
const thumbSourceLimit = 32 << 20 // 32 MB

// How many decoded thumbnails are kept in memory. The process lives at most a
// few hours and a family album is a few hundred photos, not tens of
// thousands, so a fixed cap is simpler than real LRU accounting and costs
// nothing in the case that matters.
const thumbCacheLimit = 400

// shareHandler serves `root` read-only: a styled gallery for directories, and
// the plain file underneath for everything else — so Range requests (video
// scrubbing, resuming a download) and conditional GETs keep working exactly
// as they did with the stdlib file server, which this still uses for anything
// that is not a directory listing.
func shareHandler(root string) http.Handler {
	files := http.FileServer(http.Dir(root))
	thumbs := newThumbCache()

	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/.hcshare-thumb" {
			serveThumbnail(w, r, root, thumbs)
			return
		}

		full, safe := resolveWithinRoot(root, r.URL.Path)
		if !safe {
			http.Error(w, "ruta no válida", http.StatusBadRequest)
			return
		}
		if info, err := os.Stat(full); err == nil && info.IsDir() {
			// A folder that happens to hold its own index.html is left to the
			// stdlib server, the same as it always was — this only replaces
			// the bare listing http.FileServer draws when there is nothing
			// else to show.
			if _, err := os.Stat(filepath.Join(full, "index.html")); err != nil {
				renderGallery(w, root, r.URL.Path, full)
				return
			}
		}
		files.ServeHTTP(w, r)
	})
}

// resolveWithinRoot turns a URL path into a filesystem path guaranteed to sit
// inside root. The gallery page and the thumbnail endpoint do their own path
// handling — unlike the plain file server, which already sandboxes this
// itself — so both route through here rather than trusting a client-supplied
// path on faith.
func resolveWithinRoot(root, urlPath string) (string, bool) {
	cleaned := path.Clean("/" + urlPath)
	full := filepath.Join(root, filepath.FromSlash(cleaned))
	rootAbs, err1 := filepath.Abs(root)
	fullAbs, err2 := filepath.Abs(full)
	if err1 != nil || err2 != nil {
		return "", false
	}
	if fullAbs != rootAbs && !strings.HasPrefix(fullAbs, rootAbs+string(filepath.Separator)) {
		return "", false
	}
	return full, true
}

// ---- the gallery page ------------------------------------------------------

type entry struct {
	name    string
	urlPath string
	isDir   bool
	size    int64
	modTime time.Time
	kind    fileKind
	items   int // for a directory: how many things are inside it
}

func renderGallery(w http.ResponseWriter, root, urlPath, fsPath string) {
	rows, err := os.ReadDir(fsPath)
	if err != nil {
		http.Error(w, "no se pudo leer la carpeta", http.StatusInternalServerError)
		return
	}

	entries := make([]entry, 0, len(rows))
	var totalBytes int64
	for _, row := range rows {
		info, err := row.Info()
		if err != nil {
			continue // a file that vanished mid-listing is skipped, not fatal
		}
		child := entry{
			name:    row.Name(),
			urlPath: path.Join(urlPath, url.PathEscape(row.Name())),
			isDir:   row.IsDir(),
			size:    info.Size(),
			modTime: info.ModTime(),
		}
		if child.isDir {
			child.urlPath += "/"
			// One level deep only: a full recursive count would turn a big
			// tree into a slow page load for a number nobody is scanning for.
			if siblings, err := os.ReadDir(filepath.Join(fsPath, row.Name())); err == nil {
				child.items = len(siblings)
			}
		} else {
			child.kind = kindOf(row.Name())
			totalBytes += info.Size()
		}
		entries = append(entries, child)
	}

	sort.Slice(entries, func(i, j int) bool {
		if entries[i].isDir != entries[j].isDir {
			return entries[i].isDir // folders first
		}
		return strings.ToLower(entries[i].name) < strings.ToLower(entries[j].name)
	})

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	fmt.Fprint(w, pageHead(breadcrumbTitle(urlPath)))
	fmt.Fprint(w, breadcrumbs(urlPath))

	folders, files := 0, 0
	for _, e := range entries {
		if e.isDir {
			folders++
		} else {
			files++
		}
	}
	fmt.Fprintf(w, `<p class="summary">%s · solo lectura</p>`, html.EscapeString(summaryLine(folders, files, totalBytes)))

	fmt.Fprint(w, `<div class="grid">`)
	for _, e := range entries {
		fmt.Fprint(w, cardFor(e))
	}
	fmt.Fprint(w, `</div>`)
	fmt.Fprint(w, pageTail())
}

func summaryLine(folders, files int, totalBytes int64) string {
	parts := make([]string, 0, 2)
	if folders > 0 {
		word := "carpeta"
		if folders != 1 {
			word = "carpetas"
		}
		parts = append(parts, fmt.Sprintf("%d %s", folders, word))
	}
	if files > 0 || folders == 0 {
		word := "fichero"
		if files != 1 {
			word = "ficheros"
		}
		parts = append(parts, fmt.Sprintf("%d %s (%s)", files, word, formatBytes(totalBytes)))
	}
	return strings.Join(parts, " · ")
}

func cardFor(e entry) string {
	name := html.EscapeString(e.name)
	href := html.EscapeString(e.urlPath)

	var visual string
	if !e.isDir && e.kind == kindImage {
		thumb := "/.hcshare-thumb?p=" + url.QueryEscape(e.urlPath)
		// The icon sits behind the real thumbnail and only shows if the image
		// fails to decode — a broken or huge file still gets a recognisable
		// card instead of a blank box.
		visual = fmt.Sprintf(
			`<div class="thumb">%s<img loading="lazy" src="%s" onerror="this.remove()"></div>`,
			iconFor(e.kind, e.isDir), thumb,
		)
	} else {
		visual = fmt.Sprintf(`<div class="thumb">%s</div>`, iconFor(e.kind, e.isDir))
	}

	subtitle := formatDate(e.modTime)
	if e.isDir {
		word := "elemento"
		if e.items != 1 {
			word = "elementos"
		}
		subtitle = fmt.Sprintf("%d %s", e.items, word)
	} else {
		subtitle = fmt.Sprintf("%s · %s", formatBytes(e.size), subtitle)
	}

	return fmt.Sprintf(
		`<a class="card" href="%s" title="%s">%s<span class="name">%s</span><span class="sub">%s</span></a>`,
		href, name, visual, name, html.EscapeString(subtitle),
	)
}

func breadcrumbTitle(urlPath string) string {
	segments := strings.FieldsFunc(urlPath, func(r rune) bool { return r == '/' })
	if len(segments) == 0 {
		return "HomeCloud"
	}
	decoded, err := url.PathUnescape(segments[len(segments)-1])
	if err != nil {
		return segments[len(segments)-1]
	}
	return decoded
}

func breadcrumbs(urlPath string) string {
	segments := strings.FieldsFunc(urlPath, func(r rune) bool { return r == '/' })
	var b strings.Builder
	b.WriteString(`<nav class="crumbs">`)
	fmt.Fprint(&b, `<a href="/">HomeCloud</a>`)
	built := ""
	for _, segment := range segments {
		built += "/" + segment
		decoded, err := url.PathUnescape(segment)
		if err != nil {
			decoded = segment
		}
		fmt.Fprintf(&b, ` / <a href="%s/">%s</a>`, html.EscapeString(built), html.EscapeString(decoded))
	}
	b.WriteString(`</nav>`)
	return b.String()
}

func formatBytes(n int64) string {
	const unit = 1000
	if n < unit {
		return fmt.Sprintf("%d B", n)
	}
	div, exp := int64(unit), 0
	for v := n / unit; v >= unit; v /= unit {
		div *= unit
		exp++
	}
	return fmt.Sprintf("%.1f %cB", float64(n)/float64(div), "kMGTPE"[exp])
}

func formatDate(t time.Time) string {
	return t.Local().Format("2 Jan 2006")
}

// ---- file kinds and their icons --------------------------------------------

type fileKind int

const (
	kindGeneric fileKind = iota
	kindImage
	kindVideo
	kindAudio
	kindPDF
	kindDoc
	kindSheet
	kindArchive
)

var kindByExt = map[string]fileKind{
	".jpg": kindImage, ".jpeg": kindImage, ".png": kindImage, ".gif": kindImage,
	".webp": kindImage, ".bmp": kindImage, ".heic": kindImage, ".heif": kindImage,
	".mp4": kindVideo, ".mov": kindVideo, ".avi": kindVideo, ".mkv": kindVideo,
	".webm": kindVideo, ".m4v": kindVideo,
	".mp3": kindAudio, ".wav": kindAudio, ".flac": kindAudio, ".m4a": kindAudio, ".ogg": kindAudio,
	".pdf": kindPDF,
	".doc": kindDoc, ".docx": kindDoc, ".odt": kindDoc, ".txt": kindDoc, ".md": kindDoc,
	".xls": kindSheet, ".xlsx": kindSheet, ".csv": kindSheet, ".ods": kindSheet,
	".zip": kindArchive, ".rar": kindArchive, ".7z": kindArchive, ".tar": kindArchive, ".gz": kindArchive,
}

func kindOf(name string) fileKind {
	return kindByExt[strings.ToLower(filepath.Ext(name))]
}

// One inline SVG per kind, monoline and single-colour so it reads the same in
// light and dark. Folders get their own glyph regardless of the map above.
func iconFor(kind fileKind, isDir bool) string {
	if isDir {
		return svgIcon(`<path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l1.8 2.2h7.2A1.5 1.5 0 0 1 19 9.7v7.8a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 17.5z"/>`)
	}
	switch kind {
	case kindImage:
		return svgIcon(`<rect x="3.5" y="4.5" width="17" height="15" rx="1.6"/><circle cx="9" cy="10" r="1.6"/><path d="M4 17l5-5 3.5 3.5L16 12l4.5 5"/>`)
	case kindVideo:
		return svgIcon(`<rect x="3.5" y="5.5" width="13" height="13" rx="1.6"/><path d="M16.5 10.5l4-2.5v8l-4-2.5"/>`)
	case kindAudio:
		return svgIcon(`<circle cx="8" cy="17" r="2.3"/><circle cx="17" cy="15" r="2.3"/><path d="M10.3 17V6l9-1.6v10.6"/>`)
	case kindPDF:
		return svgIcon(`<path d="M6.5 3.5h7l4 4v13h-11z"/><path d="M13.5 3.5v4h4"/><path d="M8.5 13h1.6a1.4 1.4 0 0 1 0 2.8H8.5zM8.5 15.8v2.2M13 13v5M13 15.3h1.7"/>`)
	case kindDoc:
		return svgIcon(`<path d="M6.5 3.5h7l4 4v13h-11z"/><path d="M13.5 3.5v4h4"/><path d="M8.5 12.5h7M8.5 15h7M8.5 17.5h4.5"/>`)
	case kindSheet:
		return svgIcon(`<rect x="4" y="4" width="16" height="16" rx="1.6"/><path d="M4 9.5h16M4 14.5h16M10 4v16M15 4v16"/>`)
	case kindArchive:
		return svgIcon(`<rect x="4.5" y="4.5" width="15" height="15" rx="1.6"/><path d="M12 4.5v3M12 9.5v2M12 13.5v2"/><circle cx="12" cy="16.3" r="1.1"/>`)
	default:
		return svgIcon(`<path d="M6.5 3.5h7l4 4v13h-11z"/><path d="M13.5 3.5v4h4"/>`)
	}
}

func svgIcon(paths string) string {
	return `<svg viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" ` +
		`stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">` + paths + `</svg>`
}

// ---- thumbnails -------------------------------------------------------------

type cachedThumb struct {
	jpeg []byte
	key  string // path + size + modtime: changes invalidate the cache entry
}

type thumbCache struct {
	mu    sync.Mutex
	byURL map[string]cachedThumb
	order []string // insertion order, for a simple FIFO eviction
}

func newThumbCache() *thumbCache {
	return &thumbCache{byURL: make(map[string]cachedThumb)}
}

func (c *thumbCache) get(urlPath, key string) ([]byte, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	cached, ok := c.byURL[urlPath]
	if !ok || cached.key != key {
		return nil, false
	}
	return cached.jpeg, true
}

func (c *thumbCache) put(urlPath, key string, jpegBytes []byte) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if _, exists := c.byURL[urlPath]; !exists {
		c.order = append(c.order, urlPath)
		if len(c.order) > thumbCacheLimit {
			oldest := c.order[0]
			c.order = c.order[1:]
			delete(c.byURL, oldest)
		}
	}
	c.byURL[urlPath] = cachedThumb{jpeg: jpegBytes, key: key}
}

func serveThumbnail(w http.ResponseWriter, r *http.Request, root string, cache *thumbCache) {
	requested := r.URL.Query().Get("p")
	full, safe := resolveWithinRoot(root, requested)
	if !safe {
		http.Error(w, "ruta no válida", http.StatusBadRequest)
		return
	}
	info, err := os.Stat(full)
	if err != nil || info.IsDir() || info.Size() > thumbSourceLimit {
		http.NotFound(w, r)
		return
	}

	key := fmt.Sprintf("%d-%d", info.Size(), info.ModTime().UnixNano())
	if cached, ok := cache.get(requested, key); ok {
		writeThumb(w, cached)
		return
	}

	source, err := os.Open(full)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	defer source.Close()

	decoded, _, err := image.Decode(source)
	if err != nil {
		// Not a format this process can decode (HEIC, a corrupt file, an
		// image type nobody bothered registering). The card's icon is still
		// there; nothing about the gallery breaks.
		http.NotFound(w, r)
		return
	}

	small := resizeToFit(decoded, thumbMaxSide)
	var buf strings.Builder
	if err := jpeg.Encode(&byteWriter{&buf}, small, &jpeg.Options{Quality: 78}); err != nil {
		http.NotFound(w, r)
		return
	}
	result := []byte(buf.String())
	cache.put(requested, key, result)
	writeThumb(w, result)
}

func writeThumb(w http.ResponseWriter, jpegBytes []byte) {
	w.Header().Set("Content-Type", "image/jpeg")
	w.Header().Set("Cache-Control", "private, max-age=3600")
	w.Write(jpegBytes)
}

// resizeToFit shrinks an image so its longest side is at most maxSide,
// averaging each destination pixel over the block of source pixels it
// covers. A dependency-free stand-in for a real resampling library: plainer
// than Lanczos, but a 320px preview does not need to survive a print.
func resizeToFit(src image.Image, maxSide int) image.Image {
	bounds := src.Bounds()
	w, h := bounds.Dx(), bounds.Dy()
	if w <= 0 || h <= 0 || (w <= maxSide && h <= maxSide) {
		return src
	}

	scale := float64(maxSide) / float64(w)
	if h > w {
		scale = float64(maxSide) / float64(h)
	}
	dw := max(1, int(float64(w)*scale))
	dh := max(1, int(float64(h)*scale))

	dst := image.NewRGBA(image.Rect(0, 0, dw, dh))
	for y := 0; y < dh; y++ {
		sy0, sy1 := y*h/dh, (y+1)*h/dh
		if sy1 <= sy0 {
			sy1 = sy0 + 1
		}
		for x := 0; x < dw; x++ {
			sx0, sx1 := x*w/dw, (x+1)*w/dw
			if sx1 <= sx0 {
				sx1 = sx0 + 1
			}
			var r, g, b, a, n uint64
			for sy := sy0; sy < sy1 && sy < h; sy++ {
				for sx := sx0; sx < sx1 && sx < w; sx++ {
					pr, pg, pb, pa := src.At(bounds.Min.X+sx, bounds.Min.Y+sy).RGBA()
					r += uint64(pr)
					g += uint64(pg)
					b += uint64(pb)
					a += uint64(pa)
					n++
				}
			}
			if n == 0 {
				n = 1
			}
			dst.Set(x, y, color.RGBA64{
				R: uint16(r / n), G: uint16(g / n), B: uint16(b / n), A: uint16(a / n),
			})
		}
	}
	return dst
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}

// byteWriter adapts a strings.Builder to io.Writer for jpeg.Encode without
// pulling in bytes.Buffer just for this.
type byteWriter struct{ b *strings.Builder }

func (w *byteWriter) Write(p []byte) (int, error) { return w.b.Write(p) }

// ---- the page shell ---------------------------------------------------------

func pageHead(title string) string {
	return fmt.Sprintf(`<!doctype html>
<html lang="es">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>%s</title>
<link rel="icon" href="data:,">
<style>
:root { color-scheme: light dark; --bg:#f4f5f7; --card:#ffffff; --line:#e2e4e8; --text:#1c1e21; --muted:#6b7280; --accent:#3b82f6; }
@media (prefers-color-scheme: dark) { :root { --bg:#17181b; --card:#212226; --line:#2e3036; --text:#eef0f2; --muted:#9aa0a8; --accent:#5b9bff; } }
* { box-sizing: border-box; }
body { margin:0; padding:16px; background:var(--bg); color:var(--text); font:15px/1.4 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif; }
.crumbs { margin:0 0 6px; font-size:13px; color:var(--muted); overflow-wrap:anywhere; }
.crumbs a { color:var(--muted); text-decoration:none; }
.crumbs a:last-child { color:var(--text); font-weight:600; }
.summary { margin:0 0 16px; font-size:13px; color:var(--muted); }
.grid { display:grid; grid-template-columns:repeat(auto-fill, minmax(136px, 1fr)); gap:12px; }
.card { display:flex; flex-direction:column; gap:6px; padding:8px; border:1px solid var(--line); border-radius:12px; background:var(--card); text-decoration:none; color:var(--text); overflow:hidden; }
.thumb { position:relative; aspect-ratio:1/1; border-radius:8px; background:var(--bg); display:flex; align-items:center; justify-content:center; color:var(--muted); overflow:hidden; }
.thumb img { position:absolute; inset:0; width:100%%; height:100%%; object-fit:cover; }
.name { font-size:13px; font-weight:500; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.sub { font-size:11.5px; color:var(--muted); }
</style>
</head>
<body>`, html.EscapeString(title))
}

func pageTail() string {
	return `</body></html>`
}
