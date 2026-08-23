package dev.lucasmoy.recpocket

import java.util.Calendar
import java.util.Locale
import java.util.TimeZone

/**
 * What a recording is called.
 *
 * The date, backwards: `2026-08-23_17-49-43`. That is the one filename
 * convention that sorts chronologically in every file manager, gallery and
 * `ls` on earth, which is the whole point when the folder fills up with
 * recordings whose contents are indistinguishable from their names.
 *
 * After the stamp comes a word for *what* it is (`voz`, `salida`, `mixto`,
 * `pantalla`, `llamada`), because "which of these fourteen files is the call
 * from Tuesday" is the second question anyone asks.
 *
 * Deliberately hand-rolled with `Calendar` instead of a formatter: this is
 * called from a foreground service on old devices, `SimpleDateFormat` is not
 * thread-safe, and `java.time` formatting patterns are a needless dependency
 * for six numbers.
 */
object Naming {
    enum class Kind(val label: String, val extension: String) {
        MIC("voz", "m4a"),
        PLAYBACK("salida", "m4a"),
        BOTH("mixto", "m4a"),
        SCREEN("pantalla", "mp4"),
        CALL_AUDIO("llamada", "m4a"),
        CALL_VIDEO("videollamada", "mp4"),
        SCREENSHOT("captura", "jpg"),
    }

    /** `2026-08-23_17-49-43`, in the phone's own time zone. */
    fun stamp(millis: Long, zone: TimeZone = TimeZone.getDefault()): String {
        val calendar = Calendar.getInstance(zone, Locale.ROOT).apply { timeInMillis = millis }
        return "%04d-%02d-%02d_%02d-%02d-%02d".format(
            Locale.ROOT,
            calendar.get(Calendar.YEAR),
            calendar.get(Calendar.MONTH) + 1,
            calendar.get(Calendar.DAY_OF_MONTH),
            calendar.get(Calendar.HOUR_OF_DAY),
            calendar.get(Calendar.MINUTE),
            calendar.get(Calendar.SECOND),
        )
    }

    /**
     * The full name. `note` is anything worth remembering about this one
     * (the app that was on screen, say); it is sanitised and appended.
     */
    fun fileName(
        kind: Kind,
        millis: Long,
        note: String = "",
        zone: TimeZone = TimeZone.getDefault(),
    ): String {
        val extra = sanitize(note)
        val middle = if (extra.isEmpty()) kind.label else "${kind.label} $extra"
        return "${stamp(millis, zone)} $middle.${kind.extension}"
    }

    /**
     * Anything a filesystem (or a person reading a file list) would rather
     * not see. `/ \ : * ? " < > |` become dashes rather than vanishing, so
     * "AC/DC" reads "AC-DC" instead of "ACDC", and runs of whitespace
     * collapse.
     */
    fun sanitize(part: String): String =
        part.map { c ->
            when {
                c in "/\\:*?\"<>|" -> '-'
                c.code < 0x20 -> ' '
                else -> c
            }
        }
            .joinToString("")
            .replace(Regex("\\s+"), " ")
            .trim()
            .trimEnd('.')
}
