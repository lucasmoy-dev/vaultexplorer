import { useEffect, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { api, osOpen, formatSize, joinPath } from "../../api";
import { PHONE_STORAGE_PATH } from "../../constants";
import { ComputerGlyph, ChevronLeft } from "../../icons";
import { Dropdown } from "../../ContextMenu";
import { RecoverySheet, UnfreezeSheet } from "./vault-sheets";
import {
  SensitiveTimeout,
  SENSITIVE_TIMEOUT_CHOICES,
  sensitiveTimeoutLabel,
} from "../../types";

// The same durations the unlock sheet's picker offers (see types.ts) -- the
// sheet preselects whatever is configured here, so the two lists have to be
// one list. A value saved under the older, longer list (e.g. 20 minutes) is
// appended so it still shows accurately instead of looking unset; picking
// anything else drops it.
function timeoutOptions(current: SensitiveTimeout): { value: SensitiveTimeout; label: string }[] {
  const known = SENSITIVE_TIMEOUT_CHOICES.some((o) => o.value === current);
  return known
    ? SENSITIVE_TIMEOUT_CHOICES
    : [...SENSITIVE_TIMEOUT_CHOICES, { value: current, label: sensitiveTimeoutLabel(current) }];
}

const RELEASES_API = "https://api.github.com/repos/lucasmoy-dev/vaultexplorer/releases/latest";
const RELEASES_PAGE = "https://github.com/lucasmoy-dev/vaultexplorer/releases/latest";

function isNewerVersion(latest: string, current: string): boolean {
  const a = latest.replace(/^v/, "").split(".").map(Number);
  const b = current.replace(/^v/, "").split(".").map(Number);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const diff = (a[i] || 0) - (b[i] || 0);
    if (diff !== 0) return diff > 0;
  }
  return false;
}

type ReleaseInfo = { latestVersion: string; apkUrl?: string };
type UpdateCheck = ReleaseInfo | null;

// GitHub's own release API, not a custom update-manifest server -- this is
// a small side project, not something that needs its own infrastructure
// just to answer "is there a newer version". No auth needed for a public
// repo's release info.
async function fetchLatestRelease(): Promise<ReleaseInfo> {
  const res = await fetch(RELEASES_API);
  if (!res.ok) throw new Error(`GitHub API returned ${res.status}`);
  const data = await res.json();
  const asset = (data.assets ?? []).find((a: { name: string }) => a.name.toLowerCase().endsWith(".apk"));
  return { latestVersion: String(data.tag_name ?? "").replace(/^v/, ""), apkUrl: asset?.browser_download_url };
}

