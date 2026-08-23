package dev.lucasmoy.livesubs

import org.json.JSONObject

/**
 * The Rust core, over JNI (see `android/jni/src/lib.rs`).
 *
 * The interesting part of speech recognition -- where an utterance starts
 * and ends, what whisper is asked for, what counts as a caption at all --
 * is shared with the desktop app rather than written twice. This class is
 * only the boundary.
 *
 * Handles are raw pointers. Whoever calls [loadModel] or [createStream]
 * owns the result and must free it; [CaptionService] is the only caller,
 * and it frees streams before the model.
 */
object NativeEngine {
    init {
        System.loadLibrary("livesubs")
    }

    /** Samples per [feed] call -- decided by the VAD, not by Kotlin. */
    external fun frameSize(): Int

    external fun loadModel(path: String, modelName: String): Long
    external fun freeModel(handle: Long)

    external fun createStream(engine: Long, sensitivity: Float): Long
    external fun freeStream(handle: Long)
    external fun setSensitivity(handle: Long, sensitivity: Float)

    /**
     * Feed one frame of 16kHz mono audio. Returns null while an utterance
     * is still in progress (the common case), and a finished caption when
     * one just ended -- which is also when this call is slow, because that
     * is when whisper runs. Called from the capture thread of one source,
     * so a long transcription on one side never stalls the other.
     */
    external fun feed(handle: Long, samples: FloatArray, language: String?): String?

    /** Transcribe whatever is buffered, for when a source stops. */
    external fun flush(handle: Long, language: String?): String?

}

/**
 * What a caption looks like once the native side is done with it. Split out
 * of [NativeEngine] on purpose: that object loads the .so in its
 * initialiser, and the JSON handling has to stay testable on a plain JVM
 * where no `liblivesubs.so` exists.
 */
data class Recognition(val text: String, val language: String)

object CaptionJson {
    /** Parse what `NativeEngine.feed`/`flush` return; null for "no caption". */
    fun parse(json: String?): Recognition? {
        if (json.isNullOrEmpty()) return null
        return runCatching {
            val obj = JSONObject(json)
            val text = obj.optString("text").trim()
            if (text.isEmpty()) null else Recognition(text = text, language = obj.optString("language"))
        }.getOrNull()
    }
}
