import { useEffect, useRef, useState } from "react";
import { Entry, joinPath } from "../api";
import { ColumnRow } from "./ColumnRow";
import { PreviewColumn } from "./PreviewColumn";

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
}: {
  chain: { dirs: string[]; sel: string[] };
  list: (dir: string) => Promise<Entry[]>;
  inVault: boolean;
  root?: string;
  onActivate: (dir: string, entry: Entry) => void;
  onMenu: (e: React.MouseEvent, dir: string, entry: Entry) => void;
  previewEntry: { dir: string; entry: Entry } | null;
  onSelectFile: (dir: string, entry: Entry) => void;
  cutPaths?: string[];
}) {
  const [cols, setCols] = useState<{ dir: string; entries: Entry[] }[]>([]);
  const endRef = useRef<HTMLDivElement>(null);
  const dirsKey = chain.dirs.join("|");

  useEffect(() => {
    let cancelled = false;
    Promise.all(
      chain.dirs.map(async (dir) => {
        try {
          const entries = await list(dir);
          entries.sort((a, b) => {
            if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
            return a.name.localeCompare(b.name);
          });
          return { dir, entries };
        } catch {
          return { dir, entries: [] as Entry[] };
        }
      })
    ).then((c) => !cancelled && setCols(c));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dirsKey]);

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
              (previewEntry?.dir === col.dir && previewEntry.entry.name === entry.name);
            return (
              <ColumnRow
                key={entry.name}
                entry={entry}
                fullPath={joinPath(col.dir, entry.name)}
                inVault={inVault}
                isSel={isSel}
                cut={!!cutPaths?.includes(joinPath(col.dir, entry.name))}
                onActivate={() => onActivate(col.dir, entry)}
                onSelect={() => onSelectFile(col.dir, entry)}
                onMenu={(e) => onMenu(e, col.dir, entry)}
              />
            );
          })}
        </div>
      ))}
      {previewEntry && (
        <PreviewColumn
          entry={previewEntry.entry}
          fullPath={joinPath(previewEntry.dir, previewEntry.entry.name)}
          inVault={inVault}
          root={root}
        />
      )}
      <div ref={endRef} />
    </div>
  );
}
