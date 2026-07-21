import { useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { Entry, api, joinPath, parentPath, baseName } from "../api";
import { FileIcon, PlaceGlyph, DiskGlyph } from "../icons";
import { ContextMenu, MenuState } from "../ContextMenu";
import { useFavorites } from "../hooks/useFavorites";

type FilterGroup = { name: string; patterns: string[] };

function favLabel(path: string, home: string): string {
  if (path === "/") return "System";
  if (path === home) return "Home";
  if (path === "/usr/share/applications") return "Applications";
  return baseName(path) || path;
}

// Swaps the extension on a filename to match a portal-supplied glob filter
// pattern (e.g. "*.txt" -> ".txt") -- patterns that aren't a plain
// single-extension glob (globsets like "*.jpg;*.jpeg", or bare "*") are left
// alone since there's no single unambiguous extension to apply.
function applyPattern(name: string, pattern: string): string {
  const m = /^\*(\.[A-Za-z0-9]+)$/.exec(pattern);
  if (!m) return name;
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  return stem + m[1];
}

// Minimal file browser shown in a dedicated window when VaultExplorer is
// acting as the system file-picker portal backend (see portal.rs). Kept
// deliberately separate from the full `Explorer` component -- reusing its
// entire vault/clipboard/menu state machine for a modal pick-one-file flow
// would be a lot of surface for little benefit; this only needs to browse,
// select, and hand back a real fs path. Mirrors macOS's Save panel: a
// collapsed name+location form that expands into a full browser with a
// Favorites sidebar.
export function PickerView({
  mode,
  reqId,
  multiple,
  initialName,
  initialFilters,
  initialFolder,
}: {
  mode: "open" | "save";
  reqId: string;
  multiple: boolean;
  initialName: string | null;
  initialFilters: FilterGroup[];
  initialFolder: string | null;
}) {
  const [home, setHome] = useState<string | null>(null);
  const [mobile, setMobile] = useState(false);
  const [dir, setDir] = useState<string | null>(null);
  const [pathInput, setPathInput] = useState("");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [saveName, setSaveName] = useState(initialName || "untitled.txt");
  const [expanded, setExpanded] = useState(mode === "open");
  const [filterIdx, setFilterIdx] = useState(0);
  const [error, setError] = useState("");
  const [renaming, setRenaming] = useState<{ name: string; value: string } | null>(null);
  const [menu, setMenu] = useState<MenuState>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  const dragNameRef = useRef<string | null>(null);
  const skipBlurCommitRef = useRef(false);

  const { favPaths } = useFavorites(home ?? "", mobile);

  const skipMountResizeRef = useRef(true);
  useEffect(() => {
    // portal.rs already creates the window at whichever size matches this
    // component's *initial* `expanded` state, so the first run of this
    // effect (mount) would just be resizing the window to the size it's
    // already at -- skip it, only actual expand/collapse clicks should
    // trigger a resize. Only the Save panel collapses at all -- Open always
    // shows the full browser, so it stays at the window's built-in size.
    // Re-centering after the resize matches the native macOS panel, which
    // stays anchored to the screen's center as it grows/shrinks rather than
    // just the window's fixed top-left corner.
    if (skipMountResizeRef.current) {
      skipMountResizeRef.current = false;
      return;
    }
    if (mode !== "save") return;
    const win = getCurrentWebviewWindow();
    const size = expanded ? new LogicalSize(760, 560) : new LogicalSize(420, 230);
    win.setSize(size).then(() => win.center()).catch(() => {});
  }, [expanded, mode]);

  useEffect(() => {
    api
      .homeDir()
      .then(async (h) => {
        setHome(h);
        // The portal's `current_folder` hint (validated -- callers can send
        // stale paths) wins; failing that, Save defaults to Downloads
        // (matching every browser's own save dialog) instead of home, since
        // that's overwhelmingly the common case ("if I'm downloading
        // something"). Open has no equivalent convention, so it keeps home.
        const candidates = [initialFolder, mode === "save" ? joinPath(h, "Downloads") : null].filter(
          (p): p is string => !!p
        );
        for (const candidate of candidates) {
          try {
            await api.fsList(candidate, false);
            setDir(candidate);
            return;
          } catch {
            /* try the next candidate */
          }
        }
        setDir(h);
      })
      .catch(() => {
        setHome("/");
        setDir("/");
      });
    api.isMobilePlatform().then(setMobile).catch(() => {});
  }, [initialFolder, mode]);

  function refresh(d: string) {
    api
      .fsList(d, false)
      .then((list) => {
        setEntries([...list].sort((a, b) => (a.is_dir === b.is_dir ? a.name.localeCompare(b.name) : a.is_dir ? -1 : 1)));
      })
      .catch((e) => setError(String(e)));
  }

  useEffect(() => {
    if (dir === null) return;
    setSelected(new Set());
    setRenaming(null);
    setPathInput(dir);
    refresh(dir);
  }, [dir]);

  function navigateTo(path: string) {
    api
      .fsList(path, false)
      .then(() => setDir(path))
      .catch((e) => setError(String(e)));
  }

  function open(entry: Entry) {
    if (entry.is_dir) {
      setDir((d) => joinPath(d ?? "", entry.name));
    } else if (mode === "open") {
      toggle(entry.name);
    }
  }
  function toggle(name: string) {
    setSelected((prev) => {
      if (!multiple) return prev.has(name) ? new Set() : new Set([name]);
      const next = new Set(prev);
      next.has(name) ? next.delete(name) : next.add(name);
      return next;
    });
  }
  function click(entry: Entry) {
    toggle(entry.name);
    if (mode === "save" && !entry.is_dir) setSaveName(entry.name);
  }

  // Selects just the stem (not the extension) on click/focus, matching
  // macOS/Windows Save panels -- makes replacing "Magnum Opus" in
  // "Magnum Opus.rtf" a single overtype instead of a manual select-drag.
  // `preventDefault` on mousedown stops the browser from placing the
  // caret at the click position first (which would collapse this range
  // right back down before the user sees it).
  function selectStem(e: React.MouseEvent<HTMLInputElement> | React.FocusEvent<HTMLInputElement>) {
    const el = e.currentTarget;
    const dot = saveName.lastIndexOf(".");
    const end = dot > 0 ? dot : saveName.length;
    requestAnimationFrame(() => el.setSelectionRange(0, end));
  }

  async function createFolder() {
    if (!dir) return;
    let name = "New Folder";
    let n = 2;
    const existing = new Set(entries.map((e) => e.name));
    while (existing.has(name)) name = `New Folder ${n++}`;
    try {
      await api.fsMkdir(joinPath(dir, name));
      refresh(dir);
      setSelected(new Set([name]));
      setRenaming({ name, value: name });
    } catch (e) {
      setError(String(e));
    }
  }

  async function commitRename() {
    if (!dir || !renaming) return;
    const { name, value } = renaming;
    setRenaming(null);
    if (!value.trim() || value === name) return;
    try {
      await api.fsRename(joinPath(dir, name), joinPath(dir, value.trim()));
      refresh(dir);
    } catch (e) {
      setError(String(e));
    }
  }

  async function deleteEntry(name: string) {
    if (!dir) return;
    try {
      await api.fsTrash(joinPath(dir, name));
      refresh(dir);
    } catch (e) {
      setError(String(e));
    }
  }

  function rowMenu(e: React.MouseEvent, entry: Entry) {
    e.preventDefault();
    e.stopPropagation();
    setSelected(new Set([entry.name]));
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Rename", onClick: () => setRenaming({ name: entry.name, value: entry.name }) },
        { label: "Delete", danger: true, onClick: () => deleteEntry(entry.name) },
      ],
    });
  }

  function onDrop(e: React.DragEvent, target: Entry) {
    e.preventDefault();
    setDragOver(null);
    const name = dragNameRef.current;
    dragNameRef.current = null;
    if (!dir || !name || !target.is_dir || name === target.name) return;
    api
      .fsRename(joinPath(dir, name), joinPath(joinPath(dir, target.name), name))
      .then(() => refresh(dir))
      .catch((e) => setError(String(e)));
  }

  async function confirm() {
    if (!dir) return;
    if (mode === "save") {
      const name = saveName.trim();
      if (!name) return;
      const exists = entries.some((e) => e.name === name && !e.is_dir);
      if (exists && !window.confirm(`"${name}" already exists. Replace it?`)) return;
      await api.portalResolve(reqId, [`file://${joinPath(dir, name)}`]);
      getCurrentWebviewWindow().close();
      return;
    }
    const paths = [...selected]
      .filter((n) => !entries.find((e) => e.name === n)?.is_dir)
      .map((n) => joinPath(dir, n));
    if (paths.length === 0) return;
    const uris = paths.map((p) => `file://${p}`);
    await api.portalResolve(reqId, uris);
    getCurrentWebviewWindow().close();
  }
  async function cancel() {
    await api.portalCancel(reqId);
    getCurrentWebviewWindow().close();
  }

  function pickFilter(idx: number) {
    setFilterIdx(idx);
    const pattern = initialFilters[idx]?.patterns[0];
    if (pattern) setSaveName((n) => applyPattern(n, pattern));
  }
  function openFormatMenu(e: React.MouseEvent) {
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({
      x: r.left,
      y: r.bottom + 4,
      items: initialFilters.map((f, i) => ({ label: f.name, onClick: () => pickFilter(i) })),
    });
  }
  const formatPicker = initialFilters.length > 0 && (
    <div className="picker-field-row">
      <label>File Format:</label>
      <button className="btn-plain picker-format-btn" onClick={openFormatMenu}>
        <span>{initialFilters[filterIdx]?.name ?? "Format"}</span>
        <span className="picker-format-caret">&#9662;</span>
      </button>
    </div>
  );

  const sidebar = (
    <div className="picker-sidebar">
      <div className="picker-sidebar-label">Favorites</div>
      {favPaths.map((path) => (
        <div
          key={path}
          className={`picker-sidebar-item ${dir === path ? "active" : ""}`}
          onClick={() => navigateTo(path)}
        >
          {path === "/" ? <DiskGlyph size={16} /> : <PlaceGlyph size={16} />}
          <span>{favLabel(path, home ?? "")}</span>
        </div>
      ))}
    </div>
  );

  const fileList = (
    <div className="picker-list">
      {entries.map((e) => (
        <div
          key={e.name}
          className={`picker-row ${selected.has(e.name) ? "selected" : ""} ${dragOver === e.name ? "drag-over" : ""}`}
          draggable
          onClick={(ev) => {
            ev.stopPropagation();
            click(e);
          }}
          onDoubleClick={() => open(e)}
          onContextMenu={(ev) => rowMenu(ev, e)}
          onDragStart={(ev) => {
            dragNameRef.current = e.name;
            // WebKitGTK (this app's Linux webview) won't reliably fire
            // `drop` later unless dragstart actually populates dataTransfer.
            ev.dataTransfer.setData("text/plain", e.name);
            ev.dataTransfer.effectAllowed = "move";
          }}
          onDragOver={(ev) => {
            if (e.is_dir && dragNameRef.current && dragNameRef.current !== e.name) {
              ev.preventDefault();
              setDragOver(e.name);
            }
          }}
          onDragLeave={() => setDragOver((d) => (d === e.name ? null : d))}
          onDrop={(ev) => onDrop(ev, e)}
        >
          <FileIcon entry={e} />
          {renaming?.name === e.name ? (
            <input
              className="picker-inline-rename"
              autoFocus
              value={renaming.value}
              onClick={(ev) => ev.stopPropagation()}
              onChange={(ev) => setRenaming({ name: e.name, value: ev.target.value })}
              onKeyDown={(ev) => {
                // Both branches pre-arm the skip flag: Enter already commits
                // here, and Escape means don't -- either way the blur this
                // triggers (input unmounts as `renaming` clears) must not
                // fire a second, stale commitRename() off the old closure.
                if (ev.key === "Enter") {
                  skipBlurCommitRef.current = true;
                  commitRename();
                }
                if (ev.key === "Escape") {
                  ev.stopPropagation();
                  skipBlurCommitRef.current = true;
                  setRenaming(null);
                }
              }}
              onBlur={() => {
                if (skipBlurCommitRef.current) {
                  skipBlurCommitRef.current = false;
                  return;
                }
                commitRename();
              }}
            />
          ) : (
            <span>{e.name}</span>
          )}
        </div>
      ))}
    </div>
  );

  const pathBar = (
    <input
      className="picker-path-input"
      value={pathInput}
      onChange={(e) => setPathInput(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") navigateTo(pathInput.trim() || "/");
        if (e.key === "Escape" && dir) setPathInput(dir);
      }}
      onBlur={() => dir && setPathInput(dir)}
    />
  );

  // Escape closes the dialog (Cancel) -- the window has no OS titlebar/close
  // button (decorations are off, to match the rest of the app's look), so
  // this is the only keyboard way out. Skipped while the format/context menu
  // or an inline rename is up so Escape closes *those* first, not the whole
  // picker out from under them.
  function onKeyDownRoot(e: React.KeyboardEvent) {
    if (e.key === "Escape" && !menu && !renaming) cancel();
  }

  if (mode === "save" && !expanded) {
    return (
      <div className="picker-view compressed" onClick={() => setMenu(null)} onKeyDown={onKeyDownRoot}>
        <div className="picker-field-row">
          <label>Save As:</label>
          <input
            className="picker-filename"
            value={saveName}
            onChange={(e) => setSaveName(e.target.value)}
            onMouseDown={(e) => {
              e.preventDefault();
              e.currentTarget.focus();
              selectStem(e);
            }}
            onFocus={selectStem}
            autoFocus
          />
        </div>
        <div className="picker-field-row">
          <label>Where:</label>
          {pathBar}
          <button className="btn-plain picker-expand" onClick={() => setExpanded(true)} title="Expand">
            &#9662;
          </button>
        </div>
        {formatPicker}
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={cancel}>
            Cancel
          </button>
          <button className="btn-primary" disabled={saveName.trim() === ""} onClick={confirm}>
            Save
          </button>
        </div>
        <ContextMenu state={menu} onClose={() => setMenu(null)} />
      </div>
    );
  }

  return (
    <div className="picker-view expanded" onClick={() => setMenu(null)} onKeyDown={onKeyDownRoot}>
      {mode === "save" && (
        <div className="picker-field-row">
          <label>Save As:</label>
          <input
            className="picker-filename"
            value={saveName}
            onChange={(e) => setSaveName(e.target.value)}
            onMouseDown={(e) => {
              e.preventDefault();
              e.currentTarget.focus();
              selectStem(e);
            }}
            onFocus={selectStem}
            autoFocus
          />
        </div>
      )}
      <div className="picker-toolbar">
        <button
          className="btn-plain"
          disabled={!dir || dir === "/"}
          onClick={() => dir && setDir(parentPath(dir) || "/")}
        >
          Up
        </button>
        {pathBar}
        {mode === "save" && (
          <button className="btn-plain picker-expand" onClick={() => setExpanded(false)} title="Collapse">
            &#9652;
          </button>
        )}
      </div>
      {error && <p className="error">{error}</p>}
      <div className="picker-body">
        {sidebar}
        <div className="picker-main">{fileList}</div>
      </div>
      {formatPicker}
      <div className="sheet-actions">
        <button className="btn-plain" onClick={createFolder}>
          New Folder
        </button>
        <span className="sheet-actions-spacer" />
        <button className="btn-plain" onClick={cancel}>
          Cancel
        </button>
        <button
          className="btn-primary"
          disabled={mode === "open" ? selected.size === 0 : saveName.trim() === ""}
          onClick={confirm}
        >
          {mode === "open" ? "Open" : "Save"}
        </button>
      </div>
      <ContextMenu state={menu} onClose={() => setMenu(null)} />
    </div>
  );
}
