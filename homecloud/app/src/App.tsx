import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  api,
  formatBytes,
  peerSummary,
  remaining,
  timeAgo,
  shortfall,
  shortId,
  type CodePreview,
  type DeletedFile,
  type FolderMode,
  type Destination,
  type Invitation,
  type LinkStatus,
  type Peer,
  type Readiness,
  type SharedFolder,
} from "./api";
import { copyText } from "./clipboard";
import { StatusDot, stateLabel } from "./StatusDot";
import { PairingCard } from "./PairingCard";
import { SettingsSheet } from "./SettingsSheet";
import { QrScanner } from "./QrScanner";
import { showToast, ToastHost } from "./Toast";
import { Help } from "./Help";
import { setLanguage, t, tf } from "./i18n";
import {
  CheckIcon,
  CloseIcon,
  CopyIcon,
  FolderIcon,
  GearIcon,
  JoinIcon,
  PauseIcon,
  PencilIcon,
  PlayIcon,
  LinkIcon,
  PlusIcon,
  RefreshIcon,
  QrIcon,
  ShareIcon,
  TrashIcon,
} from "./Icons";

/** Slow enough not to hammer the engine, fast enough that a sync looks live. */
const POLL_MS = 1500;
/** While the engine is still starting, ready is the only thing anyone is
 * waiting on — polled faster so "Arrancando…" does not linger after the
 * engine has actually answered. */
const STARTUP_POLL_MS = 300;

type Screen =
  | { name: "list" }
  | { name: "share"; label: string; code: string }
  | { name: "join" }
  | { name: "settings" }
  | { name: "folder"; folder: SharedFolder };

/** Which way a connected peer is reached, in words. The engine always
 * prefers the local network; this is how to see that it did. */
function routeWords(route: Peer["route"]): string {
  switch (route) {
    case "lan":
      return " · en tu red";
    case "internet":
      return " · por internet";
    case "relay":
      return " · por repetidor (lento)";
    default:
      return "";
  }
}

