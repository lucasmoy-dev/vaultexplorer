import { useEffect, useState } from "react";
import { api, AnimeflvEpisode } from "../../api";

// AnimeFLV's own player can't be scraped (see webfind.rs's
// resolve_provider_playable) -- this picker is the honest middle ground
// between "opens the show's front page" and "opens directly in a
// browser with zero context": pick the actual episode first, *then*
// fall back externally to that episode's own page.
export function AnimeflvEpisodeSheet({
  title,
  pageUrl,
  onClose,
  onPick,
}: {
  title: string;
  pageUrl: string;
  onClose: () => void;
  onPick: (episode: AnimeflvEpisode) => void;
}) {
  const [episodes, setEpisodes] = useState<AnimeflvEpisode[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setEpisodes(null);
    setError("");
    api
      .listAnimeflvEpisodes(pageUrl)
      .then((eps) => {
        if (!cancelled) setEpisodes(eps);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
        setEpisodes([]);
      });
    return () => {
      cancelled = true;
    };
  }, [pageUrl]);

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div
        className="sheet-card animeflv-episode-card"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
      >
        <h3>{title}</h3>
        <p className="animeflv-episode-hint">
          AnimeFLV's player needs JavaScript this app can't run -- pick an episode to open its page in
          your browser.
        </p>
        {episodes === null ? (
          <div className="app-choice-empty">Loading episodes…</div>
        ) : error ? (
          <div className="app-choice-empty">{error}</div>
        ) : episodes.length === 0 ? (
          <div className="app-choice-empty">No episodes found on this show's page.</div>
        ) : (
          <div className="animeflv-episode-list">
            {episodes.map((ep) => (
              <div
                key={ep.number}
                className="animeflv-episode-tile"
                onClick={() => onPick(ep)}
                title={`Episode ${ep.number}`}
              >
                <div className="animeflv-episode-thumb-wrap">
                  {ep.thumbnail ? (
                    <img className="animeflv-episode-thumb" src={ep.thumbnail} draggable={false} />
                  ) : (
                    <div className="animeflv-episode-thumb animeflv-episode-thumb-blank" />
                  )}
                </div>
                <span className="animeflv-episode-number">{ep.number}</span>
              </div>
            ))}
          </div>
        )}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
