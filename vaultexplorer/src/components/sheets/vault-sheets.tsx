import { useEffect, useState } from "react";
import { api, baseName, ENCRYPTED_FILE_EXT } from "../../api";
import { LockGlyph, EyeGlyph, EyeOffGlyph } from "../../icons";
import {
  PendingAction,
  VaultCreateOptions,
  SensitiveTimeout,
  SENSITIVE_TIMEOUT_CHOICES,
  nearestSensitiveChoice,
} from "../../types";
import { useShakeOnError } from "../../hooks/useShakeOnError";

// ---------- small modal sheets ----------

// A password `<input>` with a Finder/Chrome-style eye toggle to reveal the
// typed value -- every password/passphrase field in the app uses this
// instead of a bare `type="password"` input.
export function PasswordInput({
  className,
  ...rest
}: Omit<React.InputHTMLAttributes<HTMLInputElement>, "type">) {
  const [show, setShow] = useState(false);
  return (
    <div className="password-field">
      <input {...rest} type={show ? "text" : "password"} className={className} />
      <button
        type="button"
        className="password-toggle"
        tabIndex={-1}
        aria-label={show ? "Hide password" : "Show password"}
        onClick={() => setShow((s) => !s)}
      >
        {show ? <EyeOffGlyph size={15} /> : <EyeGlyph size={15} />}
      </button>
    </div>
  );
}

const SHRED_PASSES = 3;

export function ActionSheet({
  action,
  onCancel,
  onSubmit,
}: {
  action: Exclude<PendingAction, { kind: "unlock" } | { kind: "newVault" }>;
  onCancel: () => void;
  onSubmit: (value: string) => void;
}) {
  const isDelete = action.kind === "delete";
  const isSecureDelete = action.kind === "secureDelete";
  const [value, setValue] = useState("");

  const titles: Record<string, string> = {
    delete: "Delete",
    secureDelete: "Secure Delete",
    gitCommit: "Commit All Changes",
    freeze: "Freeze Folder",
  };
  const confirmLabels: Record<string, string> = {
    delete: "Delete",
    secureDelete: "Secure Delete",
    gitCommit: "Commit",
    freeze: "Freeze",
  };
  const deleteLabel =
    action.kind === "delete" || action.kind === "secureDelete"
      ? action.names.length === 1
        ? action.names[0]
        : `${action.names.length} items`
      : "";

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>{titles[action.kind]}</h3>
        {isSecureDelete ? (
          <p>
            Overwrite <strong>{deleteLabel}</strong> with random data {SHRED_PASSES} times before
            deleting. This cannot be undone, and does not guarantee unrecoverability on SSDs or
            other wear-leveled flash storage — only on classic spinning disks.
          </p>
        ) : isDelete ? (
          <p>
            Are you sure you want to delete <strong>{deleteLabel}</strong>? This action cannot be
            undone.
          </p>
        ) : action.kind === "freeze" ? (
          <PasswordInput
            autoFocus
            placeholder="Choose a freeze password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") onSubmit(value);
              if (e.key === "Escape") onCancel();
            }}
          />
        ) : (
          <input
            autoFocus
            placeholder="Commit message"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") onSubmit(value);
              if (e.key === "Escape") onCancel();
            }}
          />
        )}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button
            className={isDelete || isSecureDelete ? "btn-primary danger" : "btn-primary"}
            onClick={() => onSubmit(value)}
          >
            {confirmLabels[action.kind]}
          </button>
        </div>
      </div>
    </div>
  );
}

export function UnlockSheet({
  name,
  error,
  onCancel,
  onSubmit,
}: {
  name: string;
  error: string;
  onCancel: () => void;
  onSubmit: (password: string, keepUnlocked: boolean) => void;
}) {
  const [pw, setPw] = useState("");
  const [keep, setKeep] = useState(false);
  const cardRef = useShakeOnError<HTMLDivElement>(error);
  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div ref={cardRef} className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="sheet-lock">
          <LockGlyph size={22} />
        </div>
        <h3>Unlock “{name}”</h3>
        <p>This folder is an encrypted vault. Enter its password to open it.</p>
        <PasswordInput
          autoFocus
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onSubmit(pw, keep);
            if (e.key === "Escape") onCancel();
          }}
        />
        <label className="checkbox-row">
          <input type="checkbox" checked={keep} onChange={(e) => setKeep(e.target.checked)} />
          Keep unlocked when I navigate away
        </label>
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" onClick={() => onSubmit(pw, keep)}>
            Unlock
          </button>
        </div>
      </div>
    </div>
  );
}

