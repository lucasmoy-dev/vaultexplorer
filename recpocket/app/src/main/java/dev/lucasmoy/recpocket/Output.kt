package dev.lucasmoy.recpocket

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.provider.MediaStore
import java.io.File

/**
 * Where a finished recording goes.
 *
 * `MediaStore` with `IS_PENDING`, into `Movies/RecPocket`,
 * `Music/RecPocket` or `Pictures/RecPocket`. Two reasons, both of them the
 * difference between "it works" and "why can't I find my recording":
 *
 *  * It needs **no storage permission at all** on Android 10+, so the app
 *    asks for the microphone and nothing else.
 *  * `IS_PENDING` keeps the entry invisible until the bytes are all there,
 *    so the gallery never shows a half-written video, and the file appears
 *    in the gallery and in every file manager the moment it is done.
 */
object Output {
    const val ALBUM = "RecPocket"

    /** Scratch space while recording: the file is only published when it is
     *  complete, so an interrupted recording leaves nothing behind. */
    fun workDir(context: Context): File =
        File(context.cacheDir, "recordings").apply { mkdirs() }

    fun publish(context: Context, source: File, displayName: String): Uri {
        val (collection, relative, mime) = when (source.extension.lowercase()) {
            "mp4" -> Triple(
                MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
                "${android.os.Environment.DIRECTORY_MOVIES}/$ALBUM",
                "video/mp4",
            )
            "jpg", "jpeg" -> Triple(
                MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
                "${android.os.Environment.DIRECTORY_PICTURES}/$ALBUM",
                "image/jpeg",
            )
            else -> Triple(
                MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
                "${android.os.Environment.DIRECTORY_MUSIC}/$ALBUM",
                "audio/mp4",
            )
        }

        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.RELATIVE_PATH, relative)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(collection, values)
            ?: throw IllegalStateException("no se pudo crear el archivo en la galería")
        resolver.openOutputStream(uri).use { out ->
            requireNotNull(out) { "no se pudo escribir el archivo" }
            source.inputStream().use { it.copyTo(out) }
        }
        resolver.update(
            uri,
            ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) },
            null,
            null,
        )
        source.delete()
        return uri
    }
}
