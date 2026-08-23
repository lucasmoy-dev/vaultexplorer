package dev.lucasmoy.livesubs

import android.content.Context
import android.net.Uri
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * The optional transcript file, in the same format the desktop app writes:
 *
 * ```
 * [2026-08-23 00:59:03] [system] [en] Hello everyone
 * [2026-08-23 00:59:03] [system] [->] Hola a todos
 * ```
 *
 * The destination is a document the user picked (SAF), not a path: on
 * Android an app cannot simply write to ~/Documents, and a file in the
 * app's private storage would be a transcript nobody can open. Appending
 * needs mode "wa" -- plain "w" truncates, which would leave only the last
 * line of a meeting.
 */
object Transcript {
    private val stamp = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US)

    fun append(
        context: Context,
        uri: Uri,
        source: String,
        language: String,
        text: String,
        translated: String?,
    ): Result<Unit> = runCatching {
        val now = stamp.format(Date())
        val lang = language.ifEmpty { "??" }
        val builder = StringBuilder().append("[$now] [$source] [$lang] $text\n")
        if (translated != null && translated != text) {
            builder.append("[$now] [$source] [->] $translated\n")
        }
        context.contentResolver.openOutputStream(uri, "wa")?.use { stream ->
            stream.write(builder.toString().toByteArray())
        } ?: throw IllegalStateException("no se pudo abrir el archivo de transcripción")
    }
}
