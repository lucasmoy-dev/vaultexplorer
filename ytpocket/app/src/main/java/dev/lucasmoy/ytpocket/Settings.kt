package dev.lucasmoy.ytpocket

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract

/**
 * The three choices the app remembers: where MP3s go, where MP4s go, and
 * whether a new MP3 is tagged and filed by artist as it lands.
 *
 * A folder is a Storage Access Framework *tree* the user picked, kept as its
 * URI with a persisted permission. Not a path: since scoped storage an app
 * cannot write to an arbitrary path at all, and a picked tree is the one
 * mechanism that reaches any folder -- an SD card included -- without asking
 * for "all files" access. No folder chosen means the default, `Music/YT Pocket`
 * and `Movies/YT Pocket` through `MediaStore`, which needs no permission.
 */
object Settings {
    private const val FILE = "settings"
    private const val KEY_MP3_TREE = "mp3_tree"
    private const val KEY_MP4_TREE = "mp4_tree"
    private const val KEY_AUTO_ORGANISE = "auto_organise"

    private fun prefs(context: Context) = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** The picked folder for this kind, or null for the default. */
    fun tree(context: Context, audio: Boolean): Uri? =
        prefs(context).getString(if (audio) KEY_MP3_TREE else KEY_MP4_TREE, null)?.let(Uri::parse)

    /**
     * Remember a picked folder (null = back to the default), keeping the
     * permission to write there across reboots and giving up the old one --
     * an app holds a limited number of persisted grants.
     */
    fun setTree(context: Context, audio: Boolean, uri: Uri?) {
        val resolver = context.contentResolver
        val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
        val previous = tree(context, audio)
        if (uri != null) resolver.takePersistableUriPermission(uri, flags)
        prefs(context).edit()
            .putString(if (audio) KEY_MP3_TREE else KEY_MP4_TREE, uri?.toString())
            .apply()
        // Only released when the other kind is not using the same folder.
        if (previous != null && previous != uri && previous != tree(context, !audio)) {
            runCatching { resolver.releasePersistableUriPermission(previous, flags) }
        }
    }

    /**
     * On by default: the user asked for tagged, named, filed music, and a
     * lookup costs one request per song.
     */
    fun autoOrganise(context: Context): Boolean = prefs(context).getBoolean(KEY_AUTO_ORGANISE, true)

    fun setAutoOrganise(context: Context, on: Boolean) {
        prefs(context).edit().putBoolean(KEY_AUTO_ORGANISE, on).apply()
    }

    /** "Música/YT Pocket", or the picked folder as a person would say it. */
    fun label(context: Context, audio: Boolean): String {
        val tree = tree(context, audio)
            ?: return context.getString(if (audio) R.string.folder_default_mp3 else R.string.folder_default_mp4)
        return describeTree(context, tree)
    }

    /**
     * `primary:Music/Mine` -> "Memoria interna/Music/Mine". The document id of
     * a tree from the platform's storage provider is `<volume>:<path>`; any
     * other provider's id is opaque, and its last path segment is the best
     * there is.
     */
    fun describeTree(context: Context, tree: Uri): String {
        val id = runCatching { DocumentsContract.getTreeDocumentId(tree) }.getOrNull()
            ?: return tree.lastPathSegment.orEmpty()
        val volume = id.substringBefore(':', "")
        val path = id.substringAfter(':', id)
        val root = when {
            volume == "primary" -> context.getString(R.string.folder_internal)
            volume.isNotEmpty() && tree.authority == "com.android.externalstorage.documents" ->
                context.getString(R.string.folder_sd)
            else -> volume
        }
        return listOf(root, path).filter { it.isNotEmpty() }.joinToString("/")
    }
}