/// A file (or a file under a folder) marked "sensitive" re-locks after the
/// configured window even while its vault stays unlocked. Shown before
/// opening/previewing such a file to re-verify the vault password and open
/// the time-boxed sensitive session.
export function SensitiveUnlockSheet({
  name,
  error,
  defaultTimeout,
  onCancel,
  onSubmit,
}: {
  name: string;
  error: string;
  /// What Settings has configured -- the selector starts here, so the common
  /// case is "type the password, press Enter" with no extra decision.
  defaultTimeout: SensitiveTimeout;
  onCancel: () => void;
  onSubmit: (password: string, timeout: SensitiveTimeout) => void;
}) {
  const [pw, setPw] = useState("");
  const [timeout_, setTimeout_] = useState<SensitiveTimeout>(() =>
    nearestSensitiveChoice(defaultTimeout)
  );
  const cardRef = useShakeOnError<HTMLDivElement>(error);
  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div ref={cardRef} className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="sheet-lock">
          <LockGlyph size={22} />
        </div>
        <h3>Sensitive file</h3>
        <p>
          “{name}” is marked sensitive. Re-enter this vault's password to view it, and choose how
          long the session stays open.
        </p>
        <PasswordInput
          autoFocus
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onSubmit(pw, timeout_);
            if (e.key === "Escape") onCancel();
          }}
        />
        {/* Per-unlock duration, not just the configured default: how long
            this should stay readable depends on what you're about to do with
            it, and going to Settings mid-task to change that isn't it. */}
        <div className="duration-picker" role="radiogroup" aria-label="Keep open for">
          {SENSITIVE_TIMEOUT_CHOICES.map((o) => (
            <button
              key={String(o.value)}
              type="button"
              role="radio"
              aria-checked={timeout_ === o.value}
              className={`duration-chip ${timeout_ === o.value ? "on" : ""}`}
              onClick={() => setTimeout_(o.value)}
            >
              {o.label}
            </button>
          ))}
        </div>
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" onClick={() => onSubmit(pw, timeout_)}>
            Unlock
          </button>
        </div>
      </div>
    </div>
  );
}

export function ReauthOverlay({
  name,
  error,
  onSubmit,
  onLockInstead,
}: {
  name: string;
  error: string;
  onSubmit: (password: string) => void;
  onLockInstead: () => void;
}) {
  const [pw, setPw] = useState("");
  const cardRef = useShakeOnError<HTMLDivElement>(error);
  return (
    <div className="reauth-overlay">
      <div ref={cardRef} className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="sheet-lock">
          <LockGlyph size={22} />
        </div>
        <h3>Re-authenticate</h3>
        <p>“{name}” is a sensitive vault. Confirm your password to keep viewing it.</p>
        <PasswordInput
          autoFocus
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && onSubmit(pw)}
        />
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onLockInstead}>
            Lock Instead
          </button>
          <button className="btn-primary" onClick={() => onSubmit(pw)}>
            Unlock
          </button>
        </div>
      </div>
    </div>
  );
}

export function ZipPasswordSheet({
  name,
  error,
  onCancel,
  onSubmit,
}: {
  name: string;
  error: string;
  onCancel: () => void;
  onSubmit: (password: string) => void;
}) {
  const [pw, setPw] = useState("");
  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>“{name}” requires a password</h3>
        <p>Enter the password to continue.</p>
        <PasswordInput
          autoFocus
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onSubmit(pw);
            if (e.key === "Escape") onCancel();
          }}
        />
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" onClick={() => onSubmit(pw)}>
            Extract
          </button>
        </div>
      </div>
    </div>
  );
}

export function EncryptFileSheet({
  name,
  isFolder,
  onCancel,
  onSubmit,
}: {
  name: string;
  isFolder?: boolean;
  onCancel: () => void;
  onSubmit: (password: string) => void;
}) {
  const [pw, setPw] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");

  function go() {
    if (pw === "") return;
    if (pw !== confirm) {
      setError("Passwords don't match");
      return;
    }
    onSubmit(pw);
  }

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="sheet-lock">
          <LockGlyph size={22} />
        </div>
        <h3>Encrypt “{name}”</h3>
        <p>
          {isFolder
            ? `This turns "${name}" into an encrypted vault, locked with this password.`
            : `This will create "${name}${ENCRYPTED_FILE_EXT}" encrypted with this password, and delete the plaintext original.`}
        </p>
        <PasswordInput
          autoFocus
          placeholder="Password"
          value={pw}
          onChange={(e) => setPw(e.target.value)}
        />
        <PasswordInput
          placeholder="Confirm password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && go()}
        />
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" onClick={go}>
            Encrypt
          </button>
        </div>
      </div>
    </div>
  );
}

