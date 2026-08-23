import { useEffect, useRef, useState } from "react";
import { Entry, joinPath } from "../api";
import { ColumnRow } from "./ColumnRow";
import { FilePreviewPane } from "./TextEditorPane";

export function ColumnView({
  chain,
  list,
  inVault,
  root,
  onActivate,
  onMenu,
  previewEntry,
  onSelectFile,
  cutPaths,
  selectedNames,
  curDir,
  sortEntries,
  textEditorExts,
  onOpenInEditor,
}: {
  chain: { dirs: string[]; sel: string[] };
  list: (dir: string) => Promise<Entry[]>;
  inVault: boolean;
  root?: string;
  onActivate: (dir: string, entry: Entry) => void;
  onMenu: (e: React.MouseEvent, dir: string, entry: Entry) => void;
  previewEntry: { dir: string; entry: Entry } | null;
  onSelectFile: (dir: string, entry: Entry, e: React.MouseEvent) => void;
  // The real multi-selection (Ctrl+A, Ctrl/Shift-click, and everything
  // Ctrl+C/Delete/Enter act on), which only ever covers `curDir` -- the
  // rightmost non-preview column. Rows there highlight from this, so
  // Select All has something visible to show for itself.
  selectedNames: Set<string>;
  curDir: string;
  // The app's own sort (see App.tsx), so every column matches the order
  // the toolbar's sort chose and the keyboard steps through.
  sortEntries: (dir: string, entries: Entry[]) => Entry[];
  cutPaths?: string[];
  // Threaded straight through to FilePreviewPane -- the rightmost column
  // is the same preview List-with-Preview shows (see the render below), so
  // it needs the same "which formats open as text" wiring.
  textEditorExts: Set<string>;
  onOpenInEditor: (ext: string) => void;
}) {
  const [cols, setCols] = useState<{ dir: string; entries: Entry[] }[]>([]);
  const endRef = useRef<HTMLDivElement>(null);
  const dirsKey = chain.dirs.join("|");

  useEffect(() => {
    let cancelled = false;
    Promise.all(
      chain.dirs.map(async (dir) => {
        try {
          return { dir, entries: sortEntries(dir, await list(dir)) };
        } catch {
          return { dir, entries: [] as Entry[] };
        }
      })
    ).then((c) => !cancelled && setCols(c));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dirsKey, sortEntries]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ inline: "end", block: "nearest" });
  }, [cols, previewEntry]);

  return (
    <div className="columns">
      {cols.map((col, i) => (
        <div className="column" key={col.dir || "root"}>
          {col.entries.length === 0 && <div className="column-empty">Empty</div>}
          {col.entries.map((entry) => {
            const isSel =
              chain.sel[i] === entry.name ||
              (previewEntry?.dir === col.dir && previewEntry.entry.name === entry.name) ||
              (col.dir === curDir && selectedNames.has(entry.name));
            return (
              <ColumnRow
                key={entry.name}
                entry={entry}
                fullPath={joinPath(col.dir, entry.name)}
                inVault={inVault}
                isSel={isSel}
                cut={!!cutPaths?.includes(joinPath(col.dir, entry.name))}
                onActivate={() => onActivate(col.dir, entry)}
                onSelect={(e) => onSelectFile(col.dir, entry, e)}
                onMenu={(e) => onMenu(e, col.dir, entry)}
              />
            );
          })}
        </div>
      ))}
      {/* One preview, not two implementations: the last column used to
          render PreviewColumn directly, so a PDF got a dead cover image
          and a markdown/text file got nothing but its metadata -- while
          List with Preview, going through FilePreviewPane, rendered all
          three properly. Same pane here, wrapped so it keeps a Miller
          column's fixed width instead of stretching. */}
      {previewEntry && (
        <div className="columns-preview">
          <FilePreviewPane
            target={previewEntry}
            inVault={inVault}
            root={root}
            textEditorExts={textEditorExts}
            onOpenInEditor={onOpenInEditor}
          />
        </div>
      )}
      <div ref={endRef} />
    </div>
  );
}
