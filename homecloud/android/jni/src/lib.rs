//! The Android side of HomeCloud, bridged to the shared core.
//!
//! The Kotlin app owns the Syncthing process, because only Android can run a
//! foreground service and only a foreground service is allowed to keep one
//! alive. Everything above that — what a folder's status means, how an
//! invitation is answered, what a pairing code contains — comes from `homecore`,
//! so the phone and the desktop can never disagree about a code they both have
//! to read.
//!
//! The bridge is deliberately one function. A wide JNI surface is a wide surface
//! to get wrong; a single JSON in, JSON out call is checked by the same tests on
//! both sides.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use homecore::destination;
use homecore::link::{Links, DEFAULT_LIFETIME};
use homecore::model::{Invitation, Settings};
use homecore::Syncthing;
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use serde_json::{json, Value};
use tokio::runtime::Runtime;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Runtime::new().expect("the Android side cannot work without an async runtime")
    })
}

fn client_slot() -> &'static std::sync::RwLock<Option<Syncthing>> {
    static CLIENT: OnceLock<std::sync::RwLock<Option<Syncthing>>> = OnceLock::new();
    CLIENT.get_or_init(|| std::sync::RwLock::new(None))
}

/// The public-link helper. Separate from the engine slot because the phone
/// knows where its own native libraries live and the core does not.
///
/// Behind an `Arc` so a caller can take a handle and drop the lock before
/// awaiting: holding a std lock across an await would block the runtime.
fn links_slot() -> &'static std::sync::RwLock<Option<Arc<Links>>> {
    static LINKS: OnceLock<std::sync::RwLock<Option<Arc<Links>>>> = OnceLock::new();
    LINKS.get_or_init(|| std::sync::RwLock::new(None))
}

fn links() -> Result<Arc<Links>, String> {
    links_slot()
        .read()
        .map_err(|_| "the app lost track of the link helper".to_string())?
        .clone()
        .ok_or_else(|| "los enlaces no están listos todavía".to_string())
}

/// Called by the service once it has the engine listening and knows its port.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_homecloud_Native_connect(
    mut env: JNIEnv,
    _class: JClass,
    base_url: JString,
    api_key: JString,
) {
    let (Ok(base_url), Ok(api_key)) = (env.get_string(&base_url), env.get_string(&api_key)) else {
        return;
    };
    let client = Syncthing::new(String::from(base_url), String::from(api_key));
    if let Ok(mut slot) = client_slot().write() {
        *slot = Some(client);
    }
}

/// Every call the app makes. Returns `{"ok": <value>}` or `{"error": "<sentence>"}`,
/// so Kotlin has exactly one shape to handle and errors can never be mistaken
/// for results.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_homecloud_Native_call(
    mut env: JNIEnv,
    _class: JClass,
    method: JString,
    args_json: JString,
) -> jstring {
    let method: String = match env.get_string(&method) {
        Ok(s) => s.into(),
        Err(_) => return reply(&mut env, Err("could not read the request".into())),
    };
    let args: String = match env.get_string(&args_json) {
        Ok(s) => s.into(),
        Err(_) => return reply(&mut env, Err("could not read the request".into())),
    };
    let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);

    let outcome = dispatch(&method, args);
    reply(&mut env, outcome)
}

fn reply(env: &mut JNIEnv, outcome: Result<Value, String>) -> jstring {
    let body = match outcome {
        Ok(value) => json!({ "ok": value }),
        Err(message) => json!({ "error": message }),
    };
    match env.new_string(body.to_string()) {
        Ok(s) => s.into_raw(),
        // Nothing useful is left to do if even the reply cannot be allocated.
        Err(_) => std::ptr::null_mut(),
    }
}

macro_rules! arg {
    ($args:expr, $name:literal) => {
        $args[$name]
            .as_str()
            .ok_or_else(|| format!("missing {}", $name))?
            .to_string()
    };
}

