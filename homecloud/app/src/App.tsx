import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  api,
  formatBytes,
  peerSummary,
  type CodePreview,
  type Invitation,
  type Readiness,
  type SharedFolder,
} from "./api";
import { StatusDot, stateLabel } from "./StatusDot";
import { PairingCard } from "./PairingCard";
import { SettingsSheet } from "./SettingsSheet";

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
                <span className="folder-state">{stateLabel(folder.state)}</span>
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
          Compartir carpeta
        </button>
        <button className="btn" onClick={() => setScreen({ name: "join" })}>
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

function GearIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7">
      <circle cx="12" cy="12" r="3.1" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
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
            <strong>{invitation.fromDeviceName}</strong> quiere compartir «
            <strong>{invitation.folder.label}</strong>»
          </>
        ) : (
          <>
            <strong>{invitation.fromDeviceName}</strong> quiere conectarse con este dispositivo
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
  const [path, setPath] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Reading the code as it is pasted means the user finds out it is wrong
  // immediately, rather than after committing to a destination folder.
  useEffect(() => {
    if (code.trim().length < 8) {
      setPreview(null);
      setProblem(null);
      return;
    }
    let cancelled = false;
    api
      .previewCode(code)
      .then((p) => {
        if (cancelled) return;
        setPreview(p);
        setPath(p.suggestedPath);
        setProblem(null);
      })
      .catch((e) => {
        if (cancelled) return;
        setPreview(null);
        setProblem(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [code]);

  async function choosePath() {
    const picked = await open({ directory: true, multiple: false, title: "¿Dónde guardo la carpeta?" });
    if (typeof picked === "string") setPath(picked);
  }

  async function join() {
    setBusy(true);
    try {
      await api.redeemCode(code, path);
      onJoined();
    } catch (e) {
      setProblem(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="join">
      <label className="field">
        <span>Pega aquí el código del otro dispositivo</span>
        <textarea
          value={code}
          onChange={(e) => setCode(e.target.value)}
          placeholder="HC1…"
          rows={3}
          autoFocus
        />
      </label>

      {problem && <p className="problem">{problem}</p>}

      {preview && (
        <>
          <p className="preview">
            <strong>{preview.deviceName}</strong> comparte «<strong>{preview.folderLabel}</strong>»
          </p>
          <label className="field">
            <span>Se guardará en</span>
            <button className="path-picker" onClick={choosePath} type="button">
              {path}
            </button>
          </label>
          <button className="btn btn-primary" onClick={join} disabled={busy || !path}>
            {busy ? "Conectando…" : "Unirme"}
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
  const paused = folder.state.kind === "paused";

  async function showCode() {
    try {
      setCode(await api.codeFor(folder.id));
    } catch (e) {
      onError(String(e));
    }
  }

  if (code) return <PairingCard code={code} label={folder.label} />;

  return (
    <div className="sheet-body">
      <p className="sheet-line">
        <StatusDot state={folder.state} /> {stateLabel(folder.state)}
      </p>
      <p className="sheet-line muted">
        {folder.files} ficheros · {formatBytes(folder.bytes)}
      </p>
      <button className="path-picker" onClick={() => void revealItemInDir(folder.path)} type="button">
        {folder.path}
      </button>

      {folder.peers.length > 0 && (
        <ul className="peers">
          {folder.peers.map((peer) => (
            <li key={peer.id}>
              <span className={`dot ${peer.connected ? "dot-ok" : "dot-idle"}`} aria-hidden />
              {peer.name}
              <span className="muted">{peer.connected ? "conectado" : "sin conexión"}</span>
            </li>
          ))}
        </ul>
      )}

      <div className="sheet-actions">
        <button className="btn" onClick={showCode}>
          Añadir otro dispositivo
        </button>
        <button
          className="btn"
          onClick={async () => {
            try {
              await api.setFolderPaused(folder.id, !paused);
              onChanged();
            } catch (e) {
              onError(String(e));
            }
          }}
        >
          {paused ? "Reanudar" : "Pausar"}
        </button>
        {confirmingStop ? (
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
            Sí, dejar de sincronizar
          </button>
        ) : (
          <button className="btn btn-quiet" onClick={() => setConfirmingStop(true)}>
            Dejar de sincronizar
          </button>
        )}
      </div>
      {confirmingStop && (
        <p className="muted small">
          Los ficheros que ya están en este ordenador se quedan donde están. Solo se deja de
          sincronizar.
        </p>
      )}
    </div>
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
          <button className="btn btn-quiet" onClick={onClose} aria-label="Cerrar">
            ✕
          </button>
        </header>
        {children}
      </section>
    </div>
  );
}
