#[cfg(target_os = "android")]
use crate::errmap::ToStringErr;
#[cfg(target_os = "android")]
use tauri::Manager;

/// `env.find_class` resolves against whatever classloader the *current
/// thread* is implicitly attached under. That's fine for framework
/// classes (always on the boot classpath, visible from any classloader),
/// but every one of this file's JNI callbacks runs on a thread Tauri
/// attaches to the JVM itself -- not one the app's own code created --
/// so its implicit classloader is the plain boot one, which can't see
/// classes bundled in the app's own dex (any `androidx.*` class,
/// `FileProvider` among them). `find_class`ing one of those throws
/// `NoClassDefFoundError: Class not found using the boot class loader;
/// no stack trace available` -- confirmed live (not a hypothetical) via
/// this exact call inside `android_open_path`, which is what prompted
/// pulling this out and fixing it in the two older call sites too.
/// Loading through the Activity's own classloader instead -- the one
/// that actually has the app's dex in its search path -- is the
/// standard fix for this well-known JNI pitfall. `binary_name` is the
/// dotted form (`android.foo.Bar`), not the slash form `find_class`
/// wants.
#[cfg(target_os = "android")]
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
        .call_method(&class_loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
        .l()?;
    Ok(jni::objects::JClass::from(class_obj))
}

/// `jni::errors::Error::JavaException` is a unit variant -- the crate
/// deliberately doesn't auto-extract the pending exception's message, so
/// its `Display` always prints the same generic "Java exception was
/// thrown" regardless of what actually failed (SecurityException, IO
/// error, wrong FileProvider authority...). Every JNI command in this
/// file used to just `exception_clear()` a failing exception and stringify
/// the outer `Error` -- surfacing that generic text with the real cause
/// discarded. This pulls the exception's own message out first via
/// `Throwable.toString()` (before clearing it, which is required before
/// the JNIEnv can be used for anything else), so the JS side gets an
/// actual "java.lang.SecurityException: ..." instead of a dead end.
#[cfg(target_os = "android")]
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

/// Installing an APK via `ACTION_VIEW` (see `android_download_and_install_apk`
/// below) needs "install unknown apps" enabled for this app first
/// (`PackageManager.canRequestPackageInstalls`, API 26+) -- without it,
/// tapping "Update" would previously just download the APK and start an
/// intent that quietly went nowhere (no exception, no visible prompt),
/// which looked exactly like the button doing nothing at all. Checked the
/// same way `android_storage_access_granted` checks its own permission.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_can_install_packages(app: tauri::AppHandle) -> bool {
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

/// Same "dedicated settings screen, no in-app dialog" shape as
/// `android_request_storage_access` -- `ACTION_MANAGE_UNKNOWN_APP_SOURCES`
/// is the "install unknown apps" equivalent of
/// `MANAGE_APP_ALL_FILES_ACCESS_PERMISSION`.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_request_install_packages_access(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window
        .with_webview(move |pw| {
            let jni = pw.jni_handle();
            jni.exec(move |env, activity, _webview| {
                let mut run = || -> Result<(), jni::errors::Error> {
                    use jni::objects::{JObject, JValue};
                    let action = env.new_string("android.settings.MANAGE_UNKNOWN_APP_SOURCES")?;
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
                if let Err(e) = run() {
                    let _ = describe_jni_error(env, e);
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
            let result = run().map_err(|e| describe_jni_error(env, e));
            let _ = tx.send(result);
        });
    });
    if sent.is_err() {
        return Err("failed to reach the webview".into());
    }
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "timed out".to_string())?
}

/// A small extension->MIME table, plain Rust rather than a JNI call into
/// `android.webkit.MimeTypeMap` -- that class lives outside the boot
/// classpath (unbundled WebView), and `find_class`ing it from the native
/// thread `jni_handle().exec()` runs on throws `NoClassDefFoundError:
/// Class not found using the boot class loader` (confirmed live, not a
/// hypothetical: this was the first implementation, replaced after
/// hitting exactly that on-device). Covers the kinds this app already
/// distinguishes elsewhere (see `icons.tsx`'s `kindOf`); anything else
/// falls back to `*/*`, which still gets the system's normal app-chooser
/// rather than failing outright.
#[cfg(target_os = "android")]
fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" | "heif" => "image/heic",
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "3gp" => "video/3gpp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "apk" => "application/vnd.android.package-archive",
        _ => "*/*",
    }
}