fn dispatch(method: &str, args: Value) -> Result<Value, String> {
    // Answered without the engine: these are pure functions of their arguments,
    // and the interface needs them while deciding what to do, which is before
    // anything has been asked of the engine.
    match method {
        "resolveDestination" => {
            let chosen = arg!(args, "chosen");
            let label = arg!(args, "label");
            let pick = match args["pick"].as_str() {
                Some("itself") => destination::Pick::Itself,
                Some("inside") => destination::Pick::Inside,
                _ => destination::default_pick(Path::new(&chosen), &label),
            };
            let chosen = Path::new(&chosen);
            let target = destination::resolve(chosen, &label, pick);
            return Ok(json!({
                "freeBytes": homecore::disk::free_bytes(&target),
                "path": target.to_string_lossy(),
                "pick": match pick {
                    destination::Pick::Itself => "itself",
                    destination::Pick::Inside => "inside",
                },
                "explanation": destination::describe(chosen, &label, pick),
            }));
        }
        // ---- public links ------------------------------------------------
        //
        // Nothing to do with the sync engine, so these must not wait for it to
        // be up: a folder can be handed out as a link while syncing is still
        // starting, or broken.
        // Points the core at both helper binaries. The path can only come
        // from here: only the platform knows where this app's native
        // libraries were unpacked.
        "linkSetup" => {
            let share = std::path::PathBuf::from(arg!(args, "shareBinary"));
            let tunnel = std::path::PathBuf::from(arg!(args, "tunnelBinary"));
            if let Ok(mut slot) = links_slot().write() {
                *slot = Some(Arc::new(Links::new(share, tunnel)));
            }
            return Ok(Value::Null);
        }

        "linkStart" => {
            let links = links()?;
            let folder_id = arg!(args, "folderId");
            let path = arg!(args, "path");
            let auth = args["basicAuth"].as_str().unwrap_or("").to_string();
            let status = runtime()
                .block_on(links.start(
                    &folder_id,
                    std::path::Path::new(&path),
                    &auth,
                    DEFAULT_LIFETIME,
                ))
                .map_err(plain)?;
            return Ok(serde_json::to_value(status).map_err(plain)?);
        }

        "linkStop" => {
            let links = links()?;
            let folder_id = arg!(args, "folderId");
            runtime().block_on(links.stop(&folder_id)).map_err(plain)?;
            return Ok(Value::Null);
        }

        "linkFor" => {
            let links = links()?;
            return Ok(match links.status_for(&arg!(args, "folderId")) {
                Some(status) => serde_json::to_value(status).map_err(plain)?,
                None => Value::Null,
            });
        }

        _ => {}
    }

    let guard = client_slot().read().map_err(|_| "the app lost track of the engine".to_string())?;
    let client = guard.as_ref().ok_or_else(|| "the sync engine is still starting up".to_string())?;

    runtime().block_on(async {
        let value = match method {
            "thisDevice" => to_value(client.this_device().await)?,
            "folders" => to_value(client.folders().await)?,
            "invitations" => to_value(client.invitations().await)?,
            "settings" => to_value(client.settings().await)?,

            "saveSettings" => {
                let settings: Settings =
                    serde_json::from_value(args["settings"].clone()).map_err(plain)?;
                client.save_settings(&settings).await.map_err(plain)?;
                Value::Null
            }

            "shareFolder" => {
                let code = client
                    .share_folder(&arg!(args, "path"), &arg!(args, "label"))
                    .await
                    .map_err(plain)?;
                Value::String(code.encode().map_err(plain)?)
            }

            "codeFor" => Value::String(
                client.code_text_for(&arg!(args, "folderId")).await.map_err(plain)?,
            ),

            // Reads a code without acting on it, so the phone can show what is
            // being offered — and ask for a password — before touching storage.
            "previewCode" => to_value(
                client
                    .read_code(&arg!(args, "code"), args["password"].as_str())
                    .await,
            )?,

            "setFolderPassword" => {
                let password = args["password"].as_str().unwrap_or("");
                if password.trim().is_empty() {
                    client.clear_folder_password(&arg!(args, "folderId")).await.map_err(plain)?;
                } else {
                    client
                        .set_folder_password(&arg!(args, "folderId"), password)
                        .await
                        .map_err(plain)?;
                }
                Value::Null
            }

            "redeemCode" => {
                let code = client
                    .read_code(&arg!(args, "code"), args["password"].as_str())
                    .await
                    .map_err(plain)?;
                client
                    .redeem(&code, &arg!(args, "localPath"))
                    .await
                    .map_err(plain)?;
                Value::Null
            }

            "accept" => {
                let invitation: Invitation =
                    serde_json::from_value(args["invitation"].clone()).map_err(plain)?;
                let local_path = args["localPath"].as_str();
                client.accept(&invitation, local_path).await.map_err(plain)?;
                Value::Null
            }

            "decline" => {
                let invitation: Invitation =
                    serde_json::from_value(args["invitation"].clone()).map_err(plain)?;
                client.decline(&invitation).await.map_err(plain)?;
                Value::Null
            }

            "setFolderPaused" => {
                let paused = args["paused"].as_bool().ok_or("missing paused")?;
                client
                    .set_folder_paused(&arg!(args, "folderId"), paused)
                    .await
                    .map_err(plain)?;
                Value::Null
            }

            "stopSharing" => {
                client.stop_sharing(&arg!(args, "folderId")).await.map_err(plain)?;
                Value::Null
            }

            "ping" => {
                client.ping().await.map_err(plain)?;
                Value::Null
            }

            "ensureDeviceName" => Value::String(
                client.ensure_device_name(&arg!(args, "fallback")).await.map_err(plain)?,
            ),

            "forgetUnusedDevices" => to_value(client.forget_unused_devices().await)?,

            "rescan" => {
                client.rescan(&arg!(args, "folderId")).await.map_err(plain)?;
                Value::Null
            }

            "setFolderWifiOnly" => {
                let wifi_only = args["wifiOnly"].as_bool().ok_or("missing wifiOnly")?;
                client
                    .set_folder_wifi_only(&arg!(args, "folderId"), wifi_only)
                    .await
                    .map_err(plain)?;
                Value::Null
            }

            "applyMeteredPolicy" => {
                let metered = args["metered"].as_bool().ok_or("missing metered")?;
                to_value(client.apply_metered_policy(metered).await)?
            }

            "reconnectAll" => {
                client.reconnect_all().await.map_err(plain)?;
                Value::Null
            }

            "setFolderReadOnly" => {
                let read_only = args["readOnly"].as_bool().ok_or("missing readOnly")?;
                client
                    .set_folder_read_only(&arg!(args, "folderId"), read_only)
                    .await
                    .map_err(plain)?;
                Value::Null
            }

            // The phone tells the core where it may put a folder it accepts on
            // its own, and where to keep what the engine cannot.
            "setRoots" => {
                client.set_auto_accept_root(std::path::PathBuf::from(arg!(args, "autoAcceptRoot")));
                client.set_preferences_path(std::path::PathBuf::from(arg!(args, "preferences")));
                Value::Null
            }

            other => return Err(format!("no such call: {other}")),
        };
        Ok(value)
    })
}

fn plain(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn to_value<T: serde::Serialize, E: std::fmt::Display>(
    result: std::result::Result<T, E>,
) -> Result<Value, String> {
    let value = result.map_err(plain)?;
    serde_json::to_value(value).map_err(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_call_is_refused_rather_than_ignored() {
        // Without a connected engine the dispatcher must still fail loudly.
        let err = dispatch("nonsense", Value::Null).unwrap_err();
        assert!(!err.is_empty());
    }
}
