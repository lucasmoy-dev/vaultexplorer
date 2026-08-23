//! Translation between English, Spanish and French -- offline, via Argos
//! Translate.
//!
//! Whisper can only translate *into* English, so "English meeting, Spanish
//! subtitles" needs a real MT step. Argos (OpenNMT models, CPU) runs
//! locally, costs nothing per line, and keeps the audio of every meeting
//! on this machine, which a cloud translation API would not.
//!
//! It is a Python library, so it lives in its own virtualenv under
//! `~/.local/share/livesubs/venv`, created on demand from the settings
//! window ("Instalar motor de traducción") rather than assumed to exist.
//! Talking to it is a long-lived worker process over JSON lines (see
//! `py/translate_server.py`): importing argostranslate and loading a model
//! takes seconds, which is fine once and unacceptable per subtitle.
//!
//! Only the four English pairs are installed (en<->es, en<->fr). Argos
//! pivots through English by itself, so Spanish<->French works without a
//! direct package -- and a pivot is worth far more than the ~1GB the
//! remaining direct pairs would cost.
//!
//! One sharp edge, worth the paragraph: argostranslate imports `stanza`
//! for sentence-boundary detection, `stanza` depends on torch, and pip's
//! default torch wheel drags in ~3.5GB of NVIDIA CUDA libraries. On this
//! machine (Intel iGPU) not one byte of that can ever be loaded -- the
//! first working install here weighed 4.8GB. So torch is installed from
//! PyTorch's CPU-only index *before* argostranslate, which brings the
//! whole environment down to ~1.4GB, and an environment built the old way
//! is detected and rebuilt rather than left on disk.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// The four packages that, with pivoting, cover every direction between
/// English, Spanish and French.
const PAIRS: &[(&str, &str)] = &[("en", "es"), ("es", "en"), ("en", "fr"), ("fr", "en")];

/// Pinned: this is the version the install steps and the worker protocol
/// were verified against, and argostranslate's dependency set (which is
/// where the 3.5GB CUDA trap lives) has moved before. One number to bump
/// deliberately, rather than an install that changes under the user.
const ARGOS_VERSION: &str = "1.11.0";

/// PyTorch's CPU-only wheel index. Without this, `pip install
/// argostranslate` resolves torch to the CUDA build.
const TORCH_CPU_INDEX: &str = "https://download.pytorch.org/whl/cpu";

const SERVER_SOURCE: &str = include_str!("py/translate_server.py");

pub fn venv_dir() -> PathBuf {
    crate::settings::data_dir().join("venv")
}

fn venv_python() -> PathBuf {
    venv_dir().join("bin/python")
}

fn server_script() -> PathBuf {
    crate::settings::data_dir().join("translate_server.py")
}

/// A marker written once the packages are in place. Checking for the venv
/// alone isn't enough: a half-finished install (venv created, pip still
/// downloading) would otherwise look ready and then fail on the first
/// caption.
fn ready_marker() -> PathBuf {
    crate::settings::data_dir().join("translate-ready")
}

pub fn engine_installed() -> bool {
    venv_python().is_file() && ready_marker().is_file()
}