/// Opens an arbitrary file with whatever app the system has registered for
/// its type, via the same FileProvider + `ACTION_VIEW` handoff
/// `android_import_contacts` above uses for a `.vcf` -- this is what
/// double-clicking a file that isn't a recognized in-app preview type
/// (an image, video, PDF...) resolves to.
///
/// This exists as a local replacement for `tauri-plugin-opener`'s
/// `openPath`, not a wrapper around it: that plugin's Android side has a
/// bug where its Rust `open_path` sends the mobile plugin a bare JSON
/// string instead of the `{url: ...}` object its own Kotlin `OpenArgs`
/// requires to deserialize (`no String-argument constructor... to
/// deserialize from String value`, thrown for every single file open on
/// Android) -- `open_url` on the same crate version does wrap its payload
/// correctly, so this is specifically a `open_path` regression, not
/// something wrong on this app's end. Filed upstream; until it's fixed,
/// opening a real file needs this instead.
///
/// The MIME type comes from `mime_for_ext` above, not Android's own
/// `MimeTypeMap` -- see that function's comment for why.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_open_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
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
                let path_str = env.new_string(&path)?;
                let file = env.new_object(file_class, "(Ljava/lang/String;)V", &[JValue::Object(&path_str)])?;
                let uri = env
                    .call_static_method(
                        &file_provider_class,
                        "getUriForFile",
                        "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
                        &[JValue::Object(activity), JValue::Object(&authority), JValue::Object(&file)],
                    )?
                    .l()?;

                let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
                let mime = env.new_string(mime_for_ext(&ext))?;

                let action = env.new_string("android.intent.action.VIEW")?;
                let intent_class = env.find_class("android/content/Intent")?;
                let intent = env.new_object(intent_class, "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
                env.call_method(
                    &intent,
                    "setDataAndType",
                    "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&uri), JValue::Object(&mime)],
                )?;
                // FLAG_GRANT_READ_URI_PERMISSION(1) | FLAG_ACTIVITY_NEW_TASK(0x10000000)
                env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(1 | 0x1000_0000)])?;
                env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
                Ok(())
            };
            let result = run().map_err(|e| describe_jni_error(env, e));
            let _ = tx.send(result);
        });
    });
    if sent.is_err() {
        return Err("failed to reach the webview".into());
    }
    rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "timed out".to_string())?
}

/// The media viewer's "Share" button (send a photo/video/audio file to
/// WhatsApp/etc) -- distinct from the existing desktop-oriented
/// `fs_share_file`/`vault_share_file` (which upload to an anonymous public
/// file host and copy back a link; not what "share" means for a phone).
/// Same FileProvider URI-granting pattern as `android_open_path`, just
/// wrapped in `ACTION_SEND` + `Intent.createChooser` instead of
/// `ACTION_VIEW`, so it opens the OS share sheet instead of a single fixed
/// viewer app.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_share_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
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
                let path_str = env.new_string(&path)?;
                let file = env.new_object(file_class, "(Ljava/lang/String;)V", &[JValue::Object(&path_str)])?;
                let uri = env
                    .call_static_method(
                        &file_provider_class,
                        "getUriForFile",
                        "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
                        &[JValue::Object(activity), JValue::Object(&authority), JValue::Object(&file)],
                    )?
                    .l()?;

                let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
                let mime = env.new_string(mime_for_ext(&ext))?;

                let action = env.new_string("android.intent.action.SEND")?;
                let intent_class = env.find_class("android/content/Intent")?;
                let intent = env.new_object(&intent_class, "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
                env.call_method(&intent, "setType", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&mime)])?;
                let extra_stream = env.new_string("android.intent.extra.STREAM")?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;",
                    &[JValue::Object(&extra_stream), JValue::Object(&uri)],
                )?;
                // FLAG_GRANT_READ_URI_PERMISSION -- the receiving app (WhatsApp,
                // Gmail, ...) needs this to actually read the fileprovider URI,
                // same as the VIEW intents elsewhere in this file.
                env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(1)])?;

                let chooser_title = env.new_string("Share")?;
                let chooser = env
                    .call_static_method(
                        &intent_class,
                        "createChooser",
                        "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                        &[JValue::Object(&intent), JValue::Object(&chooser_title)],
                    )?
                    .l()?;
                env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&chooser)])?;
                Ok(())
            };
            let result = run().map_err(|e| describe_jni_error(env, e));
            let _ = tx.send(result);
        });
    });
    if sent.is_err() {
        return Err("failed to reach the webview".into());
    }
    rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "timed out".to_string())?
}

