//! Android-only: download an APK and fire the system package installer via
//! JNI (FileProvider + ACTION_VIEW). Adapted from the equivalent flow in the
//! sibling `vaultexplorer` app; see its android.rs for the long-form notes on
//! the classloader and exception-message pitfalls handled here.
#![cfg(target_os = "android")]

use tauri::Manager;

/// `.str_err()` -> map any Display error to String (command return type).
trait ToStringErr<T> {
    fn str_err(self) -> Result<T, String>;
}
impl<T, E: std::fmt::Display> ToStringErr<T> for Result<T, E> {
    fn str_err(self) -> Result<T, String> {
        self.map_err(|e| e.to_string())
    }
}

/// Load a class through the Activity's own classloader (which can see the
/// app's dex, unlike the boot classloader a JNI callback thread defaults to).
/// `binary_name` is dotted form, e.g. "androidx.core.content.FileProvider".
fn find_app_class<'a>(
    env: &mut jni::JNIEnv<'a>,
    activity: &jni::objects::JObject,
    binary_name: &str,
) -> Result<jni::objects::JClass<'a>, jni::errors::Error> {
    use jni::objects::JValue;
    let activity_class = env.call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?.l()?;
    let class_loader =
        env.call_method(&activity_class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
    let name = env.new_string(binary_name)?;
    let class_obj = env
        .call_method(
            &class_loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    Ok(jni::objects::JClass::from(class_obj))
}

/// Pull the real message out of a pending Java exception (the jni crate's
/// Display is generic), clearing it as required before further JNI use.
fn describe_jni_error(env: &mut jni::JNIEnv, err: jni::errors::Error) -> String {
    if !matches!(err, jni::errors::Error::JavaException) {
        return err.to_string();
    }
    let described = (|| -> Result<String, jni::errors::Error> {
        let ex = env.exception_occurred()?;
        env.exception_clear()?;
        let msg = env.call_method(&ex, "toString", "()Ljava/lang/String;", &[])?.l()?;
        let jstr = jni::objects::JString::from(msg);
        env.get_string(&jstr).map(|s| s.into())
    })();
    described.unwrap_or_else(|_| {
        let _ = env.exception_clear();
        err.to_string()
    })
}

/// Whether the user has granted "install unknown apps" to this app.
fn can_install_packages(app: &tauri::AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, activity, _webview| {
            let granted = (|| -> Result<bool, jni::errors::Error> {
                let pm = env
                    .call_method(activity, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?
                    .l()?;
                env.call_method(&pm, "canRequestPackageInstalls", "()Z", &[])?.z()
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

/// Open the system "install unknown apps" settings screen for this app.
fn request_install_permission(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window
        .with_webview(move |pw| {
            let jni = pw.jni_handle();
            jni.exec(move |env, activity, _webview| {
                let mut run = || -> Result<(), jni::errors::Error> {
                    use jni::objects::{JObject, JValue};
                    let action = env.new_string("android.settings.MANAGE_UNKNOWN_APP_SOURCES")?;
                    let intent_class = env.find_class("android/content/Intent")?;
                    let intent =
                        env.new_object(intent_class, "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
                    let pkg_name =
                        env.call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
                    let scheme = env.new_string("package")?;
                    let uri_class = env.find_class("android/net/Uri")?;
                    let none = JObject::null();
                    let uri = env
                        .call_static_method(
                            uri_class,
                            "fromParts",
                            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;",
                            &[JValue::Object(&scheme), JValue::Object(&pkg_name), JValue::Object(&none)],
                        )?
                        .l()?;
                    env.call_method(&intent, "setData", "(Landroid/net/Uri;)Landroid/content/Intent;", &[JValue::Object(&uri)])?;
                    env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
                    Ok(())
                };
                if let Err(e) = run() {
                    let _ = describe_jni_error(env, e);
                }
            });
        })
        .str_err()
}

/// Download the APK at `url` and launch the system installer for it.
pub fn install_apk(app: tauri::AppHandle, url: String) -> Result<(), String> {
    if !can_install_packages(&app) {
        let _ = request_install_permission(&app);
        return Err("Activá \"Instalar apps desconocidas\" para Life Framework y volvé a tocar el botón.".into());
    }

    // HTTP/1.1 only: HTTP/2 stream handling has a history of truncating large
    // downloads on Android's network stack. Generous timeout for slow links.
    let client = reqwest::blocking::Client::builder()
        .http1_only()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .str_err()?;
    let bytes = client.get(&url).send().str_err()?.error_for_status().str_err()?.bytes().str_err()?;

    let cache_dir = app.path().app_cache_dir().str_err()?;
    std::fs::create_dir_all(&cache_dir).str_err()?;
    let apk_path = cache_dir.join("life-framework-update.apk");
    std::fs::write(&apk_path, &bytes).str_err()?;
    let apk_path_str = apk_path.to_string_lossy().to_string();

    let window = app.get_webview_window("main").ok_or("no main window")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, activity, _webview| {
            let mut run = || -> Result<(), jni::errors::Error> {
                use jni::objects::JValue;
                let pkg_name = env.call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
                let authority_suffix = env.new_string(".fileprovider")?;
                let authority = env
                    .call_method(
                        &pkg_name,
                        "concat",
                        "(Ljava/lang/String;)Ljava/lang/String;",
                        &[JValue::Object(&authority_suffix)],
                    )?
                    .l()?;
                let file_provider_class = find_app_class(env, activity, "androidx.core.content.FileProvider")?;
                let file_class = env.find_class("java/io/File")?;
                let path_str = env.new_string(&apk_path_str)?;
                let file = env.new_object(file_class, "(Ljava/lang/String;)V", &[JValue::Object(&path_str)])?;
                let uri = env
                    .call_static_method(
                        &file_provider_class,
                        "getUriForFile",
                        "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
                        &[JValue::Object(activity), JValue::Object(&authority), JValue::Object(&file)],
                    )?
                    .l()?;

                let action = env.new_string("android.intent.action.VIEW")?;
                let intent_class = env.find_class("android/content/Intent")?;
                let intent = env.new_object(intent_class, "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
                let mime = env.new_string("application/vnd.android.package-archive")?;
                env.call_method(
                    &intent,
                    "setDataAndType",
                    "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&uri), JValue::Object(&mime)],
                )?;
                // FLAG_GRANT_READ_URI_PERMISSION (1) | FLAG_ACTIVITY_NEW_TASK (0x10000000)
                env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(1 | 0x10000000)])?;
                env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
                Ok(())
            };
            let result = run().map_err(|e| describe_jni_error(env, e));
            let _ = tx.send(result);
        });
    });
    if sent.is_err() {
        return Err("no pude alcanzar el webview".into());
    }
    rx.recv_timeout(std::time::Duration::from_secs(15))
        .map_err(|_| "tiempo agotado".to_string())?
}
