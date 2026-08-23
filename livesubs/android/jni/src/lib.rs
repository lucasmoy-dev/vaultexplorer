//! JNI shim: the Android app's audio threads talking to `livesubs-core`.
//!
//! Kotlin owns the platform (capture, the overlay, translation,
//! notifications); this owns the decision of what a caption *is*. Keeping
//! the VAD and whisper in Rust means the phone and the desktop cut
//! utterances at the same thresholds and clean text the same way -- a bug
//! fixed in one is fixed in both, and there is no second implementation to
//! drift.
//!
//! Shape of the interface, deliberately tiny:
//!
//! * one **engine** (the loaded model), shared by both capture sources;
//! * one **stream** per source, holding that source's VAD state and its
//!   own whisper decoder state;
//! * `feed` takes one frame and returns either nothing or a finished
//!   caption as JSON.
//!
//! `feed` blocks while whisper runs, which is why Kotlin calls it from the
//! capture thread of that source: the two sources then transcribe
//! independently, and neither can stall the other.
//!
//! Handles cross the boundary as `jlong` pointers to leaked boxes. The
//! Kotlin side frees them in `Service.onDestroy`; leaking on an abrupt
//! process death costs nothing (the process is going away with them).

use jni::objects::{JClass, JFloatArray, JString};
use jni::sys::{jfloat, jint, jlong, jstring};
use jni::JNIEnv;
use livesubs_core::stt::{Engine, Session};
use livesubs_core::vad::{Vad, FRAME};
use std::sync::Arc;

/// One capture source: its VAD state, its decoder state, and a handle on
/// the model so the engine cannot be freed out from under it.
struct Stream {
    /// Never read: held so the model outlives every decoder state built
    /// from it. (`WhisperState` keeps its own `Arc` internally, but this
    /// makes the ownership visible instead of a fact about someone else's
    /// crate.)
    #[allow(dead_code)]
    engine: Arc<Engine>,
    session: Session,
    vad: Vad,
}

/// Turn a raw handle back into a reference. Unsafe by nature: the contract
/// is that Kotlin only ever passes back a handle this library returned and
/// has not yet freed, which is enforced by keeping both inside
/// `CaptionService`.
unsafe fn stream_ref(handle: jlong) -> Option<&'static mut Stream> {
    if handle == 0 {
        return None;
    }
    Some(&mut *(handle as *mut Stream))
}

unsafe fn engine_ref(handle: jlong) -> Option<&'static Arc<Engine>> {
    if handle == 0 {
        return None;
    }
    Some(&*(handle as *const Arc<Engine>))
}

/// Rust panics must not cross into the JVM (that is an abort, and the app
/// dies with a stack trace nobody can read). Every entry point below runs
/// its body through this, turning a panic into a thrown Java exception --
/// which Kotlin can log, show, and survive.
fn guard<T>(env: &mut JNIEnv, fallback: T, body: impl FnOnce(&mut JNIEnv) -> Result<T, String>) -> T {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(env)));
    match result {
        Ok(Ok(value)) => value,
        Ok(Err(message)) => {
            let _ = env.throw_new("java/lang/RuntimeException", message);
            fallback
        }
        Err(_) => {
            let _ = env.throw_new("java/lang/RuntimeException", "livesubs native panic");
            fallback
        }
    }
}

/// How many samples a `feed` call expects. Exported rather than duplicated
/// in Kotlin: the frame size is the VAD's business.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_frameSize(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    FRAME as jint
}

#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_loadModel(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
    model_name: JString,
) -> jlong {
    guard(&mut env, 0, |env| {
        let path: String = env.get_string(&path).map_err(|e| e.to_string())?.into();
        let name: String = env.get_string(&model_name).map_err(|e| e.to_string())?.into();
        let engine = Engine::load_file(std::path::Path::new(&path), &name)?;
        Ok(Box::into_raw(Box::new(Arc::new(engine))) as jlong)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_freeModel(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle came from `loadModel` and is freed once.
        unsafe { drop(Box::from_raw(handle as *mut Arc<Engine>)) };
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_createStream(
    mut env: JNIEnv,
    _class: JClass,
    engine_handle: jlong,
    sensitivity: jfloat,
) -> jlong {
    guard(&mut env, 0, |_env| {
        // SAFETY: the engine handle is alive for as long as any stream --
        // `CaptionService` frees streams before the engine.
        let engine = unsafe { engine_ref(engine_handle) }.ok_or("model not loaded")?.clone();
        let session = engine.session()?;
        let stream = Stream { engine, session, vad: Vad::new(sensitivity) };
        Ok(Box::into_raw(Box::new(stream)) as jlong)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_freeStream(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle came from `createStream` and is freed once.
        unsafe { drop(Box::from_raw(handle as *mut Stream)) };
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_setSensitivity(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    sensitivity: jfloat,
) {
    // SAFETY: see `stream_ref`.
    if let Some(stream) = unsafe { stream_ref(handle) } {
        stream.vad.set_sensitivity(sensitivity);
    }
}

/// Feed one frame of 16kHz mono audio. Returns `null` most of the time,
/// and a caption as JSON (`{"text":…,"language":…}`) when an utterance
/// just ended and whisper had something to say about it. Blocks for the
/// length of the transcription when that happens.
///
/// `language` is `null` for automatic detection.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_feed(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    samples: JFloatArray,
    language: JString,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let length = env.get_array_length(&samples).map_err(|e| e.to_string())? as usize;
        let mut frame = vec![0f32; length];
        env.get_float_array_region(&samples, 0, &mut frame).map_err(|e| e.to_string())?;
        let language = optional_string(env, language)?;

        // SAFETY: see `stream_ref`.
        let stream = unsafe { stream_ref(handle) }.ok_or("stream is closed")?;
        let Some(utterance) = stream.vad.push(&frame) else {
            return Ok(std::ptr::null_mut());
        };
        transcribe_to_json(env, stream, &utterance, language.as_deref())
    })
}

/// Transcribe whatever is still buffered -- called when a source stops, so
/// the last half-sentence isn't lost.
#[no_mangle]
pub extern "system" fn Java_dev_lucasmoy_livesubs_NativeEngine_flush(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    language: JString,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let language = optional_string(env, language)?;
        // SAFETY: see `stream_ref`.
        let stream = unsafe { stream_ref(handle) }.ok_or("stream is closed")?;
        let Some(utterance) = stream.vad.flush() else {
            return Ok(std::ptr::null_mut());
        };
        transcribe_to_json(env, stream, &utterance, language.as_deref())
    })
}

fn transcribe_to_json(
    env: &mut JNIEnv,
    stream: &mut Stream,
    utterance: &[f32],
    language: Option<&str>,
) -> Result<jstring, String> {
    let recognition = stream.session.transcribe(utterance, language)?;
    if recognition.text.is_empty() {
        // Silence, music, a door closing: the core already decided this
        // isn't a caption, and an empty one on screen is worse than none.
        return Ok(std::ptr::null_mut());
    }
    let json = serde_json::json!({
        "text": recognition.text,
        "language": recognition.language,
    })
    .to_string();
    Ok(env.new_string(json).map_err(|e| e.to_string())?.into_raw())
}

/// JNI hands `null` strings as a null object rather than an `Option`.
fn optional_string(env: &mut JNIEnv, value: JString) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let text: String = env.get_string(&value).map_err(|e| e.to_string())?.into();
    Ok(if text.is_empty() { None } else { Some(text) })
}
