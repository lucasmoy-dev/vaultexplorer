import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { api, baseName } from "../../api";

export function GitStatusSheet({
  root,
  status,
  onClose,
}: {
  root: string;
  status: Record<string, string>;
  onClose: () => void;
}) {
  const rows = Object.entries(status);
  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Git Status</h3>
        <p style={{ overflowWrap: "anywhere" }}>{root}</p>
        {rows.length === 0 ? (
          <p>No changes.</p>
        ) : (
          <div className="info-rows">
            {rows.map(([path, code]) => (
              <div className="info-row" key={path}>
                <span style={{ fontFamily: "ui-monospace, monospace" }}>{code}</span>
                <span className="info-path">{path}</span>
              </div>
            ))}
          </div>
        )}
        <div className="sheet-actions">
          <button className="btn-primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}

// Switching to sign in (a browser, for Drive) or to a terminal (to add a
// git remote) and back regains window focus with a click -- which, on
// this webview, still delivers as a real mousedown on whatever's
// underneath, including a full-viewport sheet-overlay's backdrop-click-
// to-close handler. Without this, the very click that brings the app
// back to the front also immediately closes the sheet it's meant to
// return to. Swallow backdrop clicks for a brief window after regaining
// focus so only a deliberate click closes it.
function useOverlayClose(onClose: () => void) {
  const suppressRef = useRef(false);
  useEffect(() => {
    function onWindowFocus() {
      suppressRef.current = true;
      setTimeout(() => {
        suppressRef.current = false;
      }, 400);
    }
    window.addEventListener("focus", onWindowFocus);
    return () => window.removeEventListener("focus", onWindowFocus);
  }, []);
  return () => {
    if (!suppressRef.current) onClose();
  };
}

const CLOUD_PROVIDER_LABELS: Record<string, string> = {
  drive: "Google Drive",
  onedrive: "OneDrive",
  dropbox: "Dropbox",
};

export function DriveSyncSheet({
  localPath,
  provider,
  onClose,
}: {
  localPath: string;
  provider: string;
  onClose: () => void;
}) {
  const providerLabel = CLOUD_PROVIDER_LABELS[provider] ?? provider;
  const [installed, setInstalled] = useState<boolean | null>(null);
  const [connected, setConnected] = useState<boolean | null>(null);
  const [pair, setPair] = useState<import("../../api").SyncPair | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [authUrl, setAuthUrl] = useState("");
  const [launchError, setLaunchError] = useState("");

  async function refreshState() {
    try {
      const isInstalled = await withTimeout(api.rcloneInstalled(), 5000, "Checking rclone");
      setInstalled(isInstalled);
      if (!isInstalled) return;
      const [isConn, pairs] = await withTimeout(
        Promise.all([api.rcloneIsConnected(provider), api.driveListPairs()]),
        8000,
        `Checking ${providerLabel} connection`
      );
      setConnected(isConn);
      setPair(pairs.find((p) => p.local_path === localPath && p.provider === provider) ?? null);
    } catch (e) {
      setError(String(e));
    }
  }
  useEffect(() => {
    refreshState();
  }, []);

  // rcloneConnect() can't be truly cancelled once it's called (Tauri
  // invoke has no abort signal, and the backend wait is a blocking OS
  // thread) -- this just stops the *UI* from waiting on it. The backend
  // call still finishes (or is abandoned once rclone's own local server
  // gives up) on its own in the background; its result is simply ignored
  // once cancelled.
  const connectCancelledRef = useRef(false);
  async function connect() {
    connectCancelledRef.current = false;
    setBusy(true);
    setError("");
    setAuthUrl("");
    setLaunchError("");
    setStatus("Waiting for sign-in in your browser…");
    try {
      // rclone tries to open the browser itself, but that's best-effort
      // (xdg-open/the OS default-browser association can fail for reasons
      // entirely outside this app's control) -- this channel gets the
      // real URL the moment rclone prints it, so there's always a manual
      // fallback instead of a spinner with no way to tell whether
      // anything happened. A second message prefixed "LAUNCH_FAILED:"
      // arrives if every launcher this app itself then tried as a backup
      // (xdg-open, gio open, gnome-open) also failed, carrying each one's
      // real error.
      const urlChannel = new Channel<string>();
      urlChannel.onmessage = (msg) => {
        if (connectCancelledRef.current) return;
        if (msg.startsWith("LAUNCH_FAILED:")) {
          setLaunchError(msg.slice("LAUNCH_FAILED:".length));
        } else {
          setAuthUrl(msg);
        }
      };
      await api.rcloneConnect(provider, urlChannel);
      if (connectCancelledRef.current) return;
      await refreshState();
      setStatus("");
      setAuthUrl("");
    } catch (e) {
      if (connectCancelledRef.current) return;
      setError(String(e));
      setStatus("");
    }
    if (!connectCancelledRef.current) setBusy(false);
  }
  function cancelConnect() {
    connectCancelledRef.current = true;
    setBusy(false);
    setStatus("");
    setAuthUrl("");
    setError("Cancelled. If a browser tab is still open, you can close it.");
  }
  async function link() {
    setBusy(true);
    setError("");
    try {
      const p = await api.driveAddPair(provider, localPath);
      setPair(p);
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }
  async function syncNow() {
    setBusy(true);
    setError("");
    setStatus("Syncing…");
    try {
      const report = await api.driveSyncNow(localPath);
      setStatus(report.summary);
    } catch (e) {
      setError(String(e));
      setStatus("");
    }
    setBusy(false);
  }
  async function unlink() {
    setBusy(true);
    setError("");
    try {
      await api.driveRemovePair(localPath);
      setPair(null);
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  const overlayClose = useOverlayClose(onClose);
  return (
    <div className="sheet-overlay" onMouseDown={overlayClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Sync with {providerLabel}</h3>
        {installed === null ? (
          <p>Loading…</p>
        ) : !installed ? (
          <p className="error">
            <code>rclone</code> isn't installed -- this app uses it to talk to {providerLabel}
            (no OAuth setup of your own needed). Install it (e.g. <code>sudo apt install rclone</code>)
            and reopen this.
          </p>
        ) : !connected ? (
          <p>Connect your {providerLabel} account to continue -- a browser window will open to sign in.</p>
        ) : !pair ? (
          <p>Connected. Link this folder to a paired {providerLabel} folder to start syncing.</p>
        ) : (
          <p>
            Linked to {providerLabel} folder “{pair.drive_folder_name}”. Sync is two-way (via{" "}
            <code>rclone bisync</code>): changes on either side propagate to the other.
          </p>
        )}
        {status && <p className="error" style={{ color: "var(--text-2)" }}>{status}</p>}
        {launchError && (
          <p className="hint" style={{ marginTop: -4 }}>
            Automatic browser launch failed ({launchError}) -- copy the link below and open it
            yourself, or fix this on your system (check <code>xdg-settings get
            default-web-browser</code> and that its .desktop entry is valid).
          </p>
        )}
        {authUrl && (
          <div className="info-row">
            <span>Browser didn't open?</span>
            <button
              className="btn-plain small"
              onClick={() => {
                openUrl(authUrl);
                navigator.clipboard.writeText(authUrl).catch(() => {});
              }}
            >
              Open manually
            </button>
          </div>
        )}
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Close
          </button>
          {!installed ? null : !connected ? (
            busy ? (
              <button className="btn-plain danger" onClick={cancelConnect}>
                Cancel
              </button>
            ) : (
              <button className="btn-primary" onClick={connect}>
                Connect
              </button>
            )
          ) : !pair ? (
            <button className="btn-primary" disabled={busy} onClick={link}>
              Link Folder
            </button>
          ) : (
            <>
              <button className="btn-plain danger" disabled={busy} onClick={unlink}>
                Unlink
              </button>
              <button className="btn-primary" disabled={busy} onClick={syncNow}>
                Sync Now
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// Git sync assumes the user already has git (and, for a remote like
// git@github.com:..., SSH auth) set up on this machine already -- unlike
// Drive, there's no OAuth flow here, just a remote URL to an empty repo
// they've already created.
export function GitSyncSheet({ localPath, onClose }: { localPath: string; onClose: () => void }) {
  const [pair, setPair] = useState<import("../../api").GitSyncPair | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [remoteUrlInput, setRemoteUrlInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function refreshState() {
    const pairs = await api.gitSyncListPairs();
    setPair(pairs.find((p) => p.local_path === localPath) ?? null);
    setLoaded(true);
  }
  useEffect(() => {
    refreshState();
  }, []);

  async function startSyncing() {
    if (remoteUrlInput.trim() === "") return;
    setBusy(true);
    setError("");
    try {
      // Repo "name" is just a label for the Get Info-style status rows
      // below -- the folder's own name is a fine default, no need to make
      // the user type it separately.
      await api.gitSyncAdd(localPath, remoteUrlInput.trim(), baseName(localPath));
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }
  async function unsync() {
    setBusy(true);
    setError("");
    try {
      await api.gitSyncRemove(localPath);
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  const overlayClose = useOverlayClose(onClose);
  return (
    <div className="sheet-overlay" onMouseDown={overlayClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Sync with Git</h3>
        {!loaded ? (
          <p>Loading…</p>
        ) : !pair ? (
          <>
            <p>
              Paste the URL of a remote repo you've already created (this assumes git -- and SSH
              auth for a <code>git@…</code> URL -- is already set up on this machine). Every ~25
              seconds it commits and pushes local changes, and pulls remote ones.
            </p>
            <input
              autoFocus
              placeholder="git@github.com:you/repo.git"
              value={remoteUrlInput}
              onChange={(e) => setRemoteUrlInput(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && startSyncing()}
            />
          </>
        ) : (
          <div className="info-rows">
            <div className="info-row">
              <span>Repo</span>
              <span>{pair.repo_name}</span>
            </div>
            <div className="info-row">
              <span>Remote</span>
              <span className="info-path" title={pair.remote_url}>
                {pair.remote_url}
              </span>
            </div>
            <div className="info-row">
              <span>Status</span>
              <span>Syncing every ~25s</span>
            </div>
          </div>
        )}
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Close
          </button>
          {!pair ? (
            <button className="btn-primary" disabled={busy} onClick={startSyncing}>
              Start Syncing
            </button>
          ) : (
            <button className="btn-plain danger" disabled={busy} onClick={unsync}>
              Unsync
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

// Bidirectional sync between this folder and a second, genuinely separate
// local folder (own copy of every file on each side, not a symlink) via
// the `unison` CLI -- see local_sync.rs for why that's the right tool and
// how its `-batch` mode behaves. `folderA` is fixed (whatever favorite/
// folder this sheet was opened from); the user only picks `folderB`.
export function LocalSyncSheet({ folderA, onClose }: { folderA: string; onClose: () => void }) {
  const [available, setAvailable] = useState<boolean | null>(null);
  const [pair, setPair] = useState<import("../../api").LocalSyncPair | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [folderBInput, setFolderBInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");

  async function refreshState() {
    const [isAvailable, pairs] = await Promise.all([api.localSyncAvailable(), api.localSyncListPairs()]);
    setAvailable(isAvailable);
    setPair(pairs.find((p) => p.folder_a === folderA || p.folder_b === folderA) ?? null);
    setLoaded(true);
  }
  useEffect(() => {
    refreshState();
  }, []);

  function reportConflicts(conflicts: string[]) {
    setStatus(
      conflicts.length === 0
        ? "Synced."
        : `Synced, except ${conflicts.length} file${conflicts.length === 1 ? "" : "s"} changed on ` +
            `both sides since last time -- left untouched, resolve by hand: ${conflicts.join(", ")}`
    );
  }

  async function pickFolderB() {
    const picked = await pickPath({ directory: true, multiple: false, title: "Choose the other folder" });
    if (typeof picked === "string") setFolderBInput(picked);
  }
  // Straight to the picker on first open rather than making "Browse…" an
  // extra click every time -- there's nothing else useful to do on this
  // sheet until a second folder is chosen anyway.
  const autoPickedRef = useRef(false);
  useEffect(() => {
    if (!loaded || available !== true || pair || autoPickedRef.current) return;
    autoPickedRef.current = true;
    pickFolderB();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, available, pair]);

  async function startSyncing() {
    if (folderBInput.trim() === "") return;
    setBusy(true);
    setError("");
    setStatus("");
    try {
      const conflicts = await api.localSyncAdd(folderA, folderBInput.trim());
      await refreshState();
      reportConflicts(conflicts);
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }
  async function syncNow() {
    if (!pair) return;
    setBusy(true);
    setError("");
    setStatus("Syncing…");
    try {
      const conflicts = await api.localSyncNow(pair.folder_a, pair.folder_b);
      reportConflicts(conflicts);
    } catch (e) {
      setError(String(e));
      setStatus("");
    }
    setBusy(false);
  }
  async function unsync() {
    if (!pair) return;
    setBusy(true);
    setError("");
    try {
      await api.localSyncRemove(pair.folder_a, pair.folder_b);
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  const otherFolder = pair ? (pair.folder_a === folderA ? pair.folder_b : pair.folder_a) : null;
  const overlayClose = useOverlayClose(onClose);
  return (
    <div className="sheet-overlay" onMouseDown={overlayClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Sync with Local Folder</h3>
        {!loaded ? (
          <p>Loading…</p>
        ) : available === false ? (
          <p className="error">
            <code>unison</code> isn't installed -- this needs it for real bidirectional sync
            (propagates changes both ways, flags true conflicts instead of guessing). Install it
            (e.g. <code>sudo apt install unison</code>) and reopen this.
          </p>
        ) : !pair ? (
          <>
            <p>
              Pick a second folder -- a real, separate one, not a shortcut. Every ~25 seconds,
              whatever changed on either side gets reflected on the other; a file edited on both
              sides since the last sync is left untouched and reported, not guessed at.
            </p>
            <div className="info-row">
              <input
                autoFocus
                placeholder="/path/to/other/folder"
                value={folderBInput}
                onChange={(e) => setFolderBInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && startSyncing()}
              />
              <button className="btn-plain small" onClick={pickFolderB}>
                Browse…
              </button>
            </div>
          </>
        ) : (
          <div className="info-rows">
            <div className="info-row">
              <span>Other folder</span>
              <span className="info-path" title={otherFolder ?? ""}>
                {otherFolder}
              </span>
            </div>
            <div className="info-row">
              <span>Status</span>
              <span>Syncing every ~25s</span>
            </div>
          </div>
        )}
        {status && <p className="hint">{status}</p>}
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Close
          </button>
          {available === false ? null : !pair ? (
            <button className="btn-primary" disabled={busy} onClick={startSyncing}>
              Start Syncing
            </button>
          ) : (
            <>
              <button className="btn-plain" disabled={busy} onClick={syncNow}>
                Sync Now
              </button>
              <button className="btn-plain danger" disabled={busy} onClick={unsync}>
                Unsync
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// A stable-ish Syncthing folder ID derived from this folder's own local
// path -- Syncthing folder "IDs" just need to match on both sides of a
// pairing (the actual storage location is chosen independently by each
// side), so a short hash of the path this device knows it by is enough;
// there's no coordination needed to invent one; the readable prefix is
// purely so it's recognizable in Syncthing's own GUI/logs if the user
// ever looks.
function stableFolderId(path: string): string {
  let hash = 0;
  for (let i = 0; i < path.length; i++) {
    hash = (hash * 31 + path.charCodeAt(i)) | 0;
  }
  const base = baseName(path).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "folder";
  return `${base}-${(hash >>> 0).toString(36)}`;
}

/// The "share a link" half of P2P device pairing: a `vaultexplorer://
/// add-device?id=...&name=...` URL, meant to be sent via WhatsApp/etc.
/// rather than scanned -- opening it launches VaultExplorer (or wakes an
/// already-running one, once single-instance forwarding is in place) with
/// this device's ID prefilled to add. QR pairing encodes the same link,
/// so a phone's camera and a pasted link both resolve the same way.
// A stuck local daemon call (or anything else that never settles) must
// never leave a sheet stuck on "Loading…" forever -- races the real call
// against a plain timer so the UI always resolves one way or another.
function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => setTimeout(() => reject(new Error(`${label} timed out`)), ms)),
  ]);
}

function buildAddDeviceLink(id: string): string {
  const params = new URLSearchParams({ id });
  return `vaultexplorer://add-device?${params.toString()}`;
}

// Exported: also used by App.tsx's Explorer to resolve an incoming
// `vaultexplorer://add-device` deep link (app launched by/woken by one).
export function parseAddDeviceLink(url: string): { id: string; name: string } | null {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "vaultexplorer:" || parsed.hostname !== "add-device") return null;
    const id = parsed.searchParams.get("id");
    if (!id) return null;
    return { id, name: parsed.searchParams.get("name") || id };
  } catch {
    return null;
  }
}

// Device-to-device sync via a Syncthing daemon this app manages itself --
// see syncthing.rs for why (peer-to-peer, no cloud account, no OAuth, no
// registering anything with anyone) and how the REST wrapper was verified
// (two real disposable daemons, full pairing + pending-folder-accept +
// actual file sync, before any of this frontend was written).
export function SyncthingSheet({ folderA, onClose }: { folderA: string; onClose: () => void }) {
  const [available, setAvailable] = useState<boolean | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [myId, setMyId] = useState("");
  const [qrSvg, setQrSvg] = useState("");
  const [devices, setDevices] = useState<import("../../api").SyncthingDevice[]>([]);
  const [folders, setFolders] = useState<import("../../api").SyncthingFolder[]>([]);
  const [pendingDevices, setPendingDevices] = useState<import("../../api").SyncthingPendingDevice[]>([]);
  const [pendingFolders, setPendingFolders] = useState<import("../../api").SyncthingPendingFolder[]>([]);
  const [deviceIdInput, setDeviceIdInput] = useState("");
  const [deviceNameInput, setDeviceNameInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function refreshState() {
    try {
      const isAvailable = await withTimeout(api.syncthingInstalled(), 8000, "Checking syncthing");
      setAvailable(isAvailable);
      if (!isAvailable) {
        setLoaded(true);
        return;
      }
      const [id, devs, fols, pDevs, pFols] = await withTimeout(
        Promise.all([
          api.syncthingMyDeviceId(),
          api.syncthingListDevices(),
          api.syncthingListFolders(),
          api.syncthingPendingDevices(),
          api.syncthingPendingFolders(),
        ]),
        8000,
        "Talking to syncthing"
      );
      setMyId(id);
      setDevices(devs);
      setFolders(fols);
      setPendingDevices(pDevs);
      setPendingFolders(pFols);
    } catch (e) {
      setError(String(e));
    }
    setLoaded(true);
  }
  useEffect(() => {
    refreshState();
    // Live-ish view of connection/pending status while the sheet's open,
    // without needing a push mechanism from the daemon.
    const interval = setInterval(refreshState, 5000);
    return () => clearInterval(interval);
  }, []);
  useEffect(() => {
    if (!myId) return;
    api
      .syncthingQrSvg(buildAddDeviceLink(myId))
      .then(setQrSvg)
      .catch(() => setQrSvg(""));
  }, [myId]);

  async function addDevice() {
    if (deviceIdInput.trim() === "") return;
    setBusy(true);
    setError("");
    try {
      await api.syncthingAddDevice(deviceIdInput.trim(), deviceNameInput.trim() || deviceIdInput.trim());
      setDeviceIdInput("");
      setDeviceNameInput("");
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }
  async function removeDevice(id: string) {
    setBusy(true);
    setError("");
    try {
      await api.syncthingRemoveDevice(id);
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }
  async function acceptPendingDevice(id: string) {
    setBusy(true);
    setError("");
    try {
      await api.syncthingAddDevice(id, id);
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }
  async function acceptPendingFolder(pf: import("../../api").SyncthingPendingFolder) {
    const picked = await pickPath({ directory: true, multiple: false, title: `Where should "${pf.label}" live?` });
    if (typeof picked !== "string") return;
    setBusy(true);
    setError("");
    try {
      await api.syncthingShareFolder(pf.id, pf.label, picked, [pf.offered_by_device_id]);
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  const thisFolder = folders.find((f) => f.path === folderA);
  const thisFolderId = thisFolder?.id ?? stableFolderId(folderA);
  async function toggleShareWithDevice(deviceId: string) {
    setBusy(true);
    setError("");
    try {
      const current = thisFolder?.device_ids ?? [];
      const next = current.includes(deviceId) ? current.filter((d) => d !== deviceId) : [...current, deviceId];
      if (next.length === 0) {
        await api.syncthingRemoveFolder(thisFolderId);
      } else {
        await api.syncthingShareFolder(thisFolderId, baseName(folderA), folderA, next);
      }
      await refreshState();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  const overlayClose = useOverlayClose(onClose);
  return (
    <div className="sheet-overlay" onMouseDown={overlayClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Sync P2P</h3>
        {!loaded ? (
          <p>Loading…</p>
        ) : available === false ? (
          <p className="error">
            <code>syncthing</code> isn't installed -- this app runs its own dedicated instance for
            direct device-to-device sync, no cloud account of any kind involved. Install it (e.g.{" "}
            <code>sudo apt install syncthing</code>) and reopen this.
          </p>
        ) : (
          <>
            <label className="field-label">Pair a new device</label>
            <p className="hint">Scan this on the other device, or share the link below.</p>
            {qrSvg && (
              <div className="qr-code" dangerouslySetInnerHTML={{ __html: qrSvg }} />
            )}
            <div className="info-row">
              <span className="info-path" title={myId}>
                {myId}
              </span>
              <button className="btn-plain small" onClick={() => navigator.clipboard.writeText(myId)}>
                Copy ID
              </button>
            </div>
            <div className="info-row">
              <span className="info-path">Shareable link (WhatsApp, etc.)</span>
              <button
                className="btn-plain small"
                onClick={() => navigator.clipboard.writeText(buildAddDeviceLink(myId))}
              >
                Copy Link
              </button>
            </div>

            {pendingDevices.length > 0 && (
              <>
                <label className="field-label">Wants to connect</label>
                {pendingDevices.map((pd) => (
                  <div className="info-row" key={pd.id}>
                    <span className="info-path" title={pd.id}>
                      {pd.id}
                    </span>
                    <button className="btn-plain small" disabled={busy} onClick={() => acceptPendingDevice(pd.id)}>
                      Accept
                    </button>
                  </div>
                ))}
              </>
            )}

            {pendingFolders.length > 0 && (
              <>
                <label className="field-label">Offered folders</label>
                {pendingFolders.map((pf) => (
                  <div className="info-row" key={`${pf.id}-${pf.offered_by_device_id}`}>
                    <span>{pf.label}</span>
                    <button className="btn-plain small" disabled={busy} onClick={() => acceptPendingFolder(pf)}>
                      Accept…
                    </button>
                  </div>
                ))}
              </>
            )}

            <label className="field-label">Paired devices</label>
            {devices.length === 0 && <p className="hint">None yet -- add one below.</p>}
            {devices.map((d) => (
              <div className="info-row" key={d.id}>
                <span>
                  {d.connected ? "🟢" : "⚪"} {d.name}
                </span>
                <label className="checkbox-row" style={{ margin: 0 }}>
                  <input
                    type="checkbox"
                    checked={!!thisFolder?.device_ids.includes(d.id)}
                    disabled={busy}
                    onChange={() => toggleShareWithDevice(d.id)}
                  />
                  Share this folder
                </label>
                <button className="btn-plain small danger" disabled={busy} onClick={() => removeDevice(d.id)}>
                  Remove
                </button>
              </div>
            ))}

            <label className="field-label">Add a device</label>
            <input
              placeholder="Device ID (from the other side)"
              value={deviceIdInput}
              onChange={(e) => setDeviceIdInput(e.target.value)}
            />
            <input
              placeholder="Name (optional)"
              value={deviceNameInput}
              onChange={(e) => setDeviceNameInput(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && addDevice()}
            />
          </>
        )}
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Close
          </button>
          {available !== false && (
            <button className="btn-primary" disabled={busy || deviceIdInput.trim() === ""} onClick={addDevice}>
              Add Device
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