export function NewVaultSheet({
  error,
  onCancel,
  onSubmit,
}: {
  error: string;
  onCancel: () => void;
  onSubmit: (name: string, password: string, opts: VaultCreateOptions) => void;
}) {
  const [name, setName] = useState("");
  const [pw, setPw] = useState("");
  const [confirm, setConfirm] = useState("");
  const [local, setLocal] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [sensitive, setSensitive] = useState(false);
  const [autoLockMinutes, setAutoLockMinutes] = useState(15);
  const [autoUnlock, setAutoUnlock] = useState(false);

  function go() {
    if (name.trim() === "" || pw === "") return;
    if (pw !== confirm) {
      setLocal("Passwords don't match");
      return;
    }
    onSubmit(name.trim(), pw, { sensitive, autoLockMinutes: autoLockMinutes || 15, autoUnlock });
  }
  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="sheet-lock">
          <LockGlyph size={22} />
        </div>
        <h3>New vault</h3>
        <p>An encrypted vault folder will be created here.</p>
        <input autoFocus placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} />
        <PasswordInput
          placeholder="Password"
          value={pw}
          onChange={(e) => setPw(e.target.value)}
        />
        <PasswordInput
          placeholder="Confirm password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && !showAdvanced && go()}
        />
        <button
          type="button"
          className="btn-plain small advanced-toggle"
          onClick={() => setShowAdvanced((s) => !s)}
        >
          {showAdvanced ? "Hide" : "Show"} Advanced Options
        </button>
        {showAdvanced && (
          <div className="advanced-options">
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={sensitive}
                onChange={(e) => setSensitive(e.target.checked)}
              />
              Sensitive (auto-lock, re-authenticate on refocus)
            </label>
            {sensitive && (
              <label className="field-row">
                <span className="field-label">Auto-lock after (minutes)</span>
                <input
                  type="number"
                  min={1}
                  value={autoLockMinutes}
                  onChange={(e) => setAutoLockMinutes(parseInt(e.target.value, 10) || 15)}
                />
              </label>
            )}
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={autoUnlock}
                onChange={(e) => setAutoUnlock(e.target.checked)}
              />
              Unlock automatically when the app starts
            </label>
            {autoUnlock && (
              <p className="advanced-hint">Stores this password in your OS keyring.</p>
            )}
          </div>
        )}
        {(local || error) && <p className="error">{local || error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" onClick={go}>
            Create vault
          </button>
        </div>
      </div>
    </div>
  );
}

