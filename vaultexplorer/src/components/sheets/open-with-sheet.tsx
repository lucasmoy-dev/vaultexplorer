import { useEffect, useMemo, useRef, useState } from "react";
import { api, baseName } from "../../api";

type AppChoice = { id: string; name: string; comment: string | null; icon_name: string | null };

// How many rows get real icons resolved. The list itself is unbounded (you
// can scroll/filter to any app); only icon resolution is capped, since each
// one is a GTK icon-theme lookup + file read on the main thread.
const ICON_BATCH = 60;

// "Other Application…" -- pick any installed app to open one file with,
// instead of only the handlers the desktop registered for its MIME type
// (which is all the `Open With…` submenu itself can offer). Type-to-filter
// because a machine has a few hundred apps and scanning that as a menu is
// hopeless.
export function OpenWithSheet({
  path,
  onClose,
  onError,
}: {
  path: string;
  onClose: () => void;
  onError: (msg: string) => void;
}) {
  const [apps, setApps] = useState<AppChoice[] | null>(null);
  const [query, setQuery] = useState("");
  const [icons, setIcons] = useState<Record<string, string>>({});
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api
      .listAllApps()
      .then(setApps)
      .catch((e) => {
        onError(String(e));
        setApps([]);
      });
    inputRef.current?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!apps) return [];
    if (!q) return apps;
    // Name matches rank above description matches -- typing "gimp" should
    // put GIMP first even though a dozen apps mention it in their Comment.
    const byName = apps.filter((a) => a.name.toLowerCase().includes(q));
    const byOther = apps.filter(
      (a) =>
        !a.name.toLowerCase().includes(q) &&
        ((a.comment ?? "").toLowerCase().includes(q) || a.id.toLowerCase().includes(q))
    );
    return [...byName, ...byOther];
  }, [apps, query]);

  useEffect(() => {
    setActive(0);
  }, [query]);

  // Resolve icons for the visible slice of the current filter, skipping any
  // already fetched (results accumulate, so re-filtering is mostly free).
  useEffect(() => {
    const wanted = filtered
      .slice(0, ICON_BATCH)
      .map((a) => a.icon_name)
      .filter((n): n is string => !!n && !(n in icons));
    const unique = [...new Set(wanted)];
    if (unique.length === 0) return;
    let cancelled = false;
    api
      .appIcons(unique)
      .then((uris) => {
        if (cancelled) return;
        setIcons((prev) => {
          const next = { ...prev };
          unique.forEach((name, i) => {
            // Store the miss too ("" = tried, nothing there) so a name that
            // doesn't resolve isn't re-requested on every keystroke.
            next[name] = uris[i] ?? "";
          });
          return next;
        });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filtered]);

  function launch(app: AppChoice) {
    api.openWith(path, app.id).catch((e) => onError(String(e)));
    onClose();
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => {
        const next = Math.min(filtered.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)));
        listRef.current
          ?.querySelectorAll(".app-choice")
          [next]?.scrollIntoView({ block: "nearest" });
        return next;
      });
    } else if (e.key === "Enter") {
      e.preventDefault();
      const app = filtered[active];
      if (app) launch(app);
    } else if (e.key === "Escape") {
      onClose();
    }
  }

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div
        className="sheet-card open-with-card"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <h3>Open With</h3>
        <p className="open-with-file">{baseName(path)}</p>
        <input
          ref={inputRef}
          className="open-with-search"
          placeholder="Search applications…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <div className="app-choice-list" ref={listRef}>
          {apps === null ? (
            <div className="app-choice-empty">Loading applications…</div>
          ) : filtered.length === 0 ? (
            <div className="app-choice-empty">No application matches “{query}”</div>
          ) : (
            filtered.map((a, i) => {
              const uri = a.icon_name ? icons[a.icon_name] : "";
              return (
                <div
                  key={a.id}
                  className={`app-choice ${i === active ? "active" : ""}`}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => launch(a)}
                >
                  <span className="app-choice-icon">
                    {uri ? <img src={uri} alt="" /> : <span className="app-choice-fallback" />}
                  </span>
                  <span className="app-choice-text">
                    <span className="app-choice-name">{a.name}</span>
                    {a.comment && <span className="app-choice-comment">{a.comment}</span>}
                  </span>
                </div>
              );
            })
          )}
        </div>
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn-primary"
            disabled={!filtered[active]}
            onClick={() => filtered[active] && launch(filtered[active])}
          >
            Open
          </button>
        </div>
      </div>
    </div>
  );
}
