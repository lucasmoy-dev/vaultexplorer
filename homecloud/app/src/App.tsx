import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  api,
  formatBytes,
  formatRate,
  peerSummary,
  shortfall,
  shortId,
  type CodePreview,
  type Destination,
  type Invitation,
  type Readiness,
  type SharedFolder,
} from "./api";
import { StatusDot, stateLabel } from "./StatusDot";
import { PairingCard } from "./PairingCard";
import { SettingsSheet } from "./SettingsSheet";
import { QrScanner } from "./QrScanner";
import {
  CloseIcon,
  FolderIcon,
  GearIcon,
  JoinIcon,
  PauseIcon,
  PencilIcon,
  PlayIcon,
  PlusIcon,
  RefreshIcon,
  QrIcon,
  ShareIcon,
  TrashIcon,
} from "./Icons";

/** Slow enough not to hammer the engine, fast enough that a sync looks live. */
const POLL_MS = 1500;

type Screen =
  | { name: "list" }
  | { name: "share"; label: string; code: string }
  | { name: "join" }
  | { name: "settings" }
  | { name: "folder"; folder: SharedFolder };

export default function App() {
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [folders, setFolders] = useState<SharedFolder[]>([]);
  const [invitations, setInvitations] = useState<Invitation[]>([]);
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const status = await api.readiness();
    setReadiness(status);
    if (!status.ready) return;
    try {
      const [f, i] = await Promise.all([api.listFolders(), api.listInvitations()]);
      setFolders(f);
      setInvitations(i);
      // Keep an open folder sheet in step with what the engine now reports.
      setScreen((current) =>
        current.name === "folder"
          ? (() => {
              const fresh = f.find((x) => x.id === current.folder.id);
              return fresh ? { name: "folder" as const, folder: fresh } : { name: "list" as const };
            })()
          : current,
      );
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  // Coming back from a dropped connection leaves sockets one side still
  // believes in — a desktop showing "connected" while the phone shows
  // "disconnected". Nothing recovers from that quickly on its own, so
  // regaining the network dials again.
  useEffect(() => {
    const redial = () => {
      void api.reconnectAll().then(refresh).catch(() => undefined);
    };
    window.addEventListener("online", redial);
    return () => window.removeEventListener("online", redial);
  }, [refresh]);

  async function shareNewFolder() {
    setError(null);
    const picked = await open({ directory: true, multiple: false, title: "Elige la carpeta a compartir" });
    if (typeof picked !== "string") return;
    const label = picked.split("/").filter(Boolean).pop() ?? "Carpeta";
    try {
      const code = await api.shareFolder(picked, label);
      setScreen({ name: "share", label, code });
      void refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  // A failed launch and a slow one look identical from here unless the reason
  // is shown; without this the window sits on "Arrancando…" forever.
  if (!readiness?.ready) {
    return (
      <main className="app centered">
        {readiness?.problem ? (
          <StartupProblem problem={readiness.problem} onRetry={refresh} />
        ) : (
          <div className="starting">
            <div className="spinner" />
            <p>Arrancando…</p>
          </div>
        )}
      </main>
    );
  }

  return (
    <main className="app">
      <header className="topbar">
        <h1>HomeCloud</h1>
        <button
          className="gear"
          onClick={() => setScreen({ name: "settings" })}
          title={`Ajustes · ${readiness.device?.name ?? ""}`}
          aria-label="Ajustes"
        >
          <GearIcon />
        </button>
      </header>

      {error && (
        <div className="banner banner-bad" onClick={() => setError(null)}>
          {error}
        </div>
      )}

      {folders
        .filter((folder) => doesNotFit(folder.pendingBytes, folder.freeBytes))
        .map((folder) => (
          <div key={`space-${folder.id}`} className="banner banner-bad">
            <p className="banner-text">
              A «<strong>{folder.label}</strong>» le faltan {formatBytes(folder.pendingBytes)} por
              bajar y en ese disco quedan {formatBytes(folder.freeBytes ?? 0)}. Libera{" "}
              {formatBytes(shortfall(folder.pendingBytes, folder.freeBytes ?? 0))} o guárdala en otro
              sitio.
            </p>
          </div>
        ))}

      {invitations.map((invitation) => (
        <InvitationBanner
          key={invitation.fromDeviceId + (invitation.folder?.id ?? "")}
          invitation={invitation}
          onDone={refresh}
          onError={setError}
        />
      ))}

      {folders.length === 0 ? (
        <div className="empty">
          <p className="empty-title">Todavía no compartes nada</p>
          <p className="empty-body">
            Comparte una carpeta de este ordenador, o únete a una que ya exista en otro dispositivo.
          </p>
        </div>
      ) : (
        <ul className="folders">
          {folders.map((folder) => (
            <li key={folder.id}>
              <button className="folder" onClick={() => setScreen({ name: "folder", folder })}>
                <StatusDot state={folder.state} />
                <span className="folder-text">
                  <span className="folder-label">{folder.label}</span>
                  <span className="folder-sub">
                    {formatBytes(folder.bytes)} · {peerSummary(folder.peers)}
                  </span>
                </span>
                <span className="folder-state">
                  {stateLabel(folder.state)}
                  {folder.bytesPerSecond > 0 && (
                    <span className="folder-rate">{formatRate(folder.bytesPerSecond)}</span>
                  )}
                </span>
              </button>
              {folder.conflicts > 0 && (
                <p className="conflict-note">
                  {folder.conflicts === 1
                    ? "1 fichero se editó en dos sitios a la vez. Se guardaron las dos versiones."
                    : `${folder.conflicts} ficheros se editaron en dos sitios a la vez. Se guardaron las dos versiones.`}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}

      <footer className="actions">
        <button className="btn btn-primary" onClick={shareNewFolder}>
          <ShareIcon />
          Compartir carpeta
        </button>
        <button className="btn" onClick={() => setScreen({ name: "join" })}>
          <JoinIcon />
          Unirme con un código
        </button>
      </footer>

      {screen.name === "share" && (
        <Sheet title={`Compartir «${screen.label}»`} onClose={() => setScreen({ name: "list" })}>
          <PairingCard code={screen.code} label={screen.label} />
        </Sheet>
      )}

      {screen.name === "join" && (
        <Sheet title="Unirme a una carpeta" onClose={() => setScreen({ name: "list" })}>
          <JoinForm
            onJoined={() => {
              setScreen({ name: "list" });
              void refresh();
            }}
          />
        </Sheet>
      )}

      {screen.name === "settings" && (
        <Sheet title="Ajustes" onClose={() => setScreen({ name: "list" })}>
          <SettingsSheet onSaved={refresh} />
        </Sheet>
      )}

      {screen.name === "folder" && (
        <Sheet title={screen.folder.label} onClose={() => setScreen({ name: "list" })}>
          <FolderSheet
            folder={screen.folder}
            onChanged={refresh}
            onClosed={() => setScreen({ name: "list" })}
            onError={setError}
          />
        </Sheet>
      )}
    </main>
  );
}

/**
 * Whether a folder is about to run out of room where it is going.
 *
 * `null` means the disk could not be asked, and an unknown size means an older
 * device wrote the code. Neither is a reason to warn: a warning that fires
 * without knowing is one people learn to ignore.
 */
function doesNotFit(needed: number | null, free: number | null): boolean {
  if (needed === null || free === null || needed <= 0) return false;
  return shortfall(needed, free) > 0;
}

/** The one line that turns "it will not fit" into something to do about it. */
function SpaceCheck({ needed, free }: { needed: number | null; free: number | null }) {
  if (needed === null || free === null) return null;
  const missing = shortfall(needed, free);
  return (
    <p className={missing > 0 ? "problem" : "destination-explain"}>
      Ocupa {formatBytes(needed)} · quedan {formatBytes(free)} libres
      {missing > 0 && ` · faltan ${formatBytes(missing)}`}
    </p>
  );
}

function StartupProblem({ problem, onRetry }: { problem: string; onRetry: () => void }) {
  const [retrying, setRetrying] = useState(false);

  async function retry() {
    setRetrying(true);
    try {
      await api.retryEngine();
    } finally {
      setRetrying(false);
      onRetry();
    }
  }

  // The commonest cause by far, and the one the raw message explains worst.
  const looksLikeASecondCopy =
    problem.includes("stopped while starting") || problem.includes("never answered");

  return (
    <div className="startup-problem">
      <p className="startup-title">HomeCloud no pudo arrancar</p>
      {looksLikeASecondCopy && (
        <p className="startup-hint">
          Lo más habitual es que ya haya otra copia de HomeCloud abierta. Ciérrala y vuelve a
          intentarlo.
        </p>
      )}
      <p className="startup-detail">{problem}</p>
      <button className="btn btn-primary" onClick={retry} disabled={retrying}>
        {retrying ? "Reintentando…" : "Reintentar"}
      </button>
    </div>
  );
}

function InvitationBanner({
  invitation,
  onDone,
  onError,
}: {
  invitation: Invitation;
  onDone: () => void;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState(false);

  async function accept() {
    setBusy(true);
    try {
      const path = invitation.folder ? await api.suggestedPath(invitation.folder.label) : null;
      await api.acceptInvitation(invitation, path);
      onDone();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function decline() {
    setBusy(true);
    try {
      await api.declineInvitation(invitation);
      onDone();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="banner banner-ask">
      <p className="banner-text">
        {invitation.folder ? (
          <>
            <strong>{invitation.fromDeviceName}</strong>{" "}
            <span className="muted mono">{shortId(invitation.fromDeviceId)}</span> quiere compartir «
            <strong>{invitation.folder.label}</strong>»
          </>
        ) : (
          <>
            <strong>{invitation.fromDeviceName}</strong>{" "}
            <span className="muted mono">{shortId(invitation.fromDeviceId)}</span> quiere conectarse
            con este dispositivo
          </>
        )}
      </p>
      <div className="banner-actions">
        <button className="btn btn-small" onClick={decline} disabled={busy}>
          Rechazar
        </button>
        <button className="btn btn-small btn-primary" onClick={accept} disabled={busy}>
          Aceptar
        </button>
      </div>
    </div>
  );
}

function JoinForm({ onJoined }: { onJoined: () => void }) {
  const [code, setCode] = useState("");
  const [preview, setPreview] = useState<CodePreview | null>(null);
  const [destination, setDestination] = useState<Destination | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [password, setPassword] = useState("");
  const [needsPassword, setNeedsPassword] = useState(false);

  // Reading the code as it is pasted means the user finds out it is wrong
  // immediately, rather than after committing to a destination folder.
  useEffect(() => {
    if (code.trim().length < 8) {
      setPreview(null);
      setDestination(null);
      setProblem(null);
      return;
    }
    let cancelled = false;
    api
      .previewCode(code, password)
      .then(async (p) => {
        if (cancelled) return;
        setPreview(p);
        setProblem(null);
        setNeedsPassword(false);
        setDestination(await api.resolveDestination(p.suggestedPath, p.folderLabel, "itself"));
      })
      .catch((e) => {
        if (cancelled) return;
        setPreview(null);
        setDestination(null);
        // A locked code is not a broken one: it needs one more thing typed.
        const locked = String(e).includes("contraseña");
        setNeedsPassword(locked);
        setProblem(locked && !password ? null : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [code, password]);

  async function choosePath() {
    if (!preview) return;
    const picked = await open({
      directory: true,
      multiple: false,
      title: `¿Dónde guardo «${preview.folderLabel}»?`,
    });
    if (typeof picked !== "string") return;
    // No pick is assumed: choosing a directory already called like the folder
    // means "sync this one", anything else means "put it in here", and the
    // sentence below says which one is about to happen.
    setDestination(await api.resolveDestination(picked, preview.folderLabel));
  }

  async function flipPick() {
    if (!preview || !destination) return;
    const chosen =
      destination.pick === "inside"
        ? destination.path
        : (destination.path.split("/").slice(0, -1).join("/") || "/");
    const wanted = destination.pick === "inside" ? "itself" : "inside";
    setDestination(await api.resolveDestination(chosen, preview.folderLabel, wanted));
  }

  async function join() {
    if (!destination) return;
    setBusy(true);
    try {
      await api.redeemCode(code, destination.path, password);
      onJoined();
    } catch (e) {
      setProblem(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (scanning) {
    return (
      <QrScanner
        onScanned={(scanned) => {
          setCode(scanned);
          setScanning(false);
        }}
        onClose={() => setScanning(false)}
      />
    );
  }

  return (
    <div className="join">
      <button className="btn" onClick={() => setScanning(true)} type="button">
        <QrIcon />
        Escanear el QR del otro dispositivo
      </button>
      <label className="field">
        <span>…o pega aquí el código</span>
        <textarea
          value={code}
          onChange={(e) => setCode(e.target.value)}
          placeholder="HC1…"
          rows={3}
          autoFocus
        />
      </label>

      {needsPassword && (
        <label className="field">
          <span>Esta carpeta tiene contraseña</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="La contraseña de la carpeta"
            autoFocus
          />
        </label>
      )}

      {problem && <p className="problem">{problem}</p>}

      {preview && destination && (
        <>
          <p className="preview">
            <strong>{preview.deviceName}</strong> comparte «<strong>{preview.folderLabel}</strong>»
          </p>

          <div className="destination">
            <p className="destination-path">
              <FolderIcon />
              <span>{destination.path}</span>
            </p>
            <p className="destination-explain">{destination.explanation}</p>
            <SpaceCheck needed={preview.bytes} free={destination.freeBytes} />
            <div className="destination-actions">
              <button className="btn btn-small" onClick={choosePath} type="button">
                <PencilIcon />
                Cambiar carpeta
              </button>
              <button className="btn btn-small btn-quiet" onClick={flipPick} type="button">
                {destination.pick === "inside"
                  ? "Usar esa carpeta tal cual"
                  : "Crear una subcarpeta dentro"}
              </button>
            </div>
          </div>

          <button className="btn btn-primary" onClick={join} disabled={busy}>
            <JoinIcon />
            {busy
              ? "Conectando…"
              : doesNotFit(preview.bytes, destination.freeBytes)
                ? "Unirme de todos modos"
                : "Unirme"}
          </button>
        </>
      )}
    </div>
  );
}

function FolderSheet({
  folder,
  onChanged,
  onClosed,
  onError,
}: {
  folder: SharedFolder;
  onChanged: () => void;
  onClosed: () => void;
  onError: (message: string) => void;
}) {
  const [code, setCode] = useState<string | null>(null);
  const [confirmingStop, setConfirmingStop] = useState(false);
  // Read-only and the device list are settings, not things you reach for: they
  // are looked at once when a folder is set up and never again. Out of the way
  // by default so the actions you actually use are the ones on screen.
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const paused = folder.state.kind === "paused";

  async function showCode() {
    try {
      setCode(await api.codeFor(folder.id));
    } catch (e) {
      onError(String(e));
    }
  }

  /** Every action here changes something in the engine and then re-reads it. */
  async function act(change: () => Promise<void>) {
    try {
      await change();
      onChanged();
    } catch (e) {
      onError(String(e));
    }
  }

  if (code) return <PairingCard code={code} label={folder.label} />;

  return (
    <div className="sheet-body">
      <p className="sheet-line">
        <StatusDot state={folder.state} /> {stateLabel(folder.state)}
        {folder.bytesPerSecond > 0 && (
          <span className="muted"> · {formatRate(folder.bytesPerSecond)}</span>
        )}
      </p>
      <p className="sheet-line muted">
        {folder.files} ficheros · {formatBytes(folder.bytes)}
        {folder.freeBytes !== null && ` · ${formatBytes(folder.freeBytes)} libres en el disco`}
      </p>

      <div className="sheet-actions">
        <button
          className="btn"
          onClick={() => void revealItemInDir(folder.path)}
          title={folder.path}
        >
          <FolderIcon />
          Abrir la carpeta
        </button>
        <button className="btn" onClick={showCode}>
          <PlusIcon />
          Añadir otro dispositivo
        </button>
        <button
          className="btn"
          onClick={() => act(() => api.rescan(folder.id))}
          title="Vuelve a mirar la carpeta desde cero. Arregla los avisos de ficheros que ya no están."
        >
          <RefreshIcon />
          Volver a revisar
        </button>
        <button
          className="btn"
          onClick={() => act(() => api.setFolderPaused(folder.id, !paused))}
        >
          {paused ? <PlayIcon /> : <PauseIcon />}
          {paused ? "Reanudar" : "Pausar"}
        </button>
      </div>

      <button
        className="btn btn-quiet disclosure"
        onClick={() => setAdvancedOpen((open) => !open)}
        aria-expanded={advancedOpen}
      >
        <ChevronIcon open={advancedOpen} />
        Avanzado
      </button>

      {advancedOpen && (
        <div className="advanced">
          <label className="toggle">
            <input
              type="checkbox"
              checked={folder.readOnly}
              onChange={(e) => act(() => api.setFolderReadOnly(folder.id, e.target.checked))}
            />
            <span>
              Solo lectura
              <em>Recibe los cambios de los demás, pero nunca envía los suyos.</em>
            </span>
          </label>

          <label className="toggle">
            <input
              type="checkbox"
              checked={folder.wifiOnly}
              onChange={(e) => act(() => api.setFolderWifiOnly(folder.id, e.target.checked))}
            />
            <span>
              Solo con wifi
              <em>
                Se detiene cuando la conexión se paga por datos.
                {folder.pausedByNetwork && " Ahora mismo está detenida por eso."}
              </em>
            </span>
          </label>

          <FolderPassword folder={folder} onChanged={onChanged} onError={onError} />

          <p className="section">Dispositivos</p>
          {folder.peers.length === 0 ? (
            <p className="hint">Todavía no comparte con ningún dispositivo.</p>
          ) : (
            <ul className="peers">
              {folder.peers.map((peer) => (
                <li key={peer.id}>
                  <span className={`dot ${peer.connected ? "dot-ok" : "dot-idle"}`} aria-hidden />
                  {peer.name}
                  <span className="muted mono">{shortId(peer.id)}</span>
                  <span className="muted">{peer.connected ? "conectado" : "sin conexión"}</span>
                </li>
              ))}
            </ul>
          )}

          <p className="path-line mono">{folder.path}</p>

          {confirmingStop ? (
            <>
              <button
                className="btn btn-danger"
                onClick={async () => {
                  try {
                    await api.stopSharing(folder.id);
                    onClosed();
                    onChanged();
                  } catch (e) {
                    onError(String(e));
                  }
                }}
              >
                <TrashIcon />
                Sí, dejar de sincronizar
              </button>
              <p className="muted small">
                Los ficheros que ya están en este ordenador se quedan donde están. Solo se deja de
                sincronizar.
              </p>
            </>
          ) : (
            <button className="btn btn-quiet" onClick={() => setConfirmingStop(true)}>
              <TrashIcon />
              Dejar de sincronizar
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * The password on a folder.
 *
 * It is not a password the engine checks — there is nowhere in the protocol for
 * one. It is what this folder's pairing codes are encrypted with, so a code
 * that leaks is a code nobody can use. That is why the wording talks about the
 * code and not about "protecting the folder": the files are as reachable as
 * they ever were to a device that is already sharing them.
 */
function FolderPassword({
  folder,
  onChanged,
  onError,
}: {
  folder: SharedFolder;
  onChanged: () => void;
  onError: (message: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");

  async function save(password: string) {
    try {
      await api.setFolderPassword(folder.id, password);
      setEditing(false);
      setValue("");
      onChanged();
    } catch (e) {
      onError(String(e));
    }
  }

  return (
    <div className="folder-password">
      <p className="section">Contraseña</p>
      <p className="hint">
        {folder.hasPassword
          ? "Los códigos de esta carpeta van cifrados: sin la contraseña no sirven de nada."
          : "Sin contraseña. Cualquiera con un código de esta carpeta puede entrar."}
      </p>

      {editing ? (
        <>
          <input
            type="password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder="Una contraseña para esta carpeta"
            autoFocus
          />
          <div className="destination-actions">
            <button
              className="btn btn-small btn-primary"
              onClick={() => void save(value)}
              disabled={value.trim().length === 0}
            >
              Guardar
            </button>
            <button
              className="btn btn-small btn-quiet"
              onClick={() => {
                setEditing(false);
                setValue("");
              }}
            >
              Cancelar
            </button>
          </div>
        </>
      ) : (
        <div className="destination-actions">
          <button className="btn btn-small" onClick={() => setEditing(true)}>
            {folder.hasPassword ? "Cambiar la contraseña" : "Poner una contraseña"}
          </button>
          {folder.hasPassword && (
            <button className="btn btn-small btn-quiet" onClick={() => void save("")}>
              Quitarla
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** Points down when the section is open, right when it is closed. */
function ChevronIcon({ open }: { open: boolean }) {
  return (
    <svg
      className="icon"
      viewBox="0 0 24 24"
      width="17"
      height="17"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ transform: open ? "rotate(90deg)" : "none", transition: "transform 120ms" }}
    >
      <path d="M9 5.5l7 6.5-7 6.5" />
    </svg>
  );
}

function Sheet({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="scrim" onClick={onClose}>
      <section className="sheet" onClick={(e) => e.stopPropagation()}>
        <header className="sheet-head">
          <h2>{title}</h2>
          <button className="btn btn-quiet btn-icon" onClick={onClose} aria-label="Cerrar">
            <CloseIcon />
          </button>
        </header>
        {children}
      </section>
    </div>
  );
}