/// Editing the same "Sensitive"/"Auto-lock"/"Unlock automatically" options
/// `NewVaultSheet`'s Advanced section sets at creation time, but for a vault
/// that already exists -- opened via "Vault Settings…" on any vault folder.
/// Turning "Unlock automatically" on for the first time needs the vault's
/// password (this sheet never has it otherwise -- it's never stored in
/// plaintext anywhere), which `onSave` verifies before it's written to the
/// OS keyring rather than trusting whatever was typed.
export function VaultSettingsSheet({
  name,
  initial,
  canAutoUnlock,
  onSave,
  onCancel,
}: {
  name: string;
  initial: VaultCreateOptions;
  canAutoUnlock: boolean;
  onSave: (opts: VaultCreateOptions, password: string | null) => Promise<void>;
  onCancel: () => void;
}) {
  const [sensitive, setSensitive] = useState(initial.sensitive);
  const [autoLockMinutes, setAutoLockMinutes] = useState(initial.autoLockMinutes || 15);
  const [autoUnlock, setAutoUnlock] = useState(initial.autoUnlock);
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cardRef = useShakeOnError<HTMLDivElement>(error);

  // Only need the password when *enabling* auto-unlock -- turning it off,
  // or leaving an already-enabled vault's toggle checked, changes nothing
  // that needs re-proving.
  const needsPassword = autoUnlock && !initial.autoUnlock;

  async function save() {
    if (needsPassword && pw === "") {
      setError("Enter the vault password to enable auto-unlock.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      await onSave({ sensitive, autoLockMinutes: autoLockMinutes || 15, autoUnlock }, needsPassword ? pw : null);
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  return (
    <div className="sheet-overlay" onMouseDown={onCancel}>
      <div ref={cardRef} className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <div className="sheet-lock">
          <LockGlyph size={22} />
        </div>
        <h3>Vault Settings — “{name}”</h3>
        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={sensitive}
            onChange={(e) => setSensitive(e.target.checked)}
          />
          Sensitive (auto-lock, re-authenticate on refocus)
        </label>
        {sensitive && (
          <label className="field-row">
            <span className="field-label">Auto-lock after (minutes)</span>
            <input
              type="number"
              min={1}
              value={autoLockMinutes}
              onChange={(e) => setAutoLockMinutes(parseInt(e.target.value, 10) || 15)}
            />
          </label>
        )}
        {canAutoUnlock ? (
          <>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={autoUnlock}
                onChange={(e) => setAutoUnlock(e.target.checked)}
              />
              Unlock automatically when the app starts
            </label>
            {needsPassword && (
              <>
                <p className="advanced-hint">
                  Enter the vault password once to store it in your OS keyring.
                </p>
                <PasswordInput
                  autoFocus
                  placeholder="Vault password"
                  value={pw}
                  onChange={(e) => setPw(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && save()}
                />
              </>
            )}
          </>
        ) : (
          <p className="hint">
            Nested vaults can't unlock automatically at startup — their path only exists while the
            parent vault is open.
          </p>
        )}
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn-primary" disabled={busy} onClick={save}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function RecoverySheet({ onClose }: { onClose: () => void }) {
  const [disks, setDisks] = useState<import("../../api").DiskInfo[]>([]);
  const [device, setDevice] = useState("");
  const [destDir, setDestDir] = useState("");
  const [sameDisk, setSameDisk] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);

  useEffect(() => {
    api.recoveryListDisks().then(setDisks).catch((e) => setError(String(e)));
  }, []);
  useEffect(() => {
    if (!device || !destDir) {
      setSameDisk(false);
      return;
    }
    api.recoverySameDisk(device, destDir).then(setSameDisk).catch(() => setSameDisk(false));
  }, [device, destDir]);

  async function run() {
    setRunning(true);
    setError("");
    setDone(false);
    try {
      await api.recoveryRun(device, destDir);
      setDone(true);
    } catch (e) {
      setError(String(e));
    }
    setRunning(false);
  }

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Recover Deleted Files</h3>
        <p>
          Wraps <code>photorec</code> to scan a whole disk/partition for recoverable files — this
          is separate from the app's own Trash, and cannot recover anything that was Secure
          Deleted (that's the point of Secure Delete).
        </p>
        <label className="checkbox-row" style={{ display: "block", marginBottom: 10 }}>
          Disk or partition to scan
          <select value={device} onChange={(e) => setDevice(e.target.value)} style={{ width: "100%", marginTop: 4 }}>
            <option value="">Select…</option>
            {disks.map((d) => (
              <option key={d.name} value={`/dev/${d.name}`}>
                /dev/{d.name} ({d.size}, {d.type}
                {d.mountpoint ? `, ${d.mountpoint}` : ""})
              </option>
            ))}
          </select>
        </label>
        <input
          placeholder="Destination folder (on a different disk)"
          value={destDir}
          onChange={(e) => setDestDir(e.target.value)}
        />
        {sameDisk && (
          <p className="error">
            Destination looks like it's on the same disk you're scanning — recovering onto it
            risks overwriting the very data you're trying to recover. Pick a folder on a
            different disk.
          </p>
        )}
        {error && <p className="error">{error}</p>}
        {done && <p>Done — check the destination folder for recovered files.</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose}>
            Close
          </button>
          <button
            className="btn-primary"
            disabled={!device || !destDir || sameDisk || running}
            onClick={run}
          >
            {running ? "Running… (this can take a long time)" : "Start Recovery"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function UnfreezeSheet({
  path,
  onDone,
  onClose,
}: {
  path: string;
  onDone: () => void;
  onClose: () => void;
}) {
  const [pw, setPw] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function go(keepChanges: boolean) {
    setBusy(true);
    setError("");
    try {
      await api.unfreezeFolder(path, pw, keepChanges);
      onDone();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }

  return (
    <div className="sheet-overlay" onMouseDown={onClose}>
      <div className="sheet-card" onMouseDown={(e) => e.stopPropagation()}>
        <h3>Unfreeze “{baseName(path)}”</h3>
        <PasswordInput
          autoFocus
          placeholder="Freeze password"
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && go(true)}
        />
        {error && <p className="error">{error}</p>}
        <div className="sheet-actions">
          <button className="btn-plain" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn-plain danger" disabled={busy || !pw} onClick={() => go(false)}>
            Discard Changes
          </button>
          <button className="btn-primary" disabled={busy || !pw} onClick={() => go(true)}>
            Keep Changes
          </button>
        </div>
      </div>
    </div>
  );
}
