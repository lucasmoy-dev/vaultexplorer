import { useRef, useState } from "react";
import { PasswordInput } from "./vault-sheets";
import { WHITESUR_FOLDER_ICONS, SYMBOL_ICONS } from "../../icons";

export function ChangeIconSheet({
  name,
  current,
  onCancel,
  onSubmit,
}: {
  name: string;
  current?: string;
  onCancel: () => void;
  onSubmit: (icon: string | null) => void;
}) {
  const [selected, setSelected] = useState(current ?? "");
  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card icon-picker-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Change Icon for “{name}”</h3>
        <p>Pick a macOS folder icon or an emoji, or clear it to use the default.</p>
        <div className="icon-picker-scroll">
          <div className="icon-section-label">Folders</div>
          <div className="icon-grid">
            {WHITESUR_FOLDER_ICONS.map((f) => (
              <button
                key={f.value}
                className={`icon-choice ws ${selected === f.value ? "on" : ""}`}
                title={f.label}
                onClick={() => setSelected(f.value)}
              >
                <img src={f.url} alt={f.label} draggable={false} />
              </button>
            ))}
          </div>
          <div className="icon-section-label">Symbols</div>
          <div className="icon-grid">
            {SYMBOL_ICONS.map((s) => (
              <button
                key={s.value}
                className={`icon-choice sym ${selected === s.value ? "on" : ""}`}
                title={s.label}
                onClick={() => setSelected(s.value)}
              >
                <span className="symbol-icon" dangerouslySetInnerHTML={{ __html: s.svg }} />
              </button>
            ))}
          </div>
        </div>
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-plain" onClick={() => onSubmit(null)}>
            Use Default
          </button>
          <button className="btn-primary" onClick={() => onSubmit(selected || null)}>
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

export function ResizeSheet({
  count,
  onCancel,
  onSubmit,
}: {
  count: number;
  onCancel: () => void;
  onSubmit: (width: number, height: number) => void;
}) {
  const [width, setWidth] = useState(1920);
  const [height, setHeight] = useState(1080);
  const [lockAspect, setLockAspect] = useState(true);
  const ratio = useRef(width / height);

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Resize {count > 1 ? `${count} Images` : "Image"}</h3>
        <p>Fits within the given box, keeping the original proportions. Saved as a new file.</p>
        <div className="sheet-actions" style={{ justifyContent: "flex-start", gap: 8 }}>
          <input
            type="number"
            min={1}
            value={width}
            style={{ width: 90 }}
            onChange={(e) => {
              const w = Number(e.target.value);
              setWidth(w);
              if (lockAspect) setHeight(Math.round(w / ratio.current));
            }}
          />
          <span>×</span>
          <input
            type="number"
            min={1}
            value={height}
            style={{ width: 90 }}
            onChange={(e) => {
              const h = Number(e.target.value);
              setHeight(h);
              if (lockAspect) setWidth(Math.round(h * ratio.current));
            }}
          />
        </div>
        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={lockAspect}
            onChange={(e) => {
              setLockAspect(e.target.checked);
              if (e.target.checked) ratio.current = width / height;
            }}
          />
          Maintain aspect ratio
        </label>
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" onClick={() => onSubmit(width, height)}>
            Resize
          </button>
        </div>
      </div>
    </div>
  );
}

export function ConvertSheet({
  names,
  targetLabel,
  mode,
  onCancel,
  onSubmit,
}: {
  // The whole selection: the quality picked here applies to all of them.
  names: string[];
  targetLabel: string;
  mode: "imageQuality" | "mediaQuality";
  onCancel: () => void;
  onSubmit: (value: string) => void;
}) {
  const [imgQuality, setImgQuality] = useState(85);
  const [mediaQuality, setMediaQuality] = useState<"high" | "medium" | "low">("medium");

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>
          {names.length > 1
            ? `Convert ${names.length} items to ${targetLabel}`
            : `Convert “${names[0]}” to ${targetLabel}`}
        </h3>
        {mode === "imageQuality" ? (
          <div className="quality-slider">
            <span>Quality</span>
            <input
              type="range"
              min={1}
              max={100}
              value={imgQuality}
              onChange={(e) => setImgQuality(Number(e.target.value))}
            />
            <span className="quality-value">{imgQuality}</span>
          </div>
        ) : (
          <div className="sheet-actions" style={{ justifyContent: "flex-start", gap: 8 }}>
            {(["high", "medium", "low"] as const).map((q) => (
              <button
                key={q}
                className={`seg seg-text ${mediaQuality === q ? "on" : ""}`}
                onClick={() => setMediaQuality(q)}
              >
                {q[0].toUpperCase() + q.slice(1)}
              </button>
            ))}
          </div>
        )}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button
            className="btn-primary"
            onClick={() => onSubmit(mode === "imageQuality" ? String(imgQuality) : mediaQuality)}
          >
            Convert
          </button>
        </div>
      </div>
    </div>
  );
}

const MONTAGE_RESOLUTIONS: { label: string; width: number; height: number }[] = [
  { label: "480p", width: 854, height: 480 },
  { label: "720p", width: 1280, height: 720 },
  { label: "1080p", width: 1920, height: 1080 },
];

