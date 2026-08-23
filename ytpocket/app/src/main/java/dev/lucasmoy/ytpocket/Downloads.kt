package dev.lucasmoy.ytpocket

import android.content.ContentValues
import android.content.Context
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMuxer
import android.net.Uri
import android.provider.MediaStore
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.nio.ByteBuffer

/**
 * Getting the bytes, and putting the finished file where the phone can see
 * it.
 *
 * Three things here are not obvious:
 *
 * 1. **An MP4 download is two downloads.** YouTube stopped serving
 *    progressive streams, so a watchable file means fetching a video-only
 *    track and an audio track and muxing them -- with `MediaMuxer`, since
 *    there is no ffmpeg on a phone. (Established by the sibling app, and
 *    the reason `resolve` returns AVC video and AAC audio specifically:
 *    those are the two the platform muxer takes without argument.)
 * 2. **An MP3 download is a transcode.** YouTube's audio is AAC; Android
 *    has no MP3 encoder, so the native side does it (see `../jni`).
 * 3. **Where the file goes.** `MediaStore` with `IS_PENDING`, into
 *    `Music/YT Pocket` or `Movies/YT Pocket`: this needs no storage
 *    permission at all, and the file shows up in the user's music player
 *    and gallery immediately -- which a file in the app's private
 *    directory never would.
 */
object Downloads {
    /** Folder name inside Music/ and Movies/, so downloads stay together. */
    const val ALBUM = "YT Pocket"

    /** How much to ask for per request. See [fetch] for why this exists. */
    private const val CHUNK = 4L * 1024 * 1024