export default function App() {
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [folders, setFolders] = useState<SharedFolder[]>([]);
  const [invitations, setInvitations] = useState<Invitation[]>([]);
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [error, setError] = useState<string | null>(null);
  // The poll's own failure, kept apart from errors of things the user did:
  // it clears itself on the next poll that works. One transient failure used
  // to leave its banner up until someone clicked it away, long after the
  // engine was answering again.
  const [pollError, setPollError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const status = await api.readiness();
    setReadiness(status);
    if (!status.ready) return status;
    try {
      const [f, i] = await Promise.all([api.listFolders(), api.listInvitations()]);
      setFolders(f);
      setInvitations(i);
      setPollError(null);
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
      setPollError(String(e));
    }
    return status;
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      const status = await refresh();
      if (cancelled) return;
      timer = setTimeout(() => void tick(), status?.ready ? POLL_MS : STARTUP_POLL_MS);
    };
    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [refresh]);

  // Applied once, as soon as the engine is up, so the very first screen a
  // returning user sees is already in their language — not just the screens
  // opened after they happen to visit Settings. The dependency is the plain
  // boolean, not the `readiness` object, so this does not re-run on every poll.
  useEffect(() => {
    if (!readiness?.ready) return;
    void api.settings().then((s) => setLanguage(s.language)).catch(() => undefined);
  }, [readiness?.ready]);

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
      <ToastHost />
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

      {(error ?? pollError) && (
        <div className="banner banner-bad" onClick={() => setError(null)}>
          {error ?? pollError}
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
                  {/* "Sincronizando 9%" says nothing about whether to wait for
                      it. How long is left, and how fast, is what does. */}
                  {remaining(folder) && <span className="folder-rate">{remaining(folder)}</span>}
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
  // Guards the stop-sharing request: without it, a click that looks like it
  // did nothing (waiting on the delete plus a full folder-list refresh) gets
  // clicked again, and the second one lands on a folder the engine already
  // removed.
  const [stopBusy, setStopBusy] = useState(false);
  // Read-only and the device list are settings, not things you reach for: they
  // are looked at once when a folder is set up and never again. Out of the way
  // by default so the actions you actually use are the ones on screen.
  const [advancedOpen, setAdvancedOpen] = useState(false);
  // What the user just asked for, until the next poll confirms it. A checkbox
  // that springs back for a second and a half reads as one that did not work,
  // and gets clicked again.
  const [wanted, setWanted] = useState<{ mode?: FolderMode; wifiOnly?: boolean }>({});
  const paused = folder.state.kind === "paused";
  const mode = wanted.mode ?? folder.mode;
  const wifiOnly = wanted.wifiOnly ?? folder.wifiOnly;

  // Once the engine agrees, the guess has nothing left to say.
  useEffect(() => {
    setWanted((current) => {
      const settled = { ...current };
      if (settled.mode === folder.mode || settled.mode === undefined) delete settled.mode;
      if (settled.wifiOnly === folder.wifiOnly || settled.wifiOnly === undefined) {
        delete settled.wifiOnly;
      }
      return settled;
    });
  }, [folder.mode, folder.wifiOnly]);

  async function showCode() {
    try {
      setCode(await api.codeFor(folder.id));
    } catch (e) {
      onError(String(e));
    }
  }

  /** Every action here changes something in the engine and then re-reads it. */
  async function act(change: () => Promise<void>, onFailed?: () => void) {
    try {
      await change();
      onChanged();
    } catch (e) {
      onFailed?.();
      onError(String(e));
    }
  }

  if (code) return <PairingCard code={code} label={folder.label} />;

  return (
    <div className="sheet-body">
      <p className="sheet-line">
        <StatusDot state={folder.state} /> {stateLabel(folder.state)}
      </p>
      {remaining(folder) && <p className="sheet-line muted">{remaining(folder)}</p>}
      <p className="sheet-line muted">
        {folder.files} ficheros · {formatBytes(folder.bytes)}
        {folder.freeBytes !== null && ` · ${formatBytes(folder.freeBytes)} libres en el disco`}
      </p>
      {/* A folder stopped halfway still has a real size and a real amount
          left; the engine simply refuses to say so while it is paused. */}
      {paused && folder.pendingBytes > 0 && (
        <p className="sheet-line muted">
          Le faltan {formatBytes(folder.pendingBytes)} por bajar.
        </p>
      )}

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

      {/* Deliberately not behind "Avanzado": stopping a sync is the one
          action in this sheet a user comes looking for on purpose, and
          burying it behind a disclosure most people never open is why it
          went unnoticed. */}
      {confirmingStop ? (
        <div className="stop-confirm">
          <button
            className="btn btn-danger"
            disabled={stopBusy}
            onClick={async () => {
              setStopBusy(true);
              try {
                await api.stopSharing(folder.id);
                onClosed();
                onChanged();
              } catch (e) {
                onError(String(e));
              } finally {
                setStopBusy(false);
              }
            }}
          >
            <TrashIcon />
            {stopBusy ? "Dejando de sincronizar…" : "Sí, dejar de sincronizar"}
          </button>
          <p className="muted small">
            Los ficheros que ya están en este ordenador se quedan donde están. Solo se deja de
            sincronizar.
          </p>
        </div>
      ) : (
        <button className="btn btn-quiet" onClick={() => setConfirmingStop(true)}>
          <TrashIcon />
          Dejar de sincronizar
        </button>
      )}

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
          <p className="section">Qué hace esta copia</p>
          {MODES.map((option) => (
            <label className="toggle" key={option.mode}>
              <input
                type="radio"
                name={`mode-${folder.id}`}
                checked={mode === option.mode}
                onChange={() => {
                  setWanted((current) => ({ ...current, mode: option.mode }));
                  void act(
                    () => api.setFolderMode(folder.id, option.mode),
                    // A guess that turned out wrong must not outlive the attempt.
                    () => setWanted((current) => ({ ...current, mode: undefined })),
                  );
                }}
              />
              <span>
                {option.title}
                <em>{option.explanation}</em>
              </span>
            </label>
          ))}
          {mode === "archive" && folder.extraBytes > 0 && (
            <p className="hint">
              Ahora mismo guarda {formatBytes(folder.extraBytes)} que ya no están en los otros
              dispositivos.
            </p>
          )}

          <label className="toggle">
            <input
              type="checkbox"
              checked={wifiOnly}
              onChange={(e) => {
                const value = e.target.checked;
                setWanted((current) => ({ ...current, wifiOnly: value }));
                void act(
                  () => api.setFolderWifiOnly(folder.id, value),
                  () => setWanted((current) => ({ ...current, wifiOnly: undefined })),
                );
              }}
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

          <ShareLink folder={folder} onError={onError} />

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
                  {/* Whether the other end has finished. Without it this
                      device reads "Al día" while the phone it gave the folder
                      to is still at four per cent. */}
                  {peer.completion !== null && (
                    <span className="muted">
                      {peer.completion >= 100 ? "al día" : `${peer.completion}%`}
                    </span>
                  )}
                  <span className="muted">
                    {peer.connected ? `conectado${routeWords(peer.route)}` : "sin conexión"}
                  </span>
                </li>
              ))}
            </ul>
          )}

          <DeletedFiles folder={folder} onError={onError} />

          <p className="path-line mono">{folder.path}</p>
        </div>
      )}
    </div>
  );
}