export function MontageOptionsSheet({
  imageCount,
  videoCount,
  hasAudioTrack,
  onCancel,
  onSubmit,
}: {
  imageCount: number;
  videoCount: number;
  hasAudioTrack: boolean;
  onCancel: () => void;
  onSubmit: (opts: {
    width: number;
    height: number;
    quality: "high" | "medium" | "low";
    includeOriginalAudio: boolean;
  }) => void;
}) {
  const [resolution, setResolution] = useState(MONTAGE_RESOLUTIONS[2]);
  const [quality, setQuality] = useState<"high" | "medium" | "low">("medium");
  const [includeOriginalAudio, setIncludeOriginalAudio] = useState(true);

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Convert to MP4 (Montage)</h3>
        <p className="hint" style={{ marginTop: -4 }}>
          {imageCount} image{imageCount === 1 ? "" : "s"} and {videoCount} video{videoCount === 1 ? "" : "s"}, in
          order, with a short crossfade between each
          {hasAudioTrack ? " and the selected audio track playing underneath." : "."}
        </p>
        <label className="field-label">Resolution</label>
        <div className="segmented compress-level">
          {MONTAGE_RESOLUTIONS.map((r) => (
            <button
              key={r.label}
              className={`seg seg-text ${resolution.label === r.label ? "on" : ""}`}
              onClick={() => setResolution(r)}
            >
              {r.label}
            </button>
          ))}
        </div>
        <label className="field-label">Quality</label>
        <div className="sheet-actions" style={{ justifyContent: "flex-start", gap: 8 }}>
          {(["high", "medium", "low"] as const).map((q) => (
            <button
              key={q}
              className={`seg seg-text ${quality === q ? "on" : ""}`}
              onClick={() => setQuality(q)}
            >
              {q[0].toUpperCase() + q.slice(1)}
            </button>
          ))}
        </div>
        {videoCount > 0 && (
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={includeOriginalAudio}
              onChange={(e) => setIncludeOriginalAudio(e.target.checked)}
            />
            Keep the videos' own audio{hasAudioTrack ? " (mixed with the audio track)" : ""}
          </label>
        )}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button
            className="btn-primary"
            onClick={() =>
              onSubmit({
                width: resolution.width,
                height: resolution.height,
                quality,
                includeOriginalAudio,
              })
            }
          >
            Convert
          </button>
        </div>
      </div>
    </div>
  );
}

export function CompressOptionsSheet({
  defaultName,
  allowTargz = true,
  onCancel,
  onSubmit,
}: {
  defaultName: string;
  allowTargz?: boolean;
  onCancel: () => void;
  onSubmit: (opts: {
    destName: string;
    format: "zip" | "targz";
    password: string | null;
    level: number;
    readme: string | null;
  }) => void;
}) {
  const [format, setFormat] = useState<"zip" | "targz">("zip");
  const [destName, setDestName] = useState(defaultName);
  const [level, setLevel] = useState<6 | 9>(6);
  const [usePassword, setUsePassword] = useState(false);
  const [pw, setPw] = useState("");
  const [pwConfirm, setPwConfirm] = useState("");
  const [useReadme, setUseReadme] = useState(false);
  const [readme, setReadme] = useState("");
  const [error, setError] = useState("");

  function changeFormat(next: "zip" | "targz") {
    setFormat(next);
    setDestName((prev) => prev.replace(/\.(zip|tar\.gz|tgz)$/i, "") + (next === "zip" ? ".zip" : ".tar.gz"));
    if (next === "targz") setUsePassword(false);
  }

  function go() {
    if (destName.trim() === "") return;
    if (usePassword) {
      if (pw === "") {
        setError("Password cannot be empty");
        return;
      }
      if (pw !== pwConfirm) {
        setError("Passwords don't match");
        return;
      }
    }
    onSubmit({
      destName: destName.trim(),
      format,
      password: usePassword ? pw : null,
      level,
      readme: useReadme && readme.trim() !== "" ? readme : null,
    });
  }

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card compress-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Compress…</h3>
        {allowTargz && (
          <>
            <label className="field-label">Format</label>
            <div className="segmented compress-level">
              <button className={`seg seg-text ${format === "zip" ? "on" : ""}`} onClick={() => changeFormat("zip")}>
                Zip
              </button>
              <button
                className={`seg seg-text ${format === "targz" ? "on" : ""}`}
                onClick={() => changeFormat("targz")}
              >
                tar.gz
              </button>
            </div>
            {format === "targz" && (
              <p className="hint" style={{ marginTop: -8 }}>
                No 7z or rar here -- neither is actually installed on this machine (rar specifically
                has no free/open-source writer at all), and tar.gz doesn't support password
                protection. Pick Zip above for either.
              </p>
            )}
          </>
        )}

        <label className="field-label">Archive name</label>
        <input value={destName} onChange={(e) => setDestName(e.target.value)} />

        {format === "zip" && (
          <>
            <label className="field-label">Compression level</label>
            <div className="segmented compress-level">
              <button className={`seg seg-text ${level === 6 ? "on" : ""}`} onClick={() => setLevel(6)}>
                Normal
              </button>
              <button className={`seg seg-text ${level === 9 ? "on" : ""}`} onClick={() => setLevel(9)}>
                Maximum
              </button>
            </div>

            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={usePassword}
                onChange={(e) => setUsePassword(e.target.checked)}
              />
              Protect with password (AES-256)
            </label>
            {usePassword && (
              <>
                <PasswordInput
                  placeholder="Password"
                  value={pw}
                  onChange={(e) => setPw(e.target.value)}
                />
                <PasswordInput
                  placeholder="Confirm password"
                  value={pwConfirm}
                  onChange={(e) => setPwConfirm(e.target.value)}
                />
              </>
            )}
          </>
        )}

        <label className="checkbox-row">
          <input type="checkbox" checked={useReadme} onChange={(e) => setUseReadme(e.target.checked)} />
          Add README.txt
        </label>
        {useReadme && (
          <textarea
            className="readme-area"
            placeholder="README content…"
            value={readme}
            onChange={(e) => setReadme(e.target.value)}
          />
        )}

        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" onClick={go}>
            Compress
          </button>
        </div>
      </div>
    </div>
  );
}
