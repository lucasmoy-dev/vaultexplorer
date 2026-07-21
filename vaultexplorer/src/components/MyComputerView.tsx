import { Drive } from "../api";
import { formatSize } from "../api";
import { UsbDriveGlyph, DiskGlyph } from "../icons";

export function MyComputerView({
  drives,
  error,
  onOpenDrive,
  onMenu,
}: {
  drives: Drive[];
  error: string;
  onOpenDrive: (d: Drive) => void;
  onMenu: (e: React.MouseEvent, d: Drive) => void;
}) {
  return (
    <div className="my-computer">
      {error && <p className="error">{error}</p>}
      {drives.length === 0 && !error && <div className="column-empty">No drives found.</div>}
      <div className="drive-grid">
        {drives.map((d) => {
          const pct = d.total > 0 ? Math.min(100, Math.round((d.used / d.total) * 100)) : 0;
          return (
            <div
              key={d.path}
              className={`drive-card ${d.mountpoint ? "" : "unmounted"}`}
              onClick={() => onOpenDrive(d)}
              onContextMenu={(e) => onMenu(e, d)}
            >
              <span className="drive-ico">
                {d.removable ? <UsbDriveGlyph size={30} /> : <DiskGlyph size={30} />}
              </span>
              <div className="drive-info">
                <div className="drive-name">{d.label || d.model || d.name}</div>
                <div className="drive-sub">
                  {d.path}
                  {d.fstype ? ` · ${d.fstype.toUpperCase()}` : ""}
                  {d.mountpoint ? ` · ${d.mountpoint}` : " · Not mounted"}
                </div>
                {d.total > 0 && (
                  <>
                    <div className="drive-bar">
                      <div className="drive-bar-fill" style={{ width: `${pct}%` }} />
                    </div>
                    <div className="drive-sub">
                      {formatSize(d.free)} free of {formatSize(d.total)}
                    </div>
                  </>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