/// Create the venv, install argostranslate, and fetch the language
/// packages. Slow (hundreds of MB), so it reports progress as plain
/// strings the settings window shows verbatim.
pub fn install(mut progress: impl FnMut(&str)) -> Result<(), String> {
    std::fs::create_dir_all(crate::settings::data_dir()).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(ready_marker());

    if has_cuda_wheels() {
        progress("Quitando el entorno anterior (traía librerías CUDA que este equipo no usa)…");
        std::fs::remove_dir_all(venv_dir()).map_err(|e| e.to_string())?;
    }

    if !venv_python().is_file() {
        progress("Creando el entorno de Python…");
        let out = Command::new("python3")
            .args(["-m", "venv"])
            .arg(venv_dir())
            .output()
            .map_err(|e| format!("no se pudo ejecutar python3: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "no se pudo crear el entorno virtual: {}\nInstala python3-venv: sudo apt install python3-venv",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }

    let out = Command::new(venv_python())
        .args(["-m", "pip", "install", "--upgrade", "--quiet", "pip"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }

    // Torch first, and from the CPU index: installed after (or by)
    // argostranslate it resolves to the CUDA build and the environment
    // grows by ~3.5GB of libraries this machine cannot use. See the module
    // comment.
    progress("Instalando PyTorch (versión CPU, ~750MB)…");
    let out = Command::new(venv_python())
        .args(["-m", "pip", "install", "--quiet", "torch", "--index-url", TORCH_CPU_INDEX])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "pip install torch falló: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    progress("Instalando argostranslate…");
    let out = Command::new(venv_python())
        .args(["-m", "pip", "install", "--quiet", &format!("argostranslate=={ARGOS_VERSION}")])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "pip install argostranslate falló: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    progress("Descargando los modelos de idioma (en↔es, en↔fr)…");
    let wanted = PAIRS
        .iter()
        .map(|(from, to)| format!("(\"{from}\",\"{to}\")"))
        .collect::<Vec<_>>()
        .join(",");
    // Run as a script rather than -c: the package index and downloads are
    // argostranslate's own code path, and doing it any other way would
    // mean reimplementing its model repository handling here.
    let script = format!(
        r#"
import argostranslate.package as pkg
pkg.update_package_index()
available = pkg.get_available_packages()
wanted = {{{wanted}}}
installed = {{(p.from_code, p.to_code) for p in pkg.get_installed_packages()}}
for p in available:
    key = (p.from_code, p.to_code)
    if key in wanted and key not in installed:
        print("descargando %s -> %s" % key, flush=True)
        pkg.install_from_path(p.download())
missing = wanted - {{(p.from_code, p.to_code) for p in pkg.get_installed_packages()}}
if missing:
    raise SystemExit("faltan paquetes: %s" % sorted(missing))
print("listo", flush=True)
"#
    );
    let script_path = crate::settings::data_dir().join("install_packages.py");
    std::fs::write(&script_path, script).map_err(|e| e.to_string())?;
    let out = Command::new(venv_python())
        .arg(&script_path)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&script_path);
    if !out.status.success() {
        return Err(format!(
            "no se pudieron descargar los modelos: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    std::fs::write(server_script(), SERVER_SOURCE).map_err(|e| e.to_string())?;
    std::fs::write(ready_marker(), "ok").map_err(|e| e.to_string())?;
    progress("Motor de traducción listo.");
    Ok(())
}

/// Whether the environment on disk was built with the CUDA torch wheel
/// (i.e. before this module started pinning the CPU index). Recognised by
/// the `nvidia` package pip drops in `site-packages`.
fn has_cuda_wheels() -> bool {
    let Ok(entries) = std::fs::read_dir(venv_dir().join("lib")) else {
        return false;
    };
    for entry in entries.flatten() {
        if entry.path().join("site-packages/nvidia").is_dir() {
            return true;
        }
    }
    false
}

struct Worker {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The translator, started on first use and kept alive. Serialised behind
/// a mutex: one worker is enough for two subtitle streams (a line is tens
/// of milliseconds), and a second process would double the memory for the
/// same models.
pub struct Translator {
    worker: Mutex<Option<Worker>>,
    next_id: AtomicU64,
}

impl Default for Translator {
    fn default() -> Self {
        Translator { worker: Mutex::new(None), next_id: AtomicU64::new(1) }
    }
}

impl Translator {
    fn spawn() -> Result<Worker, String> {
        if !engine_installed() {
            return Err("el motor de traducción no está instalado".to_string());
        }
        // Rewritten every start so an app update ships a new worker
        // without the user reinstalling anything.
        std::fs::write(server_script(), SERVER_SOURCE).map_err(|e| e.to_string())?;
        let mut child = Command::new(venv_python())
            .arg(server_script())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo iniciar el traductor: {e}"))?;
        let stdin = child.stdin.take().ok_or("sin stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("sin stdout")?);
        let mut ready = String::new();
        // The worker announces itself; a broken install shows up here
        // instead of as subtitles that quietly never translate.
        if stdout.read_line(&mut ready).map_err(|e| e.to_string())? == 0 {
            return Err("el traductor terminó al arrancar".to_string());
        }
        let parsed: serde_json::Value = serde_json::from_str(ready.trim()).unwrap_or_default();
        if let Some(error) = parsed.get("error").and_then(|v| v.as_str()) {
            return Err(error.to_string());
        }
        Ok(Worker { child, stdin, stdout })
    }

    /// Translate one line. `from`/`to` are ISO codes ("en"/"es"/"fr").
    pub fn translate(&self, text: &str, from: &str, to: &str) -> Result<String, String> {
        if from == to || text.trim().is_empty() {
            return Ok(text.to_string());
        }
        let mut guard = self.worker.lock().map_err(|_| "translator mutex")?;
        if guard.is_none() {
            *guard = Some(Self::spawn()?);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = serde_json::json!({ "id": id, "text": text, "from": from, "to": to });
        let worker = guard.as_mut().expect("spawned above");
        let sent = writeln!(worker.stdin, "{request}").and_then(|()| worker.stdin.flush());
        let line = match sent {
            Ok(()) => {
                let mut line = String::new();
                match worker.stdout.read_line(&mut line) {
                    Ok(0) | Err(_) => None,
                    Ok(_) => Some(line),
                }
            }
            Err(_) => None,
        };
        let Some(line) = line else {
            // The worker died (OOM-killed, or a model file went away).
            // Drop it so the next caption starts a fresh one instead of
            // every later line failing against a corpse.
            *guard = None;
            return Err("el traductor se cerró; se reiniciará en la próxima frase".to_string());
        };
        let parsed: serde_json::Value =
            serde_json::from_str(line.trim()).map_err(|e| format!("respuesta ilegible: {e}"))?;
        if let Some(error) = parsed.get("error").and_then(|v| v.as_str()) {
            return Err(error.to_string());
        }
        Ok(parsed
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or(text)
            .to_string())
    }

    /// Drop the worker (used when translation is turned off, so its
    /// hundreds of MB of models don't sit in RAM for a feature nobody is
    /// using).
    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.worker.lock() {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_language_short_circuits_without_a_worker() {
        let translator = Translator::default();
        // No venv needed: this must not even try to spawn Python.
        assert_eq!(translator.translate("hola", "es", "es").unwrap(), "hola");
        assert_eq!(translator.translate("   ", "en", "es").unwrap(), "   ");
    }

    /// The real thing: create the venv, install argostranslate, fetch the
    /// packages, then translate in both directions and through the English
    /// pivot. `#[ignore]`d because it downloads ~500MB and takes minutes --
    /// run it on purpose after touching this module:
    ///
    /// ```text
    /// cargo test --features custom-protocol -- --ignored --nocapture translate
    /// ```
    #[test]
    #[ignore]
    fn install_then_translate_in_every_direction() {
        install(|step| println!("{step}")).expect("install failed");
        assert!(engine_installed());
        let translator = Translator::default();

        let to_spanish = translator
            .translate("Good morning, shall we start the meeting?", "en", "es")
            .expect("en->es failed");
        println!("en->es: {to_spanish}");
        assert!(to_spanish.to_lowercase().contains("reuni"), "{to_spanish}");

        let to_english = translator.translate("Buenos días, ¿empezamos?", "es", "en").expect("es->en failed");
        println!("es->en: {to_english}");
        assert!(to_english.to_lowercase().contains("morning"), "{to_english}");

        let to_french = translator.translate("The meeting starts at ten.", "en", "fr").expect("en->fr failed");
        println!("en->fr: {to_french}");
        assert!(to_french.to_lowercase().contains("réunion"), "{to_french}");

        // No direct es->fr package exists; this only works if Argos pivots
        // through English, which is the assumption the installed set rests
        // on.
        let pivoted = translator.translate("La reunión empieza a las diez.", "es", "fr").expect("es->fr failed");
        println!("es->fr (pivot): {pivoted}");
        assert!(pivoted.to_lowercase().contains("réunion"), "{pivoted}");
    }

    #[test]
    fn pairs_cover_every_direction_through_english() {
        for a in ["en", "es", "fr"] {
            for b in ["en", "es", "fr"] {
                if a == b {
                    continue;
                }
                let direct = PAIRS.contains(&(a, b));
                let pivot = PAIRS.contains(&(a, "en")) && PAIRS.contains(&("en", b));
                assert!(direct || pivot, "no route from {a} to {b}");
            }
        }
    }
}
