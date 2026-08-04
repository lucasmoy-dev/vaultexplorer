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

/// `READ_CONTACTS`/`WRITE_CONTACTS` are plain "dangerous" runtime
/// permissions (unlike `MANAGE_EXTERNAL_STORAGE` above, which needs a
/// dedicated settings screen) -- `Activity.checkSelfPermission` is enough
/// to check both without any AndroidX/ContextCompat dependency.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_contacts_permission_granted(app: tauri::AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, activity, _webview| {
            let granted = (|| -> Result<bool, jni::errors::Error> {
                use jni::objects::JValue;
                let perm = env.new_string("android.permission.READ_CONTACTS")?;
                let result = env
                    .call_method(
                        activity,
                        "checkSelfPermission",
                        "(Ljava/lang/String;)I",
                        &[JValue::Object(&perm)],
                    )?
                    .i()?;
                Ok(result == 0)
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

/// Fire-and-forget, same pattern as `android_request_storage_access`: this
/// just pops the OS permission dialog and returns immediately, with no
/// callback wired up for the user's answer (that needs a custom
/// `onRequestPermissionsResult` override, i.e. hand-written Kotlin, for a
/// one-tap flow this doesn't need) -- the caller re-checks
/// `android_contacts_permission_granted` and retries whatever it was doing
/// once the dialog closes, same as the storage-access flow already does.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_request_contacts_permission(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window
        .with_webview(move |pw| {
            let jni = pw.jni_handle();
            jni.exec(move |env, activity, _webview| {
                let mut run = || -> Result<(), jni::errors::Error> {
                    use jni::objects::{JObject, JValue};
                    let string_class = env.find_class("java/lang/String")?;
                    let perms = env.new_object_array(2, &string_class, JObject::null())?;
                    let read_perm = env.new_string("android.permission.READ_CONTACTS")?;
                    let write_perm = env.new_string("android.permission.WRITE_CONTACTS")?;
                    env.set_object_array_element(&perms, 0, &read_perm)?;
                    env.set_object_array_element(&perms, 1, &write_perm)?;
                    env.call_method(
                        activity,
                        "requestPermissions",
                        "([Ljava/lang/String;I)V",
                        &[JValue::Object(&perms), JValue::Int(1001)],
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

/// One `.vcf` per contact, named after it -- using
/// `content://com.android.contacts/contacts/as_vcard/<id>` (a real,
/// already-formatted vCard stream Android hands back for any contact ID)
/// rather than reading every `ContactsContract.Data` row and building one
/// by hand: less JNI, and it's the same vCard the Contacts app's own
/// "Share" would produce.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_export_contacts(app: tauri::AppHandle, dest_dir: String) -> Result<usize, String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| {
        let jni = pw.jni_handle();
        jni.exec(move |env, activity, _webview| {
            let mut run = || -> Result<usize, jni::errors::Error> {
                use jni::objects::JValue;

                let resolver = env
                    .call_method(
                        activity,
                        "getContentResolver",
                        "()Landroid/content/ContentResolver;",
                        &[],
                    )?
                    .l()?;
                let contacts_class = env.find_class("android/provider/ContactsContract$Contacts")?;
                let content_uri = env
                    .get_static_field(&contacts_class, "CONTENT_URI", "Landroid/net/Uri;")?
                    .l()?;
                let id_col = env.new_string("_id")?;
                let name_col = env.new_string("display_name")?;
                let projection = {
                    let string_class = env.find_class("java/lang/String")?;
                    let arr = env.new_object_array(2, &string_class, jni::objects::JObject::null())?;
                    env.set_object_array_element(&arr, 0, &id_col)?;
                    env.set_object_array_element(&arr, 1, &name_col)?;
                    arr
                };
                let none = jni::objects::JObject::null();
                let cursor = env
                    .call_method(
                        &resolver,
                        "query",
                        "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
                        &[
                            JValue::Object(&content_uri),
                            JValue::Object(&projection),
                            JValue::Object(&none),
                            JValue::Object(&none),
                            JValue::Object(&none),
                        ],
                    )?
                    .l()?;
                if cursor.is_null() {
                    return Ok(0);
                }
                let id_idx = env
                    .call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&id_col)])?
                    .i()?;
                let name_idx = env
                    .call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&name_col)])?
                    .i()?;
                std::fs::create_dir_all(&dest_dir).ok();
                let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut count = 0usize;
                loop {
                    let has_next = env.call_method(&cursor, "moveToNext", "()Z", &[])?.z()?;
                    if !has_next {
                        break;
                    }
                    let id = env.call_method(&cursor, "getLong", "(I)J", &[JValue::Int(id_idx)])?.j()?;
                    let name_obj = env
                        .call_method(&cursor, "getString", "(I)Ljava/lang/String;", &[JValue::Int(name_idx)])?
                        .l()?;
                    let raw_name = if name_obj.is_null() {
                        format!("contact-{id}")
                    } else {
                        let jstr = jni::objects::JString::from(name_obj);
                        env.get_string(&jstr).map(|s| s.into()).unwrap_or_else(|_| format!("contact-{id}"))
                    };
                    let safe_name: String = raw_name
                        .chars()
                        .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
                        .collect();
                    let mut file_name = format!("{safe_name}.vcf");
                    let mut n = 1;
                    while used_names.contains(&file_name) {
                        n += 1;
                        file_name = format!("{safe_name} {n}.vcf");
                    }
                    used_names.insert(file_name.clone());

                    let vcard_path = format!("content://com.android.contacts/contacts/as_vcard/{id}");
                    let vcard_str = env.new_string(&vcard_path)?;
                    let uri_class = env.find_class("android/net/Uri")?;
                    let vcard_uri = env
                        .call_static_method(uri_class, "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&vcard_str)])?
                        .l()?;
                    let stream = env
                        .call_method(
                            &resolver,
                            "openInputStream",
                            "(Landroid/net/Uri;)Ljava/io/InputStream;",
                            &[JValue::Object(&vcard_uri)],
                        )?
                        .l()?;
                    if stream.is_null() {
                        continue;
                    }
                    let buf = env.new_byte_array(4096)?;
                    let mut out: Vec<u8> = Vec::new();
                    loop {
                        let read = env
                            .call_method(&stream, "read", "([B)I", &[JValue::Object(&buf)])?
                            .i()?;
                        if read <= 0 {
                            break;
                        }
                        let mut chunk = vec![0i8; read as usize];
                        env.get_byte_array_region(&buf, 0, &mut chunk)?;
                        out.extend(chunk.iter().map(|b| *b as u8));
                    }
                    let _ = env.call_method(&stream, "close", "()V", &[]);
                    let full_path = std::path::Path::new(&dest_dir).join(&file_name);
                    if std::fs::write(&full_path, &out).is_ok() {
                        count += 1;
                    }
                }
                let _ = env.call_method(&cursor, "close", "()V", &[]);
                Ok(count)
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
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|_| "timed out".to_string())?
}

/// Hands each `.vcf` off to the system Contacts app's own import flow
/// (`ACTION_VIEW` with a `text/x-vcard` MIME type) rather than writing
/// `ContactsContract.RawContacts`/`Data` rows directly -- that has a lot of
/// edge cases (account type, structured-name components, per-field MIME
/// rows) the Contacts app already gets right, and this needs no more than
/// the read permission already granted for export (the actual write is the
/// system app's, under its own permission). Fires one intent per file and
/// returns immediately; each shows its own "Save contact?" prompt, same as
/// tapping a `.vcf` from any other app would.
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) fn android_import_contacts(app: tauri::AppHandle, vcf_paths: Vec<String>) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    window
        .with_webview(move |pw| {
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
                    let file_provider_class = env.find_class("androidx/core/content/FileProvider")?;

                    for path in &vcf_paths {
                        let file_class = env.find_class("java/io/File")?;
                        let path_str = env.new_string(path)?;
                        let file =
                            env.new_object(file_class, "(Ljava/lang/String;)V", &[JValue::Object(&path_str)])?;
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
                        let mime = env.new_string("text/x-vcard")?;
                        env.call_method(
                            &intent,
                            "setDataAndType",
                            "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
                            &[JValue::Object(&uri), JValue::Object(&mime)],
                        )?;
                        // FLAG_GRANT_READ_URI_PERMISSION = 1 -- the Contacts app
                        // needs this to read a FileProvider content:// URI it
                        // didn't create itself.
                        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(1)])?;
                        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
                    }
                    Ok(())
                };
                if run().is_err() {
                    let _ = env.exception_clear();
                }
            });
        })
        .str_err()
}
