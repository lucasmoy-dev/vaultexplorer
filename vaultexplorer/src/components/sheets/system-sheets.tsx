import { useEffect, useState } from "react";
import { api, formatSize, joinPath } from "../../api";
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
  async function withContactsPermission(action: () => Promise<void>) {
    setContactsBusy(true);
    setContactsMsg("");
    try {
      const granted = await api.androidContactsPermissionGranted();
      if (!granted) {
        await api.androidRequestContactsPermission();
        setContactsMsg("Grant the permission in the dialog, then try again.");
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
      const count = await api.androidExportContacts(exportDir);
      setContactsMsg(`Exported ${count} contact${count === 1 ? "" : "s"} to ${exportDir}`);
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
              <button className="settings-select" onClick={() => onExportConfig(includeCloudCreds)}>
                Copy config to clipboard
              </button>
              <button className="settings-select" onClick={onImportConfig}>
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
                  <button className="settings-select" disabled={contactsBusy} onClick={exportContacts}>
                    Export contacts
                  </button>
                </div>
                <label className="field-label">Import folder</label>
                <input value={importDir} onChange={(e) => setImportDir(e.target.value)} />
                <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
                  <button className="settings-select" disabled={contactsBusy} onClick={importContacts}>
                    Import contacts from folder
                  </button>
                </div>
                {contactsMsg && (
                  <p className="hint" style={{ marginTop: 6 }}>
                    {contactsMsg}
                  </p>
                )}
                <label className="field-label" style={{ marginTop: 14 }}>
                  Cloud & folder sync
                </label>
                <p className="hint" style={{ marginTop: -2 }}>
                  Not available on mobile yet -- desktop's sync (Drive/OneDrive/Dropbox, Syncthing,
                  Git, plain folder-to-folder) all shell out to a binary Android can't run.
                  Bringing it here needs its own design, not a straight port -- likely linking a
                  folder your phone's Drive/OneDrive app already syncs, rather than this app
                  syncing directly.
                </p>
              </>
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