    /**
     * Download `url` to `target`, reporting a 0..1 fraction.
     *
     * In ranged chunks rather than one long read, which is not premature
     * optimisation but a measured necessity: googlevideo trickles a plain
     * sequential download to a non-browser client, and a 3.4MB audio track
     * timed out entirely before finishing. The same file fetched as 4MB
     * ranges arrives in a couple of seconds.
     *
     * The first response's `Content-Range` is also the only reliable total,
     * so progress comes out of it rather than out of what YouTube declared
     * in the stream list (`expectedSize`, used as a fallback).
     */
    fun fetch(url: String, target: File, expectedSize: Long, onProgress: (Float) -> Unit) {
        target.parentFile?.mkdirs()
        var total = expectedSize
        var done = 0L
        target.outputStream().use { output ->
            while (true) {
                val connection = (URL(url).openConnection() as HttpURLConnection).apply {
                    instanceFollowRedirects = true
                    connectTimeout = 30_000
                    readTimeout = 60_000
                    setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android 13) YTPocket/1.0")
                    setRequestProperty("Range", "bytes=$done-${done + CHUNK - 1}")
                }
                try {
                    if (connection.responseCode !in 200..299) {
                        throw IllegalStateException("HTTP ${connection.responseCode}")
                    }
                    // "bytes 0-4194303/3449447" -- the part after the slash is
                    // the real length of the whole file.
                    if (total <= 0) {
                        total = connection.getHeaderField("Content-Range")
                            ?.substringAfterLast('/')
                            ?.toLongOrNull()
                            ?: connection.contentLengthLong
                    }
                    var chunkRead = 0L
                    connection.inputStream.use { input ->
                        val buffer = ByteArray(256 * 1024)
                        while (true) {
                            val read = input.read(buffer)
                            if (read <= 0) break
                            output.write(buffer, 0, read)
                            chunkRead += read
                            done += read
                            if (total > 0) onProgress((done.toFloat() / total).coerceIn(0f, 1f))
                        }
                    }
                    // A short chunk means the file ended -- servers may also
                    // answer with less than asked for, hence "short", not
                    // "empty".
                    if (chunkRead < CHUNK) break
                } finally {
                    connection.disconnect()
                }
            }
        }
        if (done <= 0) throw IllegalStateException("la descarga vino vacía")
        onProgress(1f)
    }

    /**
     * Join a video-only track and an audio track into one MP4, copying both
     * without re-encoding (so it costs seconds, not minutes, and loses
     * nothing).
     */
    fun mux(video: File, audio: File, output: File) {
        val muxer = MediaMuxer(output.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
        val extractors = mutableListOf<MediaExtractor>()
        try {
            // Track index in the output, paired with the extractor it is fed
            // from -- the two files' internal track numbers are unrelated to
            // the muxer's.
            data class Feed(val extractor: MediaExtractor, val sourceTrack: Int, val outputTrack: Int, val maxInput: Int)

            val feeds = listOf(video to "video/", audio to "audio/").mapNotNull { (file, prefix) ->
                val extractor = MediaExtractor().apply { setDataSource(file.absolutePath) }
                extractors += extractor
                val track = (0 until extractor.trackCount).firstOrNull { index ->
                    extractor.getTrackFormat(index).getString(MediaFormat.KEY_MIME)?.startsWith(prefix) == true
                } ?: return@mapNotNull null
                val format = extractor.getTrackFormat(track)
                extractor.selectTrack(track)
                val maxInput = if (format.containsKey(MediaFormat.KEY_MAX_INPUT_SIZE)) {
                    format.getInteger(MediaFormat.KEY_MAX_INPUT_SIZE)
                } else {
                    1 shl 20
                }
                Feed(extractor, track, muxer.addTrack(format), maxInput.coerceAtLeast(256 * 1024))
            }
            if (feeds.isEmpty()) throw IllegalStateException("no se encontraron pistas para unir")

            muxer.start()
            val info = MediaCodec.BufferInfo()
            for (feed in feeds) {
                val buffer = ByteBuffer.allocate(feed.maxInput)
                while (true) {
                    val size = feed.extractor.readSampleData(buffer, 0)
                    if (size < 0) break
                    info.offset = 0
                    info.size = size
                    info.presentationTimeUs = feed.extractor.sampleTime
                    // Translated, not copied: these are two different flag
                    // sets that happen to overlap. `SAMPLE_FLAG_SYNC` (1) and
                    // `BUFFER_FLAG_KEY_FRAME` (1) agree, but
                    // `SAMPLE_FLAG_ENCRYPTED` (2) collides with
                    // `BUFFER_FLAG_CODEC_CONFIG` (2) -- passing the raw value
                    // through would tell the muxer that a frame is codec
                    // configuration, and produce a file that does not play.
                    info.flags = if (feed.extractor.sampleFlags and MediaExtractor.SAMPLE_FLAG_SYNC != 0) {
                        MediaCodec.BUFFER_FLAG_KEY_FRAME
                    } else {
                        0
                    }
                    muxer.writeSampleData(feed.outputTrack, buffer, info)
                    feed.extractor.advance()
                }
            }
            muxer.stop()
        } finally {
            muxer.release()
            extractors.forEach { it.release() }
        }
    }

    /**
     * Copy a finished file into the user's Music/ or Movies/ through
     * `MediaStore`, and return where it landed. `IS_PENDING` keeps it
     * invisible until the bytes are all there, so a music player never
     * indexes a half file.
     */
    fun publish(context: Context, source: File, displayName: String, audio: Boolean): Uri {
        val collection = if (audio) {
            MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        } else {
            MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        }
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, if (audio) "audio/mpeg" else "video/mp4")
            put(
                MediaStore.MediaColumns.RELATIVE_PATH,
                if (audio) "${android.os.Environment.DIRECTORY_MUSIC}/$ALBUM"
                else "${android.os.Environment.DIRECTORY_MOVIES}/$ALBUM",
            )
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(collection, values)
            ?: throw IllegalStateException("no se pudo crear el archivo en la galería")
        try {
            resolver.openOutputStream(uri, "w")?.use { output ->
                source.inputStream().use { input -> input.copyTo(output, 256 * 1024) }
            } ?: throw IllegalStateException("no se pudo escribir el archivo")
        } catch (error: Throwable) {
            // A pending entry nobody finished is invisible clutter; drop it
            // so a retry does not accumulate "Song (1).mp3" ghosts.
            resolver.delete(uri, null, null)
            throw error
        }
        resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
        return uri
    }

    /** Scratch space for the in-flight parts, cleared as soon as they're used. */
    fun workDir(context: Context): File = File(context.cacheDir, "downloads").apply { mkdirs() }
}
