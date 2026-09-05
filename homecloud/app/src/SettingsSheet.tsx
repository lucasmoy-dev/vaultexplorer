import { useEffect, useState } from "react";
import { copyText } from "./clipboard";
import { api, type Settings } from "./api";
import { BroomIcon, CheckIcon, CopyIcon } from "./Icons";

/**
 * Everything that is not a folder. The device name is at the top because it is
 * the only setting most people ever open this for; the rest is below a line and
 * safe to ignore.
 */
export function SettingsSheet({ onSaved }: { onSaved: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [copied, setCopied] = useState(false);
  const [tidyState, setTidyState] = useState<string | null>(null);

  useEffect(() => {
    api.settings().then(setSettings).catch((e) => setProblem(String(e)));
  }, []);

  if (!settings) {
    return <p className="muted">{problem ?? "Cargando…"}</p>;
  }

  function edit(patch: Partial<Settings>) {
    setSettings((current) => (current ? { ...current, ...patch } : current));
    setSaved(false);
  }

  async function save() {
    if (!settings) return;
    try {
      await api.saveSettings(settings);
      setProblem(null);
      setSaved(true);
      onSaved();
    } catch (e) {
      setProblem(String(e));
    }
  }

  async function copyId() {
    setCopied(await copyText(settings!.deviceId));
    setTimeout(() => setCopied(false), 1600);
  }

  async function forgetUnused() {
    setTidyState("working");
    try {
      const dropped = await api.forgetUnusedDevices();
      setTidyState(
        dropped.length === 0
          ? "No había ninguno que sobrara."
          : `Se quitaron ${dropped.length}: ${dropped.join(", ")}.`,
      );
    } catch (e) {
      setTidyState(String(e));
    }
  }

  return (
    <div className="settings">
      <label className="field">
        <span>Nombre de este dispositivo</span>
        <input
          value={settings.deviceName}
          onChange={(e) => edit({ deviceName: e.target.value })}
          placeholder="Portátil de Lucas"
        />
      </label>
      <p className="hint">Es el nombre que ven los demás dispositivos al conectarse.</p>

      <label className="field">
        <span>Idioma</span>
        <select
          value={settings.language}
          onChange={(e) => edit({ language: e.target.value as "es" | "en" })}
        >
          <option value="es">Español</option>
          <option value="en">Inglés</option>
        </select>
      </label>

      <hr className="rule" />
      <p className="section">Avanzado</p>

      <label className="toggle">
        <input
          type="checkbox"
          checked={settings.localNetworkOnly}
          onChange={(e) => edit({ localNetworkOnly: e.target.checked })}
        />
        <span>
          Solo en mi red local
          <em>
            No se anuncia por internet ni usa repetidores. Sincroniza en casa, y fuera de casa no.
          </em>
        </span>
      </label>

      <label className="field">
        <span>Guardar versiones anteriores</span>
        <select
          value={settings.keepVersions}
          onChange={(e) => edit({ keepVersions: Number(e.target.value) })}
        >
          <option value={0}>No guardar</option>
          <option value={5}>Las 5 últimas</option>
          <option value={10}>Las 10 últimas</option>
          <option value={25}>Las 25 últimas</option>
        </select>
      </label>
      <p className="hint">
        Cuando un fichero cambia o se borra en otro dispositivo, se guarda una copia de la versión
        anterior en <code>.stversions</code>. Es lo que separa «se sincronizó un borrado» de «perdí
        el fichero».
      </p>

      <div className="pair">
        <label className="field">
          <span>Límite de subida</span>
          <input
            type="number"
            min={0}
            value={settings.uploadLimitKbps}
            onChange={(e) => edit({ uploadLimitKbps: Math.max(0, Number(e.target.value)) })}
          />
        </label>
        <label className="field">
          <span>Límite de bajada</span>
          <input
            type="number"
            min={0}
            value={settings.downloadLimitKbps}
            onChange={(e) => edit({ downloadLimitKbps: Math.max(0, Number(e.target.value)) })}
          />
        </label>
      </div>
      <p className="hint">En kB/s. 0 significa sin límite.</p>

      <hr className="rule" />

      <p className="section">Dispositivos olvidados</p>
      <p className="hint">
        Reinstalar la app en un teléfono le da una identidad nueva, y la vieja se queda en la lista
        con el mismo nombre. Esto quita las que ya no comparten ninguna carpeta. No borra ficheros.
      </p>
      <button className="btn" onClick={forgetUnused} disabled={tidyState === "working"}>
        <BroomIcon />
        {tidyState === "working" ? "Limpiando…" : "Limpiar dispositivos que no comparten nada"}
      </button>
      {tidyState && tidyState !== "working" && <p className="hint">{tidyState}</p>}

      <hr className="rule" />

      <p className="section">Este dispositivo</p>
      <button className="path-open" onClick={copyId} type="button" title="Copiar">
        {copied ? <CheckIcon /> : <CopyIcon />}
        <span>{copied ? "Copiado" : settings.deviceId}</span>
      </button>
      <p className="hint">
        Motor de sincronización: Syncthing {settings.engineVersion}
      </p>

      {problem && <p className="problem">{problem}</p>}

      <button className="btn btn-primary" onClick={save}>
        {saved && <CheckIcon />}
        {saved ? "Guardado" : "Guardar"}
      </button>
    </div>
  );
}