/**
 * The three things a copy of a folder can be.
 *
 * The last one is what turns a computer into somewhere a full phone can
 * delete against: it takes everything, sends nothing back, and never carries
 * out a deletion, so the videos removed from the phone stay here.
 */
const MODES: { mode: FolderMode; title: string; explanation: string }[] = [
  {
    mode: "twoWay",
    title: "La misma carpeta en los dos sitios",
    explanation: "Lo que cambies o borres aquí pasa a los demás, y al revés.",
  },
  {
    mode: "receiveOnly",
    title: "Solo recibe",
    explanation: "Recibe los cambios de los demás, pero nunca envía los suyos.",
  },
  {
    mode: "archive",
    title: "Copia de seguridad: lo guarda todo",
    explanation:
      "Recibe todo y no borra nunca. Si en el móvil borras vídeos para hacer sitio, aquí siguen.",
  },
];

/**
 * What was deleted, and how to get it back.
 *
 * Syncing a deletion is the one change that syncing again cannot undo, so
 * nothing is ever destroyed: a file another device deletes is handed to this
 * computer's recycle bin, exactly where the file manager would have put it.
 * This list is the same bin, filtered to this folder — and restoring puts the
 * file back in place, which the engine then sends to every other device.
 */
function DeletedFiles({
  folder,
  onError,
}: {
  folder: SharedFolder;
  onError: (message: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [files, setFiles] = useState<DeletedFile[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const look = useCallback(async () => {
    try {
      setFiles(await api.deletedFiles(folder.id));
    } catch (e) {
      onError(String(e));
    }
  }, [folder.id, onError]);

  useEffect(() => {
    if (open) void look();
  }, [open, look]);

  async function restore(file: DeletedFile) {
    setBusy(file.id);
    try {
      await api.restoreDeleted(folder.id, file.id);
      showToast(`«${file.name}» vuelve a estar en la carpeta`);
      await look();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(null);
    }
  }

  if (!open) {
    return (
      <button className="btn btn-quiet" onClick={() => setOpen(true)}>
        <TrashIcon />
        Buscar ficheros borrados
      </button>
    );
  }

  return (
    <div className="folder-password">
      <p className="section">
        Ficheros borrados
        <Help>
          Cuando otro dispositivo borra algo, aquí no se destruye: va a la papelera de este
          ordenador, la misma que abre el gestor de archivos. Recuperar uno lo devuelve a su sitio,
          y desde ahí vuelve solo al resto de dispositivos.
        </Help>
      </p>
      {files === null ? (
        <p className="hint">Mirando en la papelera…</p>
      ) : files.length === 0 ? (
        <p className="hint">No hay nada borrado de esta carpeta.</p>
      ) : (
        <ul className="peers">
          {files.map((file) => (
            <li key={file.id}>
              {file.name}
              <span className="muted">{formatBytes(file.bytes)}</span>
              <span className="muted">{timeAgo(file.deletedAt)}</span>
              <button
                className="btn btn-small"
                disabled={busy === file.id}
                onClick={() => void restore(file)}
              >
                {busy === file.id ? "Recuperando…" : "Recuperar"}
              </button>
            </li>
          ))}
        </ul>
      )}
      <button className="btn btn-quiet" onClick={() => setOpen(false)}>
        Ocultar
      </button>
    </div>
  );
}

/**
 * Handing a folder out as a link, for people who will not install anything.
 *
 * This is not syncing and does not pretend to be. There is no account behind
 * it — a Cloudflare Quick Tunnel needs none — which is the same reason it is
 * temporary: a quick tunnel gets a new address every time and Cloudflare
 * promises no uptime, so pretending otherwise would be a lie the app tells on
 * the user's behalf. What is on screen instead is the truth: a countdown to
 * when it stops, and that closing HomeCloud stops it early.
 */
function ShareLink({
  folder,
  onError,
}: {
  folder: SharedFolder;
  onError: (message: string) => void;
}) {
  const [status, setStatus] = useState<LinkStatus | null>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    void api.linkFor(folder.id).then(setStatus).catch(() => undefined);
  }, [folder.id]);

  // Only ticks while a link is actually up: a countdown nobody is showing
  // costs nothing to skip.
  useEffect(() => {
    if (!status) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [status]);

  async function start() {
    setBusy(true);
    try {
      const created = await api.linkStart(folder.id, folder.path, password);
      setStatus(created);
      // Creating the link and then making the user click a second button to
      // get it onto the clipboard is one step more than the moment calls
      // for: copying it straight away is what "give me a link to send" means.
      const ok = await copyText(created.url);
      showToast(t(ok ? "Enlace copiado" : "No se pudo copiar el enlace"));
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    setBusy(true);
    try {
      await api.linkStop(folder.id);
      setStatus(null);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const secondsLeft = status ? Math.max(0, Math.round(status.expiresAt - now / 1000)) : 0;
  // hcshare stops itself once its time is up; a link still shown as live past
  // that point would be this screen lying about something it can check.
  if (status && secondsLeft <= 0) {
    setStatus(null);
  }

  return (
    <div className="folder-password">
      <p className="section">
        {t("Compartir por enlace")}
        <Help>
          {t(
            "Cualquiera con el enlace podrá ver y descargar lo que hay en esta carpeta, sin instalar nada. Solo lectura: no podrá cambiar ni borrar nada. Caduca solo a las pocas horas, sin cuenta en ningún sitio. Sin contraseña, lo abre quien tenga el enlace; con ella, la comprueba este ordenador, no un tercero.",
          )}
        </Help>
      </p>

      {status && secondsLeft > 0 ? (
        <>
          <p className="destination-path">
            <span>{status.url}</span>
          </p>
          <p className="hint">
            {tf("Caduca en {n}, o antes si cierras HomeCloud en este ordenador.", formatCountdown(secondsLeft))}
          </p>
          <div className="destination-actions">
            <button
              className="btn btn-small"
              onClick={async () => {
                const ok = await copyText(status.url);
                setCopied(ok);
                showToast(t(ok ? "Enlace copiado" : "No se pudo copiar el enlace"));
                setTimeout(() => setCopied(false), 2000);
              }}
            >
              {copied ? <CheckIcon /> : <CopyIcon />}
              {copied ? t("Copiado") : t("Copiar el enlace")}
            </button>
            <button className="btn btn-small btn-quiet" onClick={stop} disabled={busy}>
              {t("Dejar de compartir")}
            </button>
          </div>
        </>
      ) : (
        <>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={t("Contraseña para el enlace (opcional)")}
          />
          <button className="btn btn-small btn-primary" onClick={start} disabled={busy}>
            <LinkIcon />
            {busy ? t("Creando el enlace…") : t("Crear el enlace")}
          </button>
        </>
      )}
    </div>
  );
}

function formatCountdown(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours > 0) return `${hours} h ${minutes} min`;
  if (minutes > 0) return `${minutes} min`;
  return `${seconds} s`;
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
