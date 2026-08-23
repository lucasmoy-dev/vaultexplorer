package dev.lucasmoy.ytpocket

import org.json.JSONArray
import org.json.JSONObject

/**
 * The native side (see `../jni`): YouTube search, stream resolution, safe
 * filenames and MP3 encoding.
 *
 * Everything crosses as JSON, and every call can come back as
 * `{"error": "..."}` -- which is deliberate: YouTube changing something is
 * this app's normal failure mode, and an error the UI can show beats an
 * empty list that looks like "no results".
 *
 * All of these block. Callers are on a background thread.
 */
object Native {
    init {
        System.loadLibrary("ytpocket")
    }

    private external fun search(query: String, limit: Int): String?
    private external fun resolve(video: String): String?
    private external fun fileName(title: String, ext: String): String?
    private external fun transcodeMp3(
        source: String,
        destination: String,
        title: String,
        artist: String,
    ): String?

    data class Hit(
        val id: String,
        val title: String,
        val channel: String,
        /** Seconds, or null for a livestream (which has nothing to download). */
        val duration: Int?,
        val views: Long?,
        val published: String?,
        val thumbnail: String,
    ) {
        val isLive: Boolean get() = duration == null
    }

    data class Stream(
        val url: String,
        val ext: String,
        val codec: String,
        val bitrate: Int,
        val height: Int,
        /** Bytes, or 0 when YouTube does not declare a length. */
        val size: Long,
    )

    data class Resolved(
        val id: String,
        val title: String,
        val channel: String,
        val duration: Int?,
        val audio: Stream?,
        val video: Stream?,
    )

    /** Search, or throw with the message the native side reported. */
    fun searchVideos(query: String, limit: Int = 25): List<Hit> {
        val raw = search(query, limit) ?: throw IllegalStateException("la búsqueda no devolvió nada")
        raw.errorOrNull()?.let { throw IllegalStateException(it) }
        val array = JSONArray(raw)
        return (0 until array.length()).mapNotNull { index ->
            val item = array.optJSONObject(index) ?: return@mapNotNull null
            Hit(
                id = item.optString("id"),
                title = item.optString("title"),
                channel = item.optString("channel"),
                duration = item.optIntOrNull("duration"),
                views = item.optLongOrNull("views"),
                published = item.optStringOrNull("published"),
                thumbnail = item.optString("thumbnail"),
            )
        }
    }

    fun resolveVideo(idOrUrl: String): Resolved {
        val raw = resolve(idOrUrl) ?: throw IllegalStateException("no se pudo resolver el vídeo")
        raw.errorOrNull()?.let { throw IllegalStateException(it) }
        val obj = JSONObject(raw)
        return Resolved(
            id = obj.optString("id"),
            title = obj.optString("title"),
            channel = obj.optString("channel"),
            duration = obj.optIntOrNull("duration"),
            audio = obj.optJSONObject("audio")?.toStream(),
            video = obj.optJSONObject("video")?.toStream(),
        )
    }

    /** A filename for this title, safe for the phone's storage. */
    fun nameFor(title: String, ext: String): String =
        fileName(title, ext) ?: "video.$ext"

    /** Transcode a downloaded .m4a to a tagged MP3, or throw. */
    fun toMp3(source: String, destination: String, title: String, artist: String) {
        val raw = transcodeMp3(source, destination, title, artist)
            ?: throw IllegalStateException("la conversión a MP3 no devolvió nada")
        raw.errorOrNull()?.let { throw IllegalStateException(it) }
    }

    private fun JSONObject.toStream() = Stream(
        url = optString("url"),
        ext = optString("ext"),
        codec = optString("codec"),
        bitrate = optInt("bitrate"),
        height = optInt("height"),
        size = optLong("size"),
    )

    /** The `{"error": …}` shape the native side uses for every failure. */
    private fun String.errorOrNull(): String? {
        val trimmed = trim()
        if (!trimmed.startsWith("{")) return null
        val error = runCatching { JSONObject(trimmed).optString("error") }.getOrNull()
        return error?.takeIf { it.isNotEmpty() }
    }

    private fun JSONObject.optIntOrNull(key: String): Int? =
        if (isNull(key)) null else optInt(key).takeIf { it > 0 }

    private fun JSONObject.optLongOrNull(key: String): Long? =
        if (isNull(key)) null else optLong(key).takeIf { it > 0 }

    private fun JSONObject.optStringOrNull(key: String): String? =
        if (isNull(key)) null else optString(key).takeIf { it.isNotEmpty() }
}

/** Seconds as `4:07` / `1:02:03`, for a result row. */
fun formatDuration(seconds: Int?): String {
    if (seconds == null || seconds <= 0) return "en directo"
    val hours = seconds / 3600
    val minutes = (seconds % 3600) / 60
    val secs = seconds % 60
    return if (hours > 0) "%d:%02d:%02d".format(hours, minutes, secs)
    else "%d:%02d".format(minutes, secs)
}

/** View counts as `1,2 M` / `340 mil`, because 1234567 tells nobody anything. */
fun formatViews(views: Long?): String = when {
    views == null || views <= 0 -> ""
    views >= 1_000_000 -> "%.1f M".format(views / 1_000_000.0).replace('.', ',')
    views >= 1_000 -> "${views / 1_000} mil"
    else -> views.toString()
}
