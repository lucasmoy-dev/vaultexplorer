#[cfg(target_os = "android")]
use crate::errmap::ToStringErr;
#[cfg(target_os = "android")]
use tauri::Manager;

/// "All files access" (`MANAGE_EXTERNAL_STORAGE`) is what actually unlocks
/// raw path listing of shared storage (Download/Pictures/DCIM) under
/// Android's scoped storage -- there's no Rust API for it, so this reaches
/// straight into the plain Android SDK (`Environment.isExternalStorageManager`)
/// over JNI via the webview's own JNI handle.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_storage_access_granted(app: tauri::AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, _activity, _webview| {
            let granted = (|| -> Result<bool, jni::errors::Error> {
                let env_class = env.find_class("android/os/Environment")?;
                env.call_static_method(env_class, "isExternalStorageManager", "()Z", &[])?
                    .z()
            })();
            if granted.is_err() {
                let _ = env.exception_clear();
            }
            let _ = tx.send(granted.unwrap_or(false));
        });
    });
    if sent.is_err() {
        return false;
    }
    rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap_or(false)
}

/// `MANAGE_EXTERNAL_STORAGE` can't be requested via a normal runtime
/// permission dialog -- the only way to grant it is a dedicated system
/// settings screen the user has to tap through themselves.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_request_storage_access(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window
        .with_webview(move |pw| {
            let jni = pw.jni_handle();
            jni.exec(move |env, activity, _webview| {
                let mut run = || -> Result<(), jni::errors::Error> {
                    use jni::objects::{JObject, JValue};
                    let action =
                        env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
                    let intent_class = env.find_class("android/content/Intent")?;
                    let intent = env.new_object(
                        intent_class,
                        "(Ljava/lang/String;)V",
                        &[JValue::Object(&action)],
                    )?;

                    let pkg_name = env
                        .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
                        .l()?;
                    let scheme = env.new_string("package")?;
                    let uri_class = env.find_class("android/net/Uri")?;
                    let none = JObject::null();
                    let uri = env
                        .call_static_method(
                            uri_class,
                            "fromParts",
                            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;",
                            &[
                                JValue::Object(&scheme),
                                JValue::Object(&pkg_name),
                                JValue::Object(&none),
                            ],
                        )?
                        .l()?;
                    env.call_method(
                        &intent,
                        "setData",
                        "(Landroid/net/Uri;)Landroid/content/Intent;",
                        &[JValue::Object(&uri)],
                    )?;
                    env.call_method(
                        activity,
                        "startActivity",
                        "(Landroid/content/Intent;)V",
                        &[JValue::Object(&intent)],
                    )?;
                    Ok(())
                };
                if run().is_err() {
                    let _ = env.exception_clear();
                }
            });
        })
        .str_err()
}
