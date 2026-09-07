/**
 * Every word the desktop shows, in the two languages it speaks.
 *
 * A flat table rather than a library: there are under a hundred strings and one
 * plural rule, and a translation framework would be more machinery than the
 * thing it translates. The keys are English sentences so a missing translation
 * still reads as something a person can act on.
 */

export type Language = "es" | "en";

const ENGLISH = {
  "Compartir carpeta": "Share a folder",
  "Unirme con un código": "Join with a code",
  "Unirme a una carpeta": "Join a folder",
  "Todavía no compartes nada": "You are not sharing anything yet",
  "Comparte una carpeta de este ordenador, o únete a una que ya exista en otro dispositivo.":
    "Share a folder from this computer, or join one that already exists on another device.",
  Ajustes: "Settings",
  Cerrar: "Close",
  Cancelar: "Cancel",
  Guardar: "Save",
  Guardado: "Saved",
  Unirme: "Join",
  "Conectando…": "Connecting…",
  "Arrancando…": "Starting…",
  Reintentar: "Retry",
  "Reintentando…": "Retrying…",
  "HomeCloud no pudo arrancar": "HomeCloud could not start",
  "Escanear el QR del otro dispositivo": "Scan the other device's QR",
  "…o pega aquí el código": "…or paste the code here",
  "Pega aquí el código del otro dispositivo": "Paste the other device's code here",
  "Cambiar carpeta": "Change folder",
  "Usar esa carpeta tal cual": "Use that folder as it is",
  "Crear una subcarpeta dentro": "Create a subfolder inside",
  "Encuadra el QR dentro del recuadro.": "Line the QR up inside the frame.",
  "Copiar código": "Copy code",
  Copiado: "Copied",
  "No se pudo copiar": "Could not copy",
  "Escanea esto desde el otro dispositivo, o pega el código.":
    "Scan this from the other device, or paste the code.",
  "No se pudo dibujar el QR": "The QR could not be drawn",
  "Añadir otro dispositivo": "Add another device",
  Pausar: "Pause",
  Reanudar: "Resume",
  "Dejar de sincronizar": "Stop syncing",
  "Sí, dejar de sincronizar": "Yes, stop syncing",
  "Solo lectura": "Read only",
  "Recibe los cambios de los demás, pero nunca envía los suyos.":
    "Takes changes from the others, but never sends its own.",
  "Ajustes de la carpeta": "Folder settings",
  "Nombre de este dispositivo": "This device's name",
  "Es el nombre que ven los demás dispositivos al conectarse.":
    "This is what other devices see when they connect.",
  Avanzado: "Advanced",
  Idioma: "Language",
  Español: "Spanish",
  Inglés: "English",
  "Solo en mi red local": "Only on my local network",
  "Guardar versiones anteriores": "Keep previous versions",
  "No guardar": "Do not keep any",
  "Las 5 últimas": "The last 5",
  "Las 10 últimas": "The last 10",
  "Las 25 últimas": "The last 25",
  "Límite de subida": "Upload limit",
  "Límite de bajada": "Download limit",
  "En kB/s. 0 significa sin límite.": "In kB/s. 0 means no limit.",
  "Dispositivos olvidados": "Forgotten devices",
  "Limpiar dispositivos que no comparten nada": "Forget devices that share nothing",
  "Limpiando…": "Cleaning…",
  "No había ninguno que sobrara.": "There were none to spare.",
  "Este dispositivo": "This device",
  conectado: "connected",
  "sin conexión": "offline",
  "Sin dispositivos todavía": "No devices yet",
  dispositivos: "devices",
  conectados: "connected",
  "Al día": "Up to date",
  Sincronizando: "Syncing",
  Pausada: "Paused",
  "Sin conexión": "Disconnected",
  "Se guardará en": "It will be saved in",
  "Se sincronizará esta carpeta:": "This folder will be synced:",
  "Se creará una carpeta nueva:": "A new folder will be created:",
  comparte: "shares",
  "Motor de sincronización: Syncthing": "Sync engine: Syncthing",
  "Abrir en el explorador de archivos": "Open in the file manager",
  "Los ficheros que ya están en este ordenador se quedan donde están. Solo se deja de sincronizar.":
    "The files already on this computer stay where they are. Only the syncing stops.",
  "Compartir por enlace": "Share as a link",
  "Enlace copiado": "Link copied",
  "No se pudo copiar el enlace": "Could not copy the link",
  "Copiar el enlace": "Copy the link",
  "Dejar de compartir": "Stop sharing",
  "Crear el enlace": "Create the link",
  "Creando el enlace…": "Creating the link…",
  "Contraseña para el enlace (opcional)": "Password for the link (optional)",
  "Caduca en {n}, o antes si cierras HomeCloud en este ordenador.":
    "Expires in {n}, or sooner if you close HomeCloud on this computer.",
  "Cualquiera con el enlace podrá ver y descargar lo que hay en esta carpeta, desde un navegador y sin instalar nada. No podrá cambiar ni borrar nada. El enlace caduca solo a las pocas horas: no hace falta cuenta en ningún sitio, y esa es la contrapartida.":
    "Anyone with the link can view and download what is in this folder, from a browser, with nothing to install. They cannot change or delete anything. The link expires on its own after a few hours — no account is needed anywhere, and that is the trade-off.",
  "Sin contraseña, el enlace lo abre quien lo tenga. Con ella, la comprueba este ordenador, no un tercero.":
    "With no password, whoever has the link can open it. With one, this computer checks it, not a third party.",
} satisfies Record<string, string>;

let current: Language = "es";

export function setLanguage(language: Language) {
  current = language;
}

export function getLanguage(): Language {
  return current;
}

/**
 * The Spanish string is the key, so an untranslated one still shows Spanish
 * rather than a placeholder.
 */
export function t(spanish: string): string {
  if (current === "es") return spanish;
  return (ENGLISH as Record<string, string>)[spanish] ?? spanish;
}

/**
 * Like {@link t}, for a string carrying one placeholder — `key` holds the
 * Spanish sentence with `{n}` where a value gets substituted, and this looks
 * up the translation before substituting so the placeholder survives the
 * lookup untouched.
 */
export function tf(key: string, value: string): string {
  return t(key).replace("{n}", value);
}
