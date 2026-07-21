import { useCallback, useEffect, useRef, useState } from "react";
import { api, baseName, joinPath, parentPath } from "../api";
import { DEFAULT_START_KEY, PHONE_STORAGE_PATH } from "../constants";

// Favorites-sidebar state: the pinned path list itself, its persisted tag
// colors, the pinned-files-sort-first set, and the configurable "opens on
// launch" default -- all localStorage-backed, all keyed by full path so
// they work across both real-fs and vault locations.
export function useFavorites(home: string, mobile: boolean) {
  // Captured once, before any effect can persist a default -- lets the
  // mobile-defaults effect below tell "user already has favorites saved"
  // apart from "this is still the placeholder we just set".
  const hadSavedFavorites = useRef(localStorage.getItem("vaultexplorer:favorites") !== null);
  const [favPaths, setFavPaths] = useState<string[]>(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:favorites");
      if (raw) return JSON.parse(raw);
    } catch {
      /* ignore */
    }
    return [
      home,
      "/usr/share/applications",
      joinPath(home, "Pictures"),
      joinPath(home, "Downloads"),
      joinPath(home, "Desktop"),
      joinPath(home, "Documents"),
    ];
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:favorites", JSON.stringify(favPaths));
  }, [favPaths]);
  // The desktop defaults above are real folders on a normal Linux $HOME,
  // but on Android there's no /usr/share/applications and no
  // ~/Pictures/Downloads/Desktop -- Android sandboxes each app to its own
  // data dir, so those paths just 404. Swap in ones that live inside that
  // sandbox instead (created on demand by `browse_root_dir`, so they're
  // guaranteed to exist and need no storage permission at all), plus the
  // real shared-storage root -- tapping that one prompts for the "All
  // files access" permission first if it isn't granted yet (see openFavorite).
  useEffect(() => {
    if (!mobile || hadSavedFavorites.current) return;
    setFavPaths([
      home,
      joinPath(home, "Documents"),
      joinPath(home, "Pictures"),
      joinPath(home, "Downloads"),
      PHONE_STORAGE_PATH,
    ]);
  }, [mobile, home]);

  // The favorite that opens on launch, instead of the home folder.
  const [defaultStartPath, setDefaultStartPathState] = useState<string | null>(() =>
    localStorage.getItem(DEFAULT_START_KEY)
  );
  function setDefaultStartPath(path: string | null) {
    setDefaultStartPathState(path);
    if (path) localStorage.setItem(DEFAULT_START_KEY, path);
    else localStorage.removeItem(DEFAULT_START_KEY);
  }

  // Pinned files/folders (full path, works across both fs and vault
  // locations) always sort first within whatever folder they're in.
  const [pinnedPaths, setPinnedPaths] = useState<Set<string>>(() => {
    try {
      const raw = localStorage.getItem("vaultexplorer:pinned");
      if (raw) return new Set(JSON.parse(raw));
    } catch {
      /* ignore */
    }
    return new Set();
  });
  useEffect(() => {
    localStorage.setItem("vaultexplorer:pinned", JSON.stringify([...pinnedPaths]));
  }, [pinnedPaths]);
  function togglePin(path: string) {
    setPinnedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  const [favTags, setFavTags] = useState<Record<string, string>>({});
  const refreshFavTags = useCallback(async () => {
    const byParent = new Map<string, string[]>();
    for (const p of favPaths) {
      if (p === "/") continue;
      const parent = parentPath(p);
      if (!byParent.has(parent)) byParent.set(parent, []);
      byParent.get(parent)!.push(p);
    }
    const next: Record<string, string> = {};
    await Promise.all(
      [...byParent.entries()].map(async ([parent, favs]) => {
        try {
          const tagsMap = await api.fsGetTags(parent);
          for (const favPath of favs) {
            const color = tagsMap[baseName(favPath)];
            if (color) next[favPath] = color;
          }
        } catch {
          /* ignore */
        }
      })
    );
    setFavTags(next);
  }, [favPaths]);
  useEffect(() => {
    refreshFavTags();
  }, [refreshFavTags]);

  function addFavorite(path: string) {
    setFavPaths((p) => (p.includes(path) ? p : [...p, path]));
  }
  function removeFavorite(path: string) {
    setFavPaths((p) => p.filter((x) => x !== path));
  }
  function moveFavorite(from: number, to: number) {
    setFavPaths((p) => {
      if (from === to || from < 0 || from >= p.length || to < 0 || to >= p.length) return p;
      const next = [...p];
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);
      return next;
    });
  }

  return {
    favPaths,
    defaultStartPath,
    setDefaultStartPath,
    pinnedPaths,
    togglePin,
    favTags,
    refreshFavTags,
    addFavorite,
    removeFavorite,
    moveFavorite,
  };
}