/// Downloads an APK (a GitHub release asset URL, see `checkForUpdate` on
/// the JS side) into this app's own cache dir, then hands it to the
/// system's package installer via `ACTION_VIEW` -- same FileProvider +
/// `ACTION_VIEW` handoff `android_import_contacts` above uses for a
/// `.vcf`, just with the APK MIME type instead. The install itself still
/// needs the user's confirmation in that system UI (and, the first time,
/// granting "install unknown apps" for this app as the source) -- there's
/// no way to skip that outside being an actual app-store client, and this
/// deliberately doesn't try to.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_download_and_install_apk(app: tauri::AppHandle, url: String) -> Result<(), String> {
    // A plain `reqwest::blocking::get` here reproducibly failed partway
    // through this ~25MB download with "error decoding response body" --
    // confirmed live, not hypothetical: `curl` against the exact same
    // release-asset URL (served over HTTP/2, redirected through
    // release-assets.githubusercontent.com) completed fine, so the bytes
    // themselves aren't the problem. HTTP/2 stream handling has a history
    // of being flakier than HTTP/1.1 on Android's network stack
    // specifically (this is a mobile-only code path; desktop's own
    // update flow just opens the releases page in a browser instead, see
    // `installUpdate` on the JS side) -- forcing HTTP/1.1 avoids that
    // class of issue, and a generous explicit timeout accounts for a
    // slow mobile connection rather than an implicit shorter default.
    let client = reqwest::blocking::Client::builder()
        .http1_only()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .str_err()?;
    let bytes = client.get(&url).send().str_err()?.error_for_status().str_err()?.bytes().str_err()?;
    let cache_dir = app.path().app_cache_dir().str_err()?;
    std::fs::create_dir_all(&cache_dir).str_err()?;
    let apk_path = cache_dir.join("vault-explorer-update.apk");
    std::fs::write(&apk_path, &bytes).str_err()?;
    let apk_path_str = apk_path.to_string_lossy().to_string();

    let window = app.get_webview_window("main").ok_or("no main window")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, activity, _webview| {
            let mut run = || -> Result<(), jni::errors::Error> {
                use jni::objects::JValue;
                let pkg_name = env
                    .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
                    .l()?;
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
                // FLAG_GRANT_READ_URI_PERMISSION (1) | FLAG_ACTIVITY_NEW_TASK
                // (0x10000000) -- the installer needs read access to a
                // FileProvider URI it didn't create, and NEW_TASK since
                // the installer is a distinct app PackageInstaller may
                // launch outside this activity's own task.
                env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(1 | 0x10000000)])?;
                env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
                Ok(())
            };
            // The old version of this just cleared a failing exception and
            // moved on -- `with_webview`'s closure return value is
            // discarded, so the outer command always resolved `Ok(())`
            // regardless of what happened here. That meant a real failure
            // (bad FileProvider authority, no activity for this intent...)
            // looked to the user exactly like nothing happening at all when
            // they tapped "Update". Piping the real result back through a
            // channel is what android_export_contacts already does.
            let result = run().map_err(|e| describe_jni_error(env, e));
            let _ = tx.send(result);
        });
    });
    if sent.is_err() {
        return Err("failed to reach the webview".into());
    }
    rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "timed out".to_string())?
}

