//! The JNI surface: four calls, all blocking, all returning strings.
//!
//! Kotlin does the parts the platform is good at -- UI, HTTP downloads with
//! progress, `MediaMuxer`, `MediaStore` -- and comes here for the parts it
//! cannot do: talking to YouTube like a client, naming a file safely, and
//! encoding MP3.
//!
//! Everything returns JSON (or a plain string) rather than Java objects
//! built through JNI: constructing a class per shape is a lot of unsafe
//! ceremony for data that Kotlin is going to parse into its own types
//! anyway, and a JSON string is trivially loggable when something breaks.
//!
//! Blocking on purpose, from a background thread on the Kotlin side: a
//! search is one network round trip, and a transcode is CPU-bound work
//! nobody can usefully interleave.

use crate::{mp3, naming, youtube};
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

/// Turn a `Result` into either the value or a `{"error": "..."}` JSON
/// document, so the Kotlin side has exactly one shape to check for and a
/// failure is never an empty result that looks like "nothing found".
fn respond(env: &mut JNIEnv, result: Result<String, String>) -> jstring {
    let payload = match result {
        Ok(json) => json,
        Err(message) => serde_json::json!({ "error": message }).to_string(),
    };
    match env.new_string(payload) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// A Rust panic must not cross into the JVM (that is an abort, and the app
/// dies with a trace nobody can read). Panics become the same error shape
/// as anything else.
fn guarded(env: &mut JNIEnv, body: impl FnOnce() -> Result<String, String>) -> jstring {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .unwrap_or_else(|_| Err("fallo interno en la capa nativa".to_string()));
    respond(env, outcome)
}

fn text(env: &mut JNIEnv, value: &JString) -> Result<String, String> {
    env.get_string(value).map(|s| s.into()).map_err(|e| e.to_string())
}

/// Per-client report: what resolved, what would actually download. Exists so
/// a failure on a phone the author cannot reach becomes a fact instead of a
/// guess.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_diagnose(
    mut env: JNIEnv,
    _class: JClass,
    video: JString,
) -> jstring {
    let video = match text(&mut env, &video) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    guarded(&mut env, || Ok(youtube::diagnose(&video)))
}

/// Tell the native side where it may write.
///
/// Not optional: rustypipe's cache defaults to the process's working
/// directory, which on Android is `/` and read-only. Without this every
/// search re-downloads and re-parses YouTube's player JS.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_initCache(
    mut env: JNIEnv,
    _class: JClass,
    dir: JString,
) {
    if let Ok(path) = text(&mut env, &dir) {
        youtube::set_storage_dir(&path);
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_search(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
    limit: jni::sys::jint,
) -> jstring {
    let query = match text(&mut env, &query) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    guarded(&mut env, || {
        let hits = youtube::search(&query, limit.max(1) as usize)?;
        serde_json::to_string(&hits).map_err(|e| e.to_string())
    })
}

/// The next page of the search already running. Empty when YouTube has no
/// more to give, which is how the list knows to stop asking.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_searchMore(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
    limit: jni::sys::jint,
) -> jstring {
    let query = match text(&mut env, &query) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    guarded(&mut env, || {
        let hits = youtube::search_more(&query, limit.max(1) as usize)?;
        serde_json::to_string(&hits).map_err(|e| e.to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_resolve(
    mut env: JNIEnv,
    _class: JClass,
    video: JString,
) -> jstring {
    let video = match text(&mut env, &video) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    guarded(&mut env, || {
        let resolved = youtube::resolve(&video)?;
        serde_json::to_string(&resolved).map_err(|e| e.to_string())
    })
}

/// The size of a stream, for the progress bar, or `{"error":...}`.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_totalSize(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    user_agent: JString,
) -> jstring {
    let url = match text(&mut env, &url) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    let user_agent = text(&mut env, &user_agent).unwrap_or_default();
    guarded(&mut env, || {
        let total = crate::download::total_size(&url, &user_agent)?;
        Ok(serde_json::json!({ "total": total }).to_string())
    })
}

/// Append one chunk of `url` to `path`. Returns `{"written": n}`; `0` means
/// the file ended. Kotlin loops, keeping progress and cancellation on its
/// side -- see `download.rs` for why the HTTP itself is here.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_downloadChunk(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    path: JString,
    offset: jni::sys::jlong,
    max_bytes: jni::sys::jlong,
    user_agent: JString,
) -> jstring {
    let url = match text(&mut env, &url) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    let path = match text(&mut env, &path) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    let user_agent = text(&mut env, &user_agent).unwrap_or_default();
    guarded(&mut env, || {
        let written = crate::download::chunk(
            &url,
            &path,
            offset.max(0) as u64,
            max_bytes.max(1) as u64,
            &user_agent,
        )?;
        Ok(serde_json::json!({ "written": written }).to_string())
    })
}

/// A filename for this title, safe for the phone's storage. In Rust rather
/// than Kotlin because it is the app's promise -- "files named after the
/// video" -- and it has the tests to prove it (see `naming.rs`).
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_fileName(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    ext: JString,
) -> jstring {
    let title = text(&mut env, &title).unwrap_or_default();
    let ext = text(&mut env, &ext).unwrap_or_default();
    let name = naming::file_name(&title, &ext);
    match env.new_string(name) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Transcode a downloaded `.m4a` to MP3, tagged with the video's title and
/// channel. Returns `{"ok":true}` or `{"error":...}`.
///
/// No progress callback: a JNI callback per frame would cost more than it
/// tells anyone, and the Kotlin side shows this step as one line of the
/// download notification ("convirtiendo…") between two things that do have
/// real progress.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_ytpocket_Native_transcodeMp3(
    mut env: JNIEnv,
    _class: JClass,
    source: JString,
    destination: JString,
    title: JString,
    artist: JString,
) -> jstring {
    let source = match text(&mut env, &source) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    let destination = match text(&mut env, &destination) {
        Ok(value) => value,
        Err(error) => return respond(&mut env, Err(error)),
    };
    let title = text(&mut env, &title).unwrap_or_default();
    let artist = text(&mut env, &artist).unwrap_or_default();
    guarded(&mut env, || {
        mp3::transcode(
            std::path::Path::new(&source),
            std::path::Path::new(&destination),
            Some(title.as_str()).filter(|t| !t.is_empty()),
            Some(artist.as_str()).filter(|a| !a.is_empty()),
            &mut |_frames| true,
        )?;
        Ok(serde_json::json!({ "ok": true }).to_string())
    })
}
