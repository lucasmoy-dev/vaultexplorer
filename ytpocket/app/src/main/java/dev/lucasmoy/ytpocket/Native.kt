package dev.lucasmoy.ytpocket

import android.content.Context
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
    /**
     * Why the library is loaded into a nullable error instead of an `init`
     * block: a throw inside an object initialiser becomes an
     * `ExceptionInInitializerError` the first time *anything* touches this
     * object -- which would take the whole app down at startup on a device
     * where the .so cannot load, instead of failing the one action that
     * needs it with a message. It also lets the JVM-side tests build the
     * screen without a native library.
     */
    private val loadError: String? = try {
        System.loadLibrary("ytpocket")
        null
    } catch (error: Throwable) {
        error.message ?: "no se pudo cargar la librería nativa"
    }

    private fun requireLibrary() {
        loadError?.let { throw IllegalStateException(it) }
    }

    private external fun initCache(dir: String)
    private external fun search(query: String, limit: Int): String?
    private external fun resolve(video: String): String?
    private external fun fileName(title: String, ext: String): String?
    private external fun totalSize(url: String, userAgent: String): String?
    private external fun downloadChunk(
        url: String,
        path: String,
        offset: Long,
        maxBytes: Long,
        userAgent: String,
    ): String?
    private external fun transcodeMp3(
        source: String,
        destination: String,
        title: String,
        artist: String,
    ): String?

    /**
     * Point the native cache at a writable directory. Must happen before the
     * first search: rustypipe defaults its cache to the process's working
     * directory, which on Android is `/` and read-only, and without a cache
     * every single search re-downloads and re-parses YouTube's player JS.
     */
    fun prepare(context: Context) {
        if (loadError != null) return
        val dir = java.io.File(context.cacheDir, "youtube").apply { mkdirs() }
        runCatching { initCache(dir.absolutePath) }
    }

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
        /** Which YouTube client minted these URLs. */
        val client: String,
        /**
         * The agent those URLs expect. Sending anything else can be answered
         * with 403 -- which is why the download goes through the native side
         * (same HTTP stack that resolved them) rather than a second client
         * here. See `jni/src/download.rs`.
         */
        val userAgent: String,
    )

    /** Search, or throw with the message the native side reported. */
    fun searchVideos(query: String, limit: Int = 25): List<Hit> {
        requireLibrary()
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
        requireLibrary()
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
            client = obj.optString("client"),
            userAgent = obj.optString("user_agent"),
        )
    }

    /** Total bytes of a stream, or 0 when the server won't say. */
    fun sizeOf(url: String, userAgent: String): Long {
        requireLibrary()
        val raw = totalSize(url, userAgent) ?: return 0
        raw.errorOrNull()?.let { throw IllegalStateException(it) }
        return JSONObject(raw).optLong("total")
    }

    /**
     * Append one chunk to `path`, returning how many bytes landed. Zero means
     * the file ended. Throws with YouTube's own reason (403, 410, …) so the
     * caller can decide whether to re-resolve and retry.
     */
    fun fetchChunk(url: String, path: String, offset: Long, maxBytes: Long, userAgent: String): Long {
        requireLibrary()
        val raw = downloadChunk(url, path, offset, maxBytes, userAgent)
            ?: throw IllegalStateException("la descarga no devolvió nada")
        raw.errorOrNull()?.let { throw IllegalStateException(it) }
        return JSONObject(raw).optLong("written")
    }

    /** A filename for this title, safe for the phone's storage. */
    fun nameFor(title: String, ext: String): String {
        requireLibrary()
        return fileName(title, ext) ?: "video.$ext"
    }

    /** Transcode a downloaded .m4a to a tagged MP3, or throw. */
    fun toMp3(source: String, destination: String, title: String, artist: String) {
        requireLibrary()
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
