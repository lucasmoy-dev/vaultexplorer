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

/// Pins a launcher icon on the home screen for one folder/vault, via
/// `ShortcutManager.requestPinShortcut` (API 26+; no Rust or Tauri-plugin
/// wrapper exists for this) -- tapping it fires an `ACTION_VIEW` intent at
/// the `vaultexplorer://` deep link `url` already registered in the
/// manifest, which routes straight to `Explorer`'s own deep-link handler
/// in App.tsx. `id` is caller-supplied (a short hash of `url`) so re-adding
/// the same folder updates the existing pin instead of piling up
/// duplicates -- Android scopes shortcut IDs per-package, not per-request.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_pin_folder_shortcut(
    app: tauri::AppHandle,
    id: String,
    label: String,
    url: String,
    icon_base64: Option<String>,
) -> Result<(), String> {
    use base64::Engine;
    // Decoded up front, in plain Rust -- no reason to make the JNI closure
    // (already doing enough) also carry the base64 crate's own error type.
    let icon_bytes = icon_base64.and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok());
    let window = app.get_webview_window("main").ok_or("no main window")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, activity, _webview| {
            let mut run = || -> Result<(), jni::errors::Error> {
                use jni::objects::{JObject, JValue};

                // API 25 has `ShortcutManager` but not `requestPinShortcut`
                // (API 26) -- below that there's no home-screen pin API at
                // all, so surface a clear error instead of a silent no-op.
                let sdk_int = env
                    .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
                    .i()?;
                if sdk_int < 26 {
                    return Err(jni::errors::Error::NullPtr("requestPinShortcut needs Android 8.0+"));
                }

                let shortcut_service = env.new_string("shortcut")?;
                let manager = env
                    .call_method(
                        activity,
                        "getSystemService",
                        "(Ljava/lang/String;)Ljava/lang/Object;",
                        &[JValue::Object(&shortcut_service)],
                    )?
                    .l()?;

                let supported = env
                    .call_method(&manager, "isRequestPinShortcutSupported", "()Z", &[])?
                    .z()?;
                if !supported {
                    return Err(jni::errors::Error::NullPtr("this launcher doesn't support pinned shortcuts"));
                }

                // A folder with a custom emoji icon gets that rendered-to-
                // PNG bitmap instead of the app's own icon (see
                // `renderEmojiIconPng` on the JS side); anything else --
                // no custom icon, or one that isn't a plain bitmap on the
                // JS side (a bundled WhiteSur SVG has no native filesystem
                // path for this to read) -- falls back to
                // `ApplicationInfo.icon`, the resource id
                // `Icon.createWithResource` wants, with no need to know
                // this package's resource names ahead of time.
                let icon_class = env.find_class("android/graphics/drawable/Icon")?;
                let icon = if let Some(bytes) = icon_bytes.as_deref() {
                    let byte_array = env.byte_array_from_slice(bytes)?;
                    let bitmap_factory = env.find_class("android/graphics/BitmapFactory")?;
                    let bitmap = env
                        .call_static_method(
                            bitmap_factory,
                            "decodeByteArray",
                            "([BII)Landroid/graphics/Bitmap;",
                            &[
                                JValue::Object(&byte_array),
                                JValue::Int(0),
                                JValue::Int(bytes.len() as i32),
                            ],
                        )?
                        .l()?;
                    env.call_static_method(
                        icon_class,
                        "createWithBitmap",
                        "(Landroid/graphics/Bitmap;)Landroid/graphics/drawable/Icon;",
                        &[JValue::Object(&bitmap)],
                    )?
                    .l()?
                } else {
                    let pkg_name = env
                        .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
                        .l()?;
                    let pm = env
                        .call_method(
                            activity,
                            "getPackageManager",
                            "()Landroid/content/pm/PackageManager;",
                            &[],
                        )?
                        .l()?;
                    let app_info = env
                        .call_method(
                            &pm,
                            "getApplicationInfo",
                            "(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;",
                            &[JValue::Object(&pkg_name), JValue::Int(0)],
                        )?
                        .l()?;
                    let icon_res_id = env.get_field(&app_info, "icon", "I")?.i()?;
                    env.call_static_method(
                        icon_class,
                        "createWithResource",
                        "(Landroid/content/Context;I)Landroid/graphics/drawable/Icon;",
                        &[JValue::Object(activity), JValue::Int(icon_res_id)],
                    )?
                    .l()?
                };

                let action = env.new_string("android.intent.action.VIEW")?;
                let intent_class = env.find_class("android/content/Intent")?;
                let intent = env.new_object(
                    intent_class,
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(&action)],
                )?;
                let uri_str = env.new_string(&url)?;
                let uri_class = env.find_class("android/net/Uri")?;
                let uri = env
                    .call_static_method(
                        uri_class,
                        "parse",
                        "(Ljava/lang/String;)Landroid/net/Uri;",
                        &[JValue::Object(&uri_str)],
                    )?
                    .l()?;
                env.call_method(
                    &intent,
                    "setData",
                    "(Landroid/net/Uri;)Landroid/content/Intent;",
                    &[JValue::Object(&uri)],
                )?;

                let shortcut_id = env.new_string(&id)?;
                let builder_class = env.find_class("android/content/pm/ShortcutInfo$Builder")?;
                let builder = env.new_object(
                    builder_class,
                    "(Landroid/content/Context;Ljava/lang/String;)V",
                    &[JValue::Object(activity), JValue::Object(&shortcut_id)],
                )?;
                let label_str = env.new_string(&label)?;
                env.call_method(
                    &builder,
                    "setShortLabel",
                    "(Ljava/lang/CharSequence;)Landroid/content/pm/ShortcutInfo$Builder;",
                    &[JValue::Object(&label_str)],
                )?;
                env.call_method(
                    &builder,
                    "setIcon",
                    "(Landroid/graphics/drawable/Icon;)Landroid/content/pm/ShortcutInfo$Builder;",
                    &[JValue::Object(&icon)],
                )?;
                env.call_method(
                    &builder,
                    "setIntent",
                    "(Landroid/content/Intent;)Landroid/content/pm/ShortcutInfo$Builder;",
                    &[JValue::Object(&intent)],
                )?;
                let shortcut_info = env
                    .call_method(&builder, "build", "()Landroid/content/pm/ShortcutInfo;", &[])?
                    .l()?;

                env.call_method(
                    &manager,
                    "requestPinShortcut",
                    "(Landroid/content/pm/ShortcutInfo;Landroid/content/IntentSender;)Z",
                    &[JValue::Object(&shortcut_info), JValue::Object(&JObject::null())],
                )?;
                Ok(())
            };
            let result = run().map_err(|e| e.to_string());
            if result.is_err() {
                let _ = env.exception_clear();
            }
            let _ = tx.send(result);
        });
    });
    if sent.is_err() {
        return Err("failed to reach the webview".into());
    }
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "timed out".to_string())?
}
