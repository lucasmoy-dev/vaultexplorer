package dev.lucasmoy.livesubs

import android.content.Context
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * The whisper models on the phone.
 *
 * Downloading is Kotlin's job here rather than Rust's (the desktop app does
 * it in Rust): the platform already has an HTTP stack, a notification to
 * report progress into and its own idea of where an app may write, and
 * linking a second TLS stack into the .so to duplicate all that would only
 * make the APK bigger.
 */
object ModelStore {
    /** name to approximate download size, for the picker. */
    val MODELS = listOf(
        "tiny" to "~75MB",
        "base" to "~148MB",
        "small" to "~488MB",
    )

    private fun fileName(model: String) = "ggml-$model.bin"

    fun dir(context: Context): File = File(context.filesDir, "models").apply { mkdirs() }

    fun file(context: Context, model: String): File = File(dir(context), fileName(model))

    fun isDownloaded(context: Context, model: String): Boolean {
        val file = file(context, model)
        return file.isFile && file.length() > 1_000_000
    }

    /**
     * Download `model` unless it is already there, reporting a 0..1
     * fraction. Writes to a `.part` file and renames on success, so an
     * interrupted download can't leave a truncated model that then fails to
     * load with something cryptic from ggml.
     */
    fun download(context: Context, model: String, onProgress: (Float) -> Unit) {
        if (isDownloaded(context, model)) return
        val target = file(context, model)
        val part = File(target.parentFile, "${target.name}.part")
        val url = URL("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${fileName(model)}")
        val connection = (url.openConnection() as HttpURLConnection).apply {
            instanceFollowRedirects = true
            connectTimeout = 30_000
            readTimeout = 60_000
        }
        try {
            if (connection.responseCode !in 200..299) {
                throw IllegalStateException("HTTP ${connection.responseCode}")
            }
            val total = connection.contentLengthLong
            connection.inputStream.use { input ->
                part.outputStream().use { output ->
                    val buffer = ByteArray(256 * 1024)
                    var done = 0L
                    while (true) {
                        val read = input.read(buffer)
                        if (read <= 0) break
                        output.write(buffer, 0, read)
                        done += read
                        if (total > 0) onProgress(done.toFloat() / total)
                    }
                }
            }
            if (!part.renameTo(target)) throw IllegalStateException("no se pudo guardar el modelo")
            onProgress(1f)
        } finally {
            connection.disconnect()
            part.delete()
        }
    }
}