/// Muxes a video-only file and an audio-only file into one playable mp4,
/// using Android's own `MediaMuxer`.
///
/// This exists because YouTube stopped serving progressive streams: a
/// video download arrives as two files that have to be joined. On desktop
/// yt-dlp does it with ffmpeg; on Android there is no ffmpeg, but the
/// platform ships a muxer for exactly this. Both tracks are already
/// H.264/AAC, so this is a container remux -- no decoding, no re-encoding,
/// and it runs in seconds rather than minutes.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) async fn android_mux_video(
    app: tauri::AppHandle,
    video_path: String,
    audio_path: String,
    out_path: String,
) -> Result<(), String> {
    use jni::objects::{JObject, JValue};
    let window = app.get_webview_window("main").ok_or("no main window")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, _activity, _webview| {
            let mut run = || -> Result<(), jni::errors::Error> {
                let out = env.new_string(&out_path)?;
                // 0 == MUXER_OUTPUT_MPEG_4
                let muxer = env.new_object(
                    "android/media/MediaMuxer",
                    "(Ljava/lang/String;I)V",
                    &[JValue::Object(&out), JValue::Int(0)],
                )?;

                // One extractor per input; each of these files has exactly
                // one track (they were downloaded as video-only and
                // audio-only), so track 0 is the one to carry over.
                let mut inputs: Vec<(JObject, i32)> = Vec::new();
                for path in [&video_path, &audio_path] {
                    if path.is_empty() {
                        continue;
                    }
                    let extractor = env.new_object("android/media/MediaExtractor", "()V", &[])?;
                    let jpath = env.new_string(path)?;
                    env.call_method(
                        &extractor,
                        "setDataSource",
                        "(Ljava/lang/String;)V",
                        &[JValue::Object(&jpath)],
                    )?;
                    let format = env
                        .call_method(&extractor, "getTrackFormat", "(I)Landroid/media/MediaFormat;", &[JValue::Int(0)])?
                        .l()?;
                    env.call_method(&extractor, "selectTrack", "(I)V", &[JValue::Int(0)])?;
                    let track = env
                        .call_method(
                            &muxer,
                            "addTrack",
                            "(Landroid/media/MediaFormat;)I",
                            &[JValue::Object(&format)],
                        )?
                        .i()?;
                    inputs.push((extractor, track));
                }

                env.call_method(&muxer, "start", "()V", &[])?;
                let buffer = env
                    .call_static_method(
                        "java/nio/ByteBuffer",
                        "allocate",
                        "(I)Ljava/nio/ByteBuffer;",
                        &[JValue::Int(1 << 20)],
                    )?
                    .l()?;
                let info = env.new_object("android/media/MediaCodec$BufferInfo", "()V", &[])?;

                for (extractor, track) in &inputs {
                    loop {
                        let size = env
                            .call_method(
                                extractor,
                                "readSampleData",
                                "(Ljava/nio/ByteBuffer;I)I",
                                &[JValue::Object(&buffer), JValue::Int(0)],
                            )?
                            .i()?;
                        if size < 0 {
                            break;
                        }
                        let time = env.call_method(extractor, "getSampleTime", "()J", &[])?.j()?;
                        let flags = env.call_method(extractor, "getSampleFlags", "()I", &[])?.i()?;
                        // BufferInfo's fields are public and there is no
                        // setter other than `set`, which also takes the
                        // flags -- so this is the whole call.
                        env.call_method(
                            &info,
                            "set",
                            "(IIJI)V",
                            &[JValue::Int(0), JValue::Int(size), JValue::Long(time), JValue::Int(flags)],
                        )?;
                        env.call_method(
                            &muxer,
                            "writeSampleData",
                            "(ILjava/nio/ByteBuffer;Landroid/media/MediaCodec$BufferInfo;)V",
                            &[JValue::Int(*track), JValue::Object(&buffer), JValue::Object(&info)],
                        )?;
                        env.call_method(extractor, "advance", "()Z", &[])?;
                    }
                    env.call_method(extractor, "release", "()V", &[])?;
                }

                env.call_method(&muxer, "stop", "()V", &[])?;
                env.call_method(&muxer, "release", "()V", &[])?;
                Ok(())
            };
            let _ = tx.send(run().map_err(|e| describe_jni_error(env, e)));
        });
    });
    if sent.is_err() {
        return Err("couldn't reach the Android webview".to_string());
    }
    rx.recv_timeout(std::time::Duration::from_secs(300))
        .map_err(|_| "muxing timed out".to_string())?
}
