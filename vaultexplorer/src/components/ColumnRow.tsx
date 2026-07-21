import { useRef } from "react";
import { Entry } from "../api";
import { FileIcon } from "../icons";
import { useThumbnail } from "../hooks/useThumbnail";

export function ColumnRow({
  entry,
  fullPath,
  inVault,
  isSel,
  cut,
  onActivate,
  onSelect,
  onMenu,
}: {
  entry: Entry;
  fullPath: string;
  inVault: boolean;
  isSel: boolean;
  cut?: boolean;
  onActivate: () => void;
  onSelect: () => void;
  onMenu: (e: React.MouseEvent) => void;
}) {
  const rowRef = useRef<HTMLDivElement>(null);
  const thumb = useThumbnail(entry, fullPath, inVault, 32, rowRef);
  const isContainer = entry.is_dir || entry.is_vault;
  return (
    <div
      ref={rowRef}
      className={`column-row ${isSel ? "selected" : ""} ${cut ? "cut" : ""}`}
      onClick={isContainer ? onActivate : onSelect}
      onDoubleClick={onActivate}
      onContextMenu={onMenu}
    >
      <span className="column-ico">
        {thumb ? <img src={thumb} className="entry-thumb" alt="" draggable={false} /> : <FileIcon entry={entry} />}
      </span>
      <span className="column-name">{entry.name}</span>
      {entry.is_dir && <span className="column-chevron">›</span>}
    </div>
  );
}
