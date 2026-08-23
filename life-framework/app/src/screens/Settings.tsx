import { useEffect, useState } from "react";
import { useStore } from "../store";
import { uid } from "../seed";
import type { Area, DB } from "../types";
import * as api from "../api";

const PALETTE = ["#22c55e", "#f59e0b", "#3b82f6", "#a855f7", "#14b8a6", "#ec4899", "#ef4444", "#eab308"];

export function Settings({ onOpenArea }: { onOpenArea: (id: string) => void }) {
  const { db, dispatch, replaceDb } = useStore();

  const addArea = () => {
    const order = db.areas.reduce((m, a) => Math.max(m, a.order), -1) + 1;
    const color = PALETTE[order % PALETTE.length];
    const area: Area = {
      id: uid("area"),
      name: "Nueva área",
      color,
      icon: "⭐",
      weight: 1,
      order,
      subs: [
        {
          id: uid("sub"),
          name: "General",
          weight: 1,
          questions: [{ id: uid("q"), text: "Nueva pregunta", weight: 1, kind: "scale", scaleMin: 1, scaleMax: 10 }],
        },
      ],
    };
    dispatch({ type: "ADD_AREA", area });
    onOpenArea(area.id);
  };

  return (
    <div className="screen settings">
      <header className="simple-head">
        <h1>⚙️ Ajustes</h1>
      </header>

      <section className="card">
        <h2>Áreas</h2>
        {[...db.areas]
          .sort((a, b) => a.order - b.order)
          .map((a) => (
            <button key={a.id} className="area-row" onClick={() => onOpenArea(a.id)}>
              <span>{a.icon}</span>
              <span className="area-row-name" style={{ color: a.color }}>
                {a.name}
              </span>
              <span className="area-row-meta">
                {a.subs.length} sub · {a.subs.reduce((n, s) => n + s.questions.length, 0)} preg
              </span>
              <span className="chev">›</span>
            </button>
          ))}
        <button className="add-btn" onClick={addArea}>
          + Nueva área
        </button>
      </section>

      <section className="card">
        <h2>Preferencias</h2>
        <label className="field row">
          <span>Tema oscuro</span>
          <input
            type="checkbox"
            checked={db.settings.theme === "dark"}
            onChange={(e) => dispatch({ type: "UPDATE_SETTINGS", patch: { theme: e.target.checked ? "dark" : "light" } })}
          />
        </label>
        <label className="field">
          <span>Recordar check-in cada {db.settings.reminderDays} días</span>
          <input
            type="range"
            min={1}
            max={30}
            value={db.settings.reminderDays}
            onChange={(e) => dispatch({ type: "UPDATE_SETTINGS", patch: { reminderDays: Number(e.target.value) } })}
          />
        </label>
      </section>

      <section className="card">
        <h2>Copia de seguridad</h2>
        <p className="muted small">Tus datos viven solo en este dispositivo. Exportá un JSON para respaldarlos.</p>
        <div className="row-btns">
          <button
            onClick={async () => {
              const dest = await api.exportDb(JSON.stringify(db, null, 2));
              if (dest) alert("Exportado a:\n" + dest);
            }}
          >
            Exportar
          </button>
          <button
            onClick={async () => {
              const json = await api.importDb();
              if (!json) return;
              try {
                const parsed = JSON.parse(json) as DB;
                if (!Array.isArray(parsed.areas)) throw new Error("formato inválido");
                if (confirm("Esto reemplaza todos tus datos actuales. ¿Continuar?")) replaceDb(parsed);
              } catch {
                alert("No pude leer ese archivo.");
              }
            }}
          >
            Importar
          </button>
        </div>
        <button
          className="danger small"
          onClick={() => {
            if (confirm("¿Restablecer la plantilla inicial? Se pierden tus datos.")) dispatch({ type: "RESET_SEED" });
          }}
        >
          Restablecer plantilla
        </button>
      </section>

      <UpdatePanel repo={db.settings.updateRepo} />
    </div>
  );
}

function UpdatePanel({ repo }: { repo: string }) {
  const [info, setInfo] = useState<api.UpdateInfo | null>(null);
  const [version, setVersion] = useState("…");
  const [autoInstall, setAutoInstall] = useState(false);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    api.appVersion().then(setVersion);
    api.canAutoInstall().then(setAutoInstall);
  }, []);

  const check = async () => {
    setBusy("check");
    setMsg(null);
    try {
      const r = await api.checkUpdate();
      setInfo(r);
      if (!r.hasUpdate) setMsg("Estás en la última versión.");
    } catch (e) {
      setMsg("No pude chequear: " + String(e));
    } finally {
      setBusy(null);
    }
  };

  const install = async () => {
    if (!info) return;
    setBusy("install");
    setMsg("Descargando e instalando…");
    try {
      await api.downloadAndInstall(info.apkUrl);
      setMsg("Se abrió el instalador de Android.");
    } catch (e) {
      setMsg("Falló la instalación automática: " + String(e) + ". Probá el navegador.");
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="card">
      <h2>Actualizar app</h2>
      <p className="muted small">
        Versión actual: <b>{version}</b> · repo <code>{repo}</code>
      </p>
      <button onClick={check} disabled={busy !== null}>
        {busy === "check" ? "Chequeando…" : "Buscar actualización"}
      </button>

      {info?.hasUpdate && (
        <div className="update-box">
          <div className="update-ver">
            Nueva versión: <b>{info.latest}</b>
          </div>
          {info.notes && <pre className="update-notes">{info.notes}</pre>}
          <div className="row-btns">
            {autoInstall && info.apkUrl && (
              <button className="primary" onClick={install} disabled={busy !== null}>
                {busy === "install" ? "Instalando…" : "Descargar e instalar"}
              </button>
            )}
            <button onClick={() => api.openUrl(info.htmlUrl || info.apkUrl)} disabled={!info.htmlUrl && !info.apkUrl}>
              Abrir en navegador
            </button>
          </div>
        </div>
      )}

      {msg && <p className="update-msg">{msg}</p>}
    </section>
  );
}
