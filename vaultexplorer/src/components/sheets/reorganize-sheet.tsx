import { useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { api, baseName } from "../../api";

// "Reorganize & Clean" -- hands a folder to a headless `claude` CLI run
// (see reorganize.rs) instead of this app trying to hardcode "what does
// a tidy folder look like" itself. Two steps in one sheet: a plain-
// language confirmation (this is a real bulk rename/move/create-folder
// action, not something to fire without a heads-up even though deletes
// specifically land in the trash, not gone for good), then a live log
// once it's actually running.
export function ReorganizeSheet({
  path,
  onClose,
  onDone,
}: {
  path: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const [started, setStarted] = useState(false);
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const [error, setError] = useState("");
  const logRef = useRef<HTMLDivElement>(null);

  function start() {
    setStarted(true);
    setRunning(true);
    setError("");
    const channel = new Channel<string>();
    channel.onmessage = (line) => {
      setLines((l) => [...l, line]);
      requestAnimationFrame(() => {
        logRef.current?.scrollTo({ top: logRef.current.scrollHeight });
      });
    };
    api
      .claudeReorganizeFolder(path, channel)
      .then(() => onDone())
      .catch((e) => setError(String(e)))
      .finally(() => setRunning(false));
  }

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card reorganize-sheet" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Reorganize &amp; Clean “{baseName(path)}”</h3>
        {!started ? (
          <>
            <p className="hint">
              Claude will look through this folder and rename, move, create subfolders, and
              restructure as it sees fit -- and move anything that looks like a leftover test
              file or duplicate to the trash (not permanently deleted). This only touches files
              inside this exact folder.
            </p>
            <div className="sheet-actions">
              <button className="btn-plain" onClick={onClose}>
                Cancel
              </button>
              <button className="btn-primary" onClick={start}>
                Start
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="reorganize-log" ref={logRef}>
              {lines.length === 0 && running && <p className="hint">Starting…</p>}
              {lines.map((line, i) => (
                <div key={i} className="reorganize-log-line">
                  {line}
                </div>
              ))}
            </div>
            {error && <p className="error">{error}</p>}
            <div className="sheet-actions">
              {running && <span className="reorganize-spinner" aria-label="Working" />}
              <button className="btn-primary" onClick={onClose} disabled={running}>
                {running ? "Working…" : "Done"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