export function ManageTemplatesSheet({
  templates,
  onRemove,
  onClose,
}: {
  templates: { id: string; label: string; storedName: string }[];
  onRemove: (id: string) => void;
  onClose: () => void;
}) {
  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Manage Templates</h3>
        {templates.length === 0 ? (
          <p>No templates yet — right-click a file and choose “Use as Template”.</p>
        ) : (
          <div className="info-rows">
            {templates.map((t) => (
              <div className="info-row" key={t.id}>
                <span>{t.label}</span>
                <button className="btn-plain" onClick={() => onRemove(t.id)}>
                  Remove
                </button>
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

export function MachineInfoSheet({ onClose }: { onClose: () => void }) {
  const [info, setInfo] = useState<import("../../api").MachineSummary | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    api.machineSummary().then(setInfo).catch((e) => setError(String(e)));
  }, []);

  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [advanced, setAdvanced] = useState<import("../../api").AdvancedInfo | null>(null);
  const [advancedError, setAdvancedError] = useState("");
  function toggleAdvanced() {
    setAdvancedOpen((open) => !open);
    if (!advanced) {
      api.machineAdvancedInfo().then(setAdvanced).catch((e) => setAdvancedError(String(e)));
    }
  }

  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateMsg, setUpdateMsg] = useState("");
  async function updateDrivers() {
    setUpdateBusy(true);
    setUpdateMsg("");
    try {
      await api.machineUpdateDrivers();
      setUpdateMsg("Started -- look for a system authentication prompt.");
    } catch (e) {
      setUpdateMsg(String(e));
    }
    setUpdateBusy(false);
  }

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card info-card machine-info-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="info-icon">
          <ComputerGlyph size={64} />
        </div>
        <h3 className="info-name">My Computer</h3>
        {error && <p className="error">{error}</p>}
        {info && (
          <div className="info-rows">
            <div className="info-row">
              <span>OS</span>
              <span>{info.os_name}</span>
            </div>
            <div className="info-row">
              <span>Processor</span>
              <span>{info.cpu_model}</span>
            </div>
            <div className="info-row">
              <span>Cores</span>
              <span>{info.cpu_cores}</span>
            </div>
            <div className="info-row">
              <span>Memory</span>
              <span>{formatSize(info.ram_total)}</span>
            </div>
            {info.disks.map((d) => (
              <div className="info-row" key={d.path}>
                <span>{d.label || d.model || d.name}</span>
                <span>
                  {formatSize(d.total)} ({formatSize(d.free)} free)
                </span>
              </div>
            ))}
          </div>
        )}

        <button className="btn-plain small advanced-toggle" onClick={toggleAdvanced}>
          {advancedOpen ? "Hide" : "Show"} Advanced
        </button>

        {advancedOpen && (
          <div className="advanced-options">
            {advancedError && <p className="error">{advancedError}</p>}
            {!advanced && !advancedError && <p className="hint">Loading…</p>}
            {advanced && (
              <>
                <div className="info-rows">
                  <div className="info-row">
                    <span>Model</span>
                    <span>{advanced.product_name}</span>
                  </div>
                  <div className="info-row">
                    <span>Motherboard</span>
                    <span>
                      {advanced.board_vendor} {advanced.board_name} (rev {advanced.board_version})
                    </span>
                  </div>
                  <div className="info-row">
                    <span>BIOS</span>
                    <span>
                      {advanced.bios_vendor} {advanced.bios_version}
                    </span>
                  </div>
                  {advanced.pci_devices.map((d) => (
                    <div className="info-row" key={d.address}>
                      <span>
                        {d.kind === "wifi" ? "Wi-Fi" : d.kind === "gpu" ? "Graphics" : "Ethernet"}
                      </span>
                      <span className={d.driver ? "" : "error"}>
                        {d.description}
                        {" — "}
                        {d.driver ? `driver: ${d.driver}` : "no driver bound"}
                      </span>
                    </div>
                  ))}
                </div>

                {advanced.ubuntu_drivers_available ? (
                  <>
                    {advanced.driver_recommendations.length > 0 && (
                      <div className="info-rows" style={{ marginTop: 10 }}>
                        {advanced.driver_recommendations.map((r, i) => (
                          <div className="info-row" key={i}>
                            <span>{r.vendor}</span>
                            <span>{r.driver}</span>
                          </div>
                        ))}
                      </div>
                    )}
                    <div className="sheet-actions" style={{ marginTop: 10, marginBottom: 4 }}>
                      <button className="btn-plain" disabled={updateBusy} onClick={updateDrivers}>
                        {updateBusy ? "Starting…" : "Update Drivers…"}
                      </button>
                    </div>
                    {updateMsg && (
                      <p style={{ fontSize: 11.5, color: "var(--text-2)" }}>{updateMsg}</p>
                    )}
                  </>
                ) : (
                  <p className="hint">
                    Install <code>ubuntu-drivers-common</code> to check for driver updates.
                  </p>
                )}
              </>
            )}
          </div>
        )}

        <div className="sheet-actions">
          <button className="btn-primary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

const FORMAT_FS_TYPES = [
  { key: "exfat", label: "exFAT" },
  { key: "vfat", label: "FAT32" },
  { key: "ntfs", label: "NTFS" },
  { key: "ext4", label: "ext4" },
] as const;

// Deliberately requires typing the exact device path to enable the
// button -- a drive-format is as destructive as it gets, and a plain
// confirm dialog is too easy to click through on autopilot.
export function FormatDriveSheet({
  drive,
  onClose,
  onFormatted,
}: {
  drive: import("../../api").Drive;
  onClose: () => void;
  onFormatted: () => void;
}) {
  const [fsType, setFsType] = useState<(typeof FORMAT_FS_TYPES)[number]["key"]>("exfat");
  const [label, setLabel] = useState(drive.label ?? "");
  const [confirmText, setConfirmText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const canFormat = confirmText === drive.path && !busy;

  async function submit() {
    if (!canFormat) return;
    setBusy(true);
    setError("");
    try {
      await api.machineFormatDrive(drive.path, fsType, label);
      onFormatted();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Format {drive.label || drive.name}</h3>
        <p>
          <strong>This permanently erases everything on {drive.path}</strong>
          {drive.mountpoint ? ` (currently mounted at ${drive.mountpoint})` : ""}. This cannot be
          undone.
        </p>
        <label className="field-label">Filesystem</label>
        <div className="segmented compress-level">
          {FORMAT_FS_TYPES.map((t) => (
            <button
              key={t.key}
              className={`seg seg-text ${fsType === t.key ? "on" : ""}`}
              onClick={() => setFsType(t.key)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <label className="field-label">Volume name</label>
        <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Untitled" />
        <label className="field-label">
          Type <code>{drive.path}</code> to confirm
        </label>
        <input
          value={confirmText}
          onChange={(e) => setConfirmText(e.target.value)}
          placeholder={drive.path}
        />
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn-primary danger" disabled={!canFormat} onClick={submit}>
            {busy ? "Formatting…" : "Format"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function SettingsScreen({
  settings,
  onChange,
  onClose,
  mobile,
  onExportConfig,
  onImportConfig,
}: {
  settings: {
    showHiddenFiles: boolean;
    hideExtensions: boolean;
    terminalApp: string;
    newFileNameTemplate: string;
    newFolderNameTemplate: string;
    theme: "light" | "dark" | "system";
    sensitiveTimeout: SensitiveTimeout;
    mobileExternalEditor: boolean;
  };
  mobile: boolean;
  onChange: (next: {
    showHiddenFiles: boolean;
    hideExtensions: boolean;
    terminalApp: string;
    newFileNameTemplate: string;
    newFolderNameTemplate: string;
    theme: "light" | "dark" | "system";
    sensitiveTimeout: SensitiveTimeout;
    mobileExternalEditor: boolean;
  }) => void;
  onClose: () => void;
  onExportConfig: (includeCloud: boolean) => void;
  onImportConfig: () => void;
}) {
  const [tab, setTab] = useState<"general" | "security" | "system">("general");
  const [includeCloudCreds, setIncludeCloudCreds] = useState(false);
  const defaultContactsDir = joinPath(joinPath(PHONE_STORAGE_PATH, "Documents"), "Contacts");
  const [exportDir, setExportDir] = useState(defaultContactsDir);
  const [importDir, setImportDir] = useState(defaultContactsDir);
  const [contactsBusy, setContactsBusy] = useState(false);
  const [contactsMsg, setContactsMsg] = useState("");
  const [appVersion, setAppVersion] = useState("");
  // Mobile Google Drive sync (see drive_rest.rs). Signing in lives here
  // rather than in the per-folder sheet because it's a one-time setup
  // step with credentials to paste, not something to redo per folder.
  const [driveConn, setDriveConn] = useState<import("../../api").DriveConnection | null>(null);
  const [drivePairs, setDrivePairs] = useState<import("../../api").MobileSyncPair[]>([]);
  const [driveClientId, setDriveClientId] = useState("");
  const [driveClientSecret, setDriveClientSecret] = useState("");
  const [driveBusy, setDriveBusy] = useState(false);
  const [driveMsg, setDriveMsg] = useState("");
  const [driveAuthUrl, setDriveAuthUrl] = useState("");
  const [updateCheck, setUpdateCheck] = useState<UpdateCheck>(null);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateMsg, setUpdateMsg] = useState("");
  useEffect(() => {
    getVersion()
      .then(setAppVersion)
      .catch(() => {});
  }, []);
  useEffect(() => {
    if (!mobile) return;
    refreshDrive();
  }, [mobile]);
  async function refreshDrive() {
    try {
      const [conn, pairs] = await Promise.all([api.driveRestConnection(), api.driveRestListPairs()]);
      setDriveConn(conn);
      setDrivePairs(pairs);
    } catch (e) {
      setDriveMsg(String(e));
    }
  }
  async function connectDrive() {
    setDriveBusy(true);
    setDriveMsg(
      "Waiting for the Google sign-in in your browser… when it says you can close the tab, switch " +
        "back here. If the page seems to hang right at the end, switching back is what finishes it: " +
        "Android pauses this app -- and the local sign-in endpoint it is serving -- while the " +
        "browser is in front."
    );
    setDriveAuthUrl("");
    try {
      // The backend serves the OAuth redirect on 127.0.0.1 and hands the
      // consent URL back through this channel the moment it's built, so
      // there's always a tappable link even when handing the URL to a
      // browser fails.
      const channel = new Channel<string>();
      channel.onmessage = (url) => setDriveAuthUrl(url);
      const email = await api.driveRestConnect(driveClientId, driveClientSecret, channel);
      setDriveMsg(email ? `Connected as ${email}.` : "Connected.");
      setDriveAuthUrl("");
      setDriveClientSecret("");
      await refreshDrive();
    } catch (e) {
      setDriveMsg(String(e));
    } finally {
      setDriveBusy(false);
    }
  }
  async function disconnectDrive() {
    setDriveBusy(true);
    try {
      await api.driveRestDisconnect();
      setDriveMsg("Disconnected. Linked folders stay linked but won't sync until you connect again.");
      await refreshDrive();
    } catch (e) {
      setDriveMsg(String(e));
    } finally {
      setDriveBusy(false);
    }
  }
  async function syncPairNow(localPath: string) {
    setDriveBusy(true);
    setDriveMsg("Syncing…");
    try {
      const channel = new Channel<import("../../api").ProgressEvent>();
      channel.onmessage = (e) => {
        if (e.total > 1) setDriveMsg(`Syncing… ${e.done}/${e.total}`);
      };
      const outcome = await api.driveRestSyncNow(localPath, channel);
      setDriveMsg(outcome.summary);
    } catch (e) {
      setDriveMsg(String(e));
    } finally {
      setDriveBusy(false);
    }
  }
  async function unlinkPair(localPath: string) {
    setDriveBusy(true);
    try {
      await api.driveRestRemovePair(localPath);
      await refreshDrive();
      setDriveMsg("Unlinked.");
    } catch (e) {
      setDriveMsg(String(e));
    } finally {
      setDriveBusy(false);
    }
  }
  async function checkForUpdate() {
    setUpdateBusy(true);
    setUpdateMsg("");
    try {
      const result = await fetchLatestRelease();
      setUpdateCheck(result);
      if (!isNewerVersion(result.latestVersion, appVersion)) {
        setUpdateMsg(`You're up to date (v${appVersion}).`);
      }
    } catch (e) {
      setUpdateMsg(String(e));
    } finally {
      setUpdateBusy(false);
    }
  }
  async function installUpdate() {
    if (!updateCheck?.apkUrl) return;
    setUpdateBusy(true);
    setUpdateMsg("");
    try {
      // Without "install unknown apps" enabled for this app, starting the
      // install intent previously just... did nothing visible -- no
      // exception, no prompt, the button looked broken. Same two-step
      // dance as "All files access": check, and if missing, open the
      // dedicated settings screen instead of trying (and silently
      // failing) anyway.
      const canInstall = await api.androidCanInstallPackages();
      if (!canInstall) {
        await api.androidRequestInstallPackagesAccess();
        setUpdateMsg('Enable "Allow from this source" for Vault Explorer (just opened in Settings), then tap Update again.');
        return;
      }
      setUpdateMsg("Downloading update…");
      await api.androidDownloadAndInstallApk(updateCheck.apkUrl);
      setUpdateMsg("Confirm the install prompt to finish.");
    } catch (e) {
      setUpdateMsg(String(e));
    } finally {
      setUpdateBusy(false);
    }
  }
  async function withContactsPermission(action: () => Promise<void>) {
    setContactsBusy(true);
    setContactsMsg("");
    try {
      const contactsGranted = await api.androidContactsPermissionGranted();
      if (!contactsGranted) {
        await api.androidRequestContactsPermission();
        setContactsMsg("Grant the permission in the dialog, then try again.");
        return;
      }
      // The export/import folders default to a real Phone Storage path
      // (see exportDir/importDir below) -- writing/reading .vcf files
      // there needs "All files access" too, same as any other real-fs
      // folder (see `go()` in App.tsx). Missing this check meant export
      // failed with a raw "Permission denied (os error 13)" from the
      // write itself instead of ever prompting for the permission.
      const storageGranted = await api.androidStorageAccessGranted();
      if (!storageGranted) {
        await api.androidRequestStorageAccess();
        setContactsMsg('Also grant "All files access" (just opened in Settings) so contacts can be written to your phone storage, then try again.');
        return;
      }
      await action();
    } catch (e) {
      setContactsMsg(String(e));
    } finally {
      setContactsBusy(false);
    }
  }
  async function exportContacts() {
    await withContactsPermission(async () => {
      const { exported, failed_names } = await api.androidExportContacts(exportDir);
      let msg = `Exported ${exported} contact${exported === 1 ? "" : "s"} to ${exportDir}`;
      if (failed_names.length > 0) {
        msg += ` -- failed: ${failed_names.join("; ")}`;
      }
      setContactsMsg(msg);
    });
  }
  async function importContacts() {
    await withContactsPermission(async () => {
      const entries = await api.fsList(importDir, false);
      const vcfPaths = entries
        .filter((e) => !e.is_dir && e.name.toLowerCase().endsWith(".vcf"))
        .map((e) => joinPath(importDir, e.name));
      if (vcfPaths.length === 0) {
        setContactsMsg(`No .vcf files found in ${importDir}`);
        return;
      }
      await api.androidImportContacts(vcfPaths);
      setContactsMsg(`Opened ${vcfPaths.length} import prompt${vcfPaths.length === 1 ? "" : "s"} -- confirm each in the Contacts app`);
    });
  }
  const [portalEnabled, setPortalEnabled] = useState(false);
  const [portalBusy, setPortalBusy] = useState(false);
  const [portalError, setPortalError] = useState("");
  useEffect(() => {
    api.portalIsEnabled().then(setPortalEnabled).catch(() => {});
  }, []);

  const [defaultFmEnabled, setDefaultFmEnabled] = useState(false);
  const [defaultFmBusy, setDefaultFmBusy] = useState(false);
  const [defaultFmError, setDefaultFmError] = useState("");
  useEffect(() => {
    api.defaultFileManagerEnabled().then(setDefaultFmEnabled).catch(() => {});
  }, []);

  async function toggleDefaultFm(checked: boolean) {
    setDefaultFmBusy(true);
    setDefaultFmError("");
    try {
      await api.setDefaultFileManager(checked);
      // Re-read rather than assuming: the backend reports "on" only when
      // the desktop entry it wrote actually resolves, so a half-applied
      // change shows as off instead of lying about it.
      setDefaultFmEnabled(await api.defaultFileManagerEnabled());
    } catch (e) {
      setDefaultFmError(String(e));
    }
    setDefaultFmBusy(false);
  }

  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartError, setAutostartError] = useState("");
  useEffect(() => {
    api.autostartEnabled().then(setAutostartEnabled).catch(() => {});
  }, []);

  async function toggleAutostart(checked: boolean) {
    setAutostartBusy(true);
    setAutostartError("");
    try {
      await api.setAutostart(checked);
      setAutostartEnabled(checked);
    } catch (e) {
      setAutostartError(String(e));
    }
    setAutostartBusy(false);
  }

  const [recoveryAvailable, setRecoveryAvailable] = useState(false);
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  useEffect(() => {
    api.recoveryToolAvailable().then(setRecoveryAvailable).catch(() => {});
  }, []);

  const [frozen, setFrozen] = useState<import("../../api").FreezeMeta[]>([]);
  const [unfreezePath, setUnfreezePath] = useState<string | null>(null);
  const refreshFrozenList = () => api.listFrozenFolders().then(setFrozen).catch(() => setFrozen([]));
  useEffect(() => {
    refreshFrozenList();
  }, []);

  async function togglePortal(checked: boolean) {
    setPortalBusy(true);
    setPortalError("");
    try {
      if (checked) await api.portalEnable();
      else await api.portalDisable();
      setPortalEnabled(checked);
    } catch (e) {
      setPortalError(String(e));
    }
    setPortalBusy(false);
  }

  return (
    <div className="settings-screen">
      <div className="settings-screen-header">
        <button className="settings-back-btn" onClick={onClose} aria-label="Back">
          <ChevronLeft size={18} />
          Back
        </button>
        <h2>Settings</h2>
      </div>
      <div className="settings-tabs">
        {(["general", "security", ...(mobile ? [] : (["system"] as const))] as const).map((t) => (
          <button
            key={t}
            className={`settings-tab ${tab === t ? "on" : ""}`}
            onClick={() => setTab(t)}
          >
            {t === "general" ? "General" : t === "security" ? "Security" : "System"}
          </button>
        ))}
      </div>
      <div className="settings-screen-body">

        {tab === "general" && (
          <>
            <label className="field-label">Appearance</label>
            <div className="segmented compress-level" style={{ marginBottom: 16 }}>
              {(["light", "dark", "system"] as const).map((t) => (
                <button
                  key={t}
                  className={`seg seg-text ${settings.theme === t ? "on" : ""}`}
                  onClick={() => onChange({ ...settings, theme: t })}
                >
                  {t === "light" ? "Light" : t === "dark" ? "Dark" : "System"}
                </button>
              ))}
            </div>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={settings.showHiddenFiles}
                onChange={(e) => onChange({ ...settings, showHiddenFiles: e.target.checked })}
              />
              Show hidden files
            </label>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={settings.hideExtensions}
                onChange={(e) => onChange({ ...settings, hideExtensions: e.target.checked })}
              />
              Hide file extensions
            </label>
            {mobile && (
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={settings.mobileExternalEditor}
                  onChange={(e) =>
                    onChange({ ...settings, mobileExternalEditor: e.target.checked })
                  }
                />
                Open text files in an external app instead of the built-in editor
              </label>
            )}
            <label className="field-label">Default name for new files</label>
            <input
              value={settings.newFileNameTemplate}
              placeholder="untitled document"
              onChange={(e) => onChange({ ...settings, newFileNameTemplate: e.target.value })}
            />
            <label className="field-label">Default name for new folders</label>
            <input
              value={settings.newFolderNameTemplate}
              placeholder="untitled folder"
              onChange={(e) => onChange({ ...settings, newFolderNameTemplate: e.target.value })}
            />
            <p className="hint" style={{ marginTop: 6 }}>
              Use <code>{"{date}"}</code>, <code>{"{time}"}</code>, or <code>{"{datetime}"}</code> to
              include the current date/time, e.g. <code>{"{datetime}"}</code> → “2026-11-30 16.16hs”.
            </p>
            <label className="field-label" style={{ marginTop: 14 }}>
              Config
            </label>
            <p className="hint" style={{ marginTop: -2 }}>
              Favorites, appearance, and other settings -- as one blob on the clipboard, to carry
              over to another device. Pasting it here remaps any path from this device's home
              folder to the equivalent spot on this one.
            </p>
            {!mobile && (
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={includeCloudCreds}
                  onChange={(e) => setIncludeCloudCreds(e.target.checked)}
                />
                Include cloud sync credentials (sensitive -- puts live account access on the
                clipboard)
              </label>
            )}
            <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
              <button className="btn-plain small" onClick={() => onExportConfig(includeCloudCreds)}>
                Copy config to clipboard
              </button>
              <button className="btn-plain small" onClick={onImportConfig}>
                Paste config from clipboard
              </button>
            </div>
            {mobile && (
              <>
                <label className="field-label" style={{ marginTop: 14 }}>
                  Contacts
                </label>
                <p className="hint" style={{ marginTop: -2 }}>
                  Export writes one .vcf per contact. Import hands each .vcf in the folder to the
                  Contacts app to confirm.
                </p>
                <label className="field-label">Export folder</label>
                <input value={exportDir} onChange={(e) => setExportDir(e.target.value)} />
                <div style={{ display: "flex", gap: 8, marginTop: 6, marginBottom: 14 }}>
                  <button className="btn-plain small" disabled={contactsBusy} onClick={exportContacts}>
                    Export contacts
                  </button>
                </div>
                <label className="field-label">Import folder</label>
                <input value={importDir} onChange={(e) => setImportDir(e.target.value)} />
                <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
                  <button className="btn-plain small" disabled={contactsBusy} onClick={importContacts}>
                    Import contacts from folder
                  </button>
                </div>
                {contactsMsg && (
                  <p className="hint" style={{ marginTop: 6 }}>
                    {contactsMsg}
                  </p>
                )}
                <label className="field-label" style={{ marginTop: 14 }}>
                  Cloud &amp; folder sync
                </label>
                <p className="hint" style={{ marginTop: -2 }}>
                  Google Drive and folder-to-folder sync both work here: this app talks to Drive
                  directly and syncs two local folders itself, so nothing has to shell out to{" "}
                  <code>rclone</code> or <code>unison</code> (Android can run neither). Both are
                  two-way, and Drive uses the same <code>VaultExplorer/&lt;folder&gt;</code> folder
                  the desktop app does. Link either from a folder: long-press it → Sync. OneDrive,
                  Dropbox, Git and P2P stay desktop-only -- each needs its own binary or its own API
                  client, not a shared one.
                </p>
                {driveConn?.connected ? (
                  <>
                    <p className="hint" style={{ marginTop: 4 }}>
                      Connected{driveConn.account_email ? ` as ${driveConn.account_email}` : ""}. Link a
                      folder by long-pressing it → Sync → Google Drive.
                    </p>
                    {drivePairs.length > 0 && (
                      <div className="info-rows">
                        {drivePairs.map((p) => (
                          <div className="info-row" key={p.local_path}>
                            <span className="info-path" title={p.local_path}>
                              {p.local_path}
                            </span>
                            <span style={{ display: "flex", gap: 6 }}>
                              <button
                                className="btn-plain small"
                                disabled={driveBusy}
                                onClick={() => syncPairNow(p.local_path)}
                              >
                                Sync now
                              </button>
                              <button
                                className="btn-plain small"
                                disabled={driveBusy}
                                onClick={() => unlinkPair(p.local_path)}
                              >
                                Unlink
                              </button>
                            </span>
                          </div>
                        ))}
                      </div>
                    )}
                    <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
                      <button className="btn-plain small" disabled={driveBusy} onClick={disconnectDrive}>
                        Disconnect Google Drive
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <p className="hint" style={{ marginTop: 4 }}>
                      One-time setup: in the Google Cloud console, enable the Drive API and create an
                      OAuth client of type “Desktop app”, then paste its ID and secret here. Your own
                      client is deliberate -- a shared one is exactly what Google is retiring
                      rclone's built-in Drive client over, and an ID shipped inside an APK isn't a
                      secret anyway. Both values stay on this device.
                    </p>
                    <label className="field-label">Client ID</label>
                    <input
                      value={driveClientId}
                      placeholder="…apps.googleusercontent.com"
                      onChange={(e) => setDriveClientId(e.target.value)}
                    />
                    <label className="field-label">Client secret</label>
                    <input
                      type="password"
                      value={driveClientSecret}
                      onChange={(e) => setDriveClientSecret(e.target.value)}
                    />
                    <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
                      <button
                        className="btn-primary small"
                        disabled={driveBusy || !driveClientId.trim() || !driveClientSecret.trim()}
                        onClick={connectDrive}
                      >
                        Connect Google Drive
                      </button>
                      {driveAuthUrl && (
                        <button
                          className="btn-plain small"
                          onClick={() => osOpen(driveAuthUrl).catch((e) => setDriveMsg(String(e)))}
                        >
                          Open sign-in page
                        </button>
                      )}
                    </div>
                  </>
                )}
                {driveMsg && (
                  <p className="hint" style={{ marginTop: 6 }}>
                    {driveMsg}
                  </p>
                )}
              </>
            )}
            <label className="field-label" style={{ marginTop: 14 }}>
              Updates
            </label>
            <p className="hint" style={{ marginTop: -2 }}>
              {appVersion ? `Running v${appVersion}.` : "Checking current version…"}{" "}
              {mobile
                ? "Downloads and hands the APK to the system installer -- you'll still confirm the install yourself."
                : "Opens the release page to download."}
            </p>
            <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
              <button className="btn-plain small" disabled={updateBusy} onClick={checkForUpdate}>
                Check for updates
              </button>
              {updateCheck && isNewerVersion(updateCheck.latestVersion, appVersion) && (
                <button
                  className="btn-primary small"
                  disabled={updateBusy}
                  onClick={() =>
                    mobile
                      ? installUpdate()
                      : osOpen(RELEASES_PAGE).catch((e) => setUpdateMsg(String(e)))
                  }
                >
                  Update to v{updateCheck.latestVersion}
                </button>
              )}
            </div>
            {updateMsg && (
              <p className="hint" style={{ marginTop: 6 }}>
                {updateMsg}
              </p>
            )}
          </>
        )}

        {tab === "security" && (
          <>
            <label className="field-label">Sensitive files timeout</label>
            <Dropdown
              value={String(settings.sensitiveTimeout)}
              options={timeoutOptions(settings.sensitiveTimeout).map((o) => ({ value: String(o.value), label: o.label }))}
              onChange={(raw) => {
                const next: SensitiveTimeout =
                  raw === "never" ? "never" : (Number(raw) as SensitiveTimeout);
                onChange({ ...settings, sensitiveTimeout: next });
              }}
            />
            <p className="hint" style={{ marginTop: 6 }}>
              After viewing a file marked sensitive, it re-locks after this long and asks for the
              vault password again.
            </p>
          </>
        )}

        {tab === "system" && !mobile && (
          <>
            <label className="field-label">Terminal app (for "Open in Terminal")</label>
            <input
              list="terminal-app-options"
              value={settings.terminalApp}
              placeholder="ghostty"
              onChange={(e) => onChange({ ...settings, terminalApp: e.target.value })}
            />
            <datalist id="terminal-app-options">
              <option value="ghostty" />
              <option value="gnome-terminal" />
              <option value="konsole" />
              <option value="xterm" />
              <option value="alacritty" />
              <option value="kitty" />
            </datalist>
            <label className="checkbox-row" style={{ marginTop: 12 }}>
              <input
                type="checkbox"
                checked={portalEnabled}
                disabled={portalBusy}
                onChange={(e) => togglePortal(e.target.checked)}
              />
              Enable as system file picker
            </label>
            <p style={{ fontSize: 11.5, color: "var(--text-2)", marginTop: -6 }}>
              Only affects apps that use the desktop portal for Open/Save dialogs (Flatpak, Snap, or
              apps that opt in) — not every app's native dialog.
            </p>
            {portalError && <p className="error">{portalError}</p>}
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={defaultFmEnabled}
                disabled={defaultFmBusy}
                onChange={(e) => toggleDefaultFm(e.target.checked)}
              />
              Use as the default file manager
            </label>
            <p style={{ fontSize: 11.5, color: "var(--text-2)", marginTop: -6 }}>
              Folders opened from any other app come here instead of Files, and so does
              “Show in folder” from Chrome, OBS and the like.
            </p>
            {defaultFmError && <p className="error">{defaultFmError}</p>}
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={autostartEnabled}
                disabled={autostartBusy}
                onChange={(e) => toggleAutostart(e.target.checked)}
              />
              Start Vault Explorer at login
            </label>
            <p style={{ fontSize: 11.5, color: "var(--text-2)", marginTop: -6 }}>
              Combine with a vault's "Unlock automatically" (Vault Settings…) so it's ready to use
              right after you log in.
            </p>
            {autostartError && <p className="error">{autostartError}</p>}
            <div className="sheet-actions" style={{ marginBottom: 4 }}>
              {recoveryAvailable ? (
                <button className="btn-plain" onClick={() => setRecoveryOpen(true)}>
                  Recover Deleted Files…
                </button>
              ) : (
                <span style={{ fontSize: 12, color: "var(--text-2)" }}>
                  Install <code>testdisk</code> (<code>sudo apt install testdisk</code>) to enable
                  file recovery.
                </span>
              )}
            </div>
            {frozen.length > 0 && (
              <div className="info-rows" style={{ marginBottom: 10 }}>
                {frozen.map((f) => (
                  <div className="info-row" key={f.original_path}>
                    <span className="info-path">{f.original_path}</span>
                    <button
                      className="btn-plain small"
                      onClick={() => setUnfreezePath(f.original_path)}
                    >
                      Unfreeze…
                    </button>
                  </div>
                ))}
              </div>
            )}
          </>
        )}
      </div>
      {recoveryOpen && <RecoverySheet onClose={() => setRecoveryOpen(false)} />}
      {unfreezePath && (
        <UnfreezeSheet
          path={unfreezePath}
          onDone={() => {
            setUnfreezePath(null);
            refreshFrozenList();
          }}
          onClose={() => setUnfreezePath(null)}
        />
      )}
    </div>
  );
}
