package dev.lucasmoy.ytpocket

import android.content.ContentUris
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import java.io.File
import java.util.Locale

/**
 * Where finished files go, and the two kinds of place that can be.
 *
 * * **Default:** `Music/YT Pocket` and `Movies/YT Pocket` through
 *   `MediaStore` (see [Downloads.publish]). No permission at all, and the
 *   app can rename, move and delete what it put there.
 * * **A folder the user picked** (see [Settings]): a Storage Access Framework
 *   tree, written through `DocumentsContract`.
 *
 * Both are an [Organizer.Shelf], so "Ordenar mis MP3" works the same in
 * either.
 */
object Library {
    /** Where a publish ended up, and whether that was the place asked for. */
    data class Placed(val uri: Uri, val fellBack: Boolean)

    /**
     * Put a finished file where this kind of download goes, in `folder`
     * (an artist, for an organised MP3) under it.
     *
     * If the picked folder cannot be written -- it was deleted, the SD card is
     * out, the permission was revoked -- the file goes to the default place
     * instead and the caller is told. Losing a finished download because a
     * setting went stale would be the worst possible outcome.
     */
    fun publish(context: Context, source: File, name: String, audio: Boolean, folder: String = ""): Placed {
        val tree = Settings.tree(context, audio)
        if (tree != null) {
            try {
                return Placed(TreeShelf(context, tree).write(source, folder, name), fellBack = false)
            } catch (_: Throwable) {
                // Falls through to the default below; `fellBack` says so.
            }
        }
        return Placed(Downloads.publish(context, source, name, audio, folder), fellBack = tree != null)
    }

    /** The shelf the MP3s are on right now: the picked folder, or the default. */
    fun mp3Shelf(context: Context): Organizer.Shelf =
        Settings.tree(context, audio = true)?.let { TreeShelf(context, it) } ?: MediaStoreShelf(context)

    /** The store to ask is the phone's own country's: catalogues differ. */
    fun country(): String = Locale.getDefault().country.takeIf { it.length == 2 } ?: "US"

    /** The real lookup, for the organiser. */
    class NativeTagger(private val country: String = country()) : Organizer.Tagger {
        override fun read(path: String) = Native.tags(path)
        override fun tag(path: String, hint: Native.Hint) = Native.tag(path, hint, country)
    }

    /**
     * The open descriptor itself, as `fd:N`, for [Native.tags]. Not
     * `/proc/self/fd/N`: that path is refused with "Permission denied" for
     * MediaStore's descriptors (found on the emulator), because reopening it
     * resolves to MediaProvider's own storage.
     */
    private fun <T> peekUri(context: Context, uri: Uri, block: (String) -> T): T =
        (context.contentResolver.openFileDescriptor(uri, "r")
            ?: throw IllegalStateException("no se pudo abrir el archivo")).use { descriptor ->
            block("fd:${descriptor.fd}")
        }

    private fun copyOut(context: Context, uri: Uri, target: File) {
        (context.contentResolver.openInputStream(uri) ?: throw IllegalStateException("no se pudo leer el archivo"))
            .use { input -> target.outputStream().use { input.copyTo(it, 256 * 1024) } }
    }

    private fun overwrite(context: Context, uri: Uri, source: File) {
        // "wt": truncate. Plain "w" leaves the old tail behind when the new
        // file is shorter, which a re-tagged MP3 can be.
        (context.contentResolver.openOutputStream(uri, "wt") ?: throw IllegalStateException("no se pudo escribir el archivo"))
            .use { output -> source.inputStream().use { it.copyTo(output, 256 * 1024) } }
    }

    /**
     * `Music/YT Pocket` in `MediaStore`. An app sees only the media it created
     * there unless it asks for read permission, which this one never does --
     * so "every MP3 in the folder" means exactly "every MP3 this app saved".
     */
    class MediaStoreShelf(private val context: Context) : Organizer.Shelf {
        private val root = "${Environment.DIRECTORY_MUSIC}/${Downloads.ALBUM}/"
        private val collection = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)

        override fun list(): List<Organizer.Item> {
            val items = mutableListOf<Organizer.Item>()
            context.contentResolver.query(
                collection,
                arrayOf(
                    MediaStore.MediaColumns._ID,
                    MediaStore.MediaColumns.DISPLAY_NAME,
                    MediaStore.MediaColumns.RELATIVE_PATH,
                ),
                "${MediaStore.MediaColumns.RELATIVE_PATH} LIKE ? AND ${MediaStore.MediaColumns.DISPLAY_NAME} LIKE ?",
                arrayOf("$root%", "%.mp3"),
                null,
            )?.use { cursor ->
                while (cursor.moveToNext()) {
                    val uri = ContentUris.withAppendedId(collection, cursor.getLong(0))
                    val folder = cursor.getString(2).orEmpty().removePrefix(root).trim('/')
                    items += Organizer.Item(uri.toString(), cursor.getString(1).orEmpty(), folder)
                }
            }
            return items
        }

        override fun <T> peek(item: Organizer.Item, block: (String) -> T): T = peekUri(context, Uri.parse(item.id), block)
        override fun copyOut(item: Organizer.Item, target: File) = copyOut(context, Uri.parse(item.id), target)
        override fun overwrite(item: Organizer.Item, source: File) = overwrite(context, Uri.parse(item.id), source)

        override fun add(source: File, folder: String, name: String) {
            Downloads.publish(context, source, name, audio = true, folder = folder)
        }

        override fun remove(item: Organizer.Item) {
            context.contentResolver.delete(Uri.parse(item.id), null, null)
        }
    }

    /**
     * A picked folder. Walks two levels down -- the root and one folder per
     * artist is the whole layout this app makes -- so pointing it at a huge
     * music library does not mean scanning every album folder in it.
     */
    class TreeShelf(private val context: Context, private val tree: Uri) : Organizer.Shelf {
        private val resolver = context.contentResolver
        private val rootId = DocumentsContract.getTreeDocumentId(tree)

        private data class Child(val id: String, val name: String, val isDir: Boolean)

        private fun children(parentId: String): List<Child> {
            val out = mutableListOf<Child>()
            resolver.query(
                DocumentsContract.buildChildDocumentsUriUsingTree(tree, parentId),
                arrayOf(
                    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                    DocumentsContract.Document.COLUMN_MIME_TYPE,
                ),
                null,
                null,
                null,
            )?.use { cursor ->
                while (cursor.moveToNext()) {
                    out += Child(
                        id = cursor.getString(0),
                        name = cursor.getString(1).orEmpty(),
                        isDir = cursor.getString(2) == DocumentsContract.Document.MIME_TYPE_DIR,
                    )
                }
            }
            return out
        }

        private fun documentUri(id: String) = DocumentsContract.buildDocumentUriUsingTree(tree, id)

        override fun list(): List<Organizer.Item> {
            val items = mutableListOf<Organizer.Item>()
            for (child in children(rootId)) {
                when {
                    child.isDir && !child.name.startsWith(".") ->
                        children(child.id)
                            .filter { !it.isDir && it.name.endsWith(".mp3", ignoreCase = true) }
                            .forEach { items += Organizer.Item(documentUri(it.id).toString(), it.name, child.name) }
                    !child.isDir && child.name.endsWith(".mp3", ignoreCase = true) ->
                        items += Organizer.Item(documentUri(child.id).toString(), child.name, "")
                }
            }
            return items
        }

        /** The folder for `name` under the root, made if it does not exist. */
        private fun folderId(name: String): String {
            if (name.isEmpty()) return rootId
            children(rootId).firstOrNull { it.isDir && it.name == name }?.let { return it.id }
            val made = DocumentsContract.createDocument(
                resolver, documentUri(rootId), DocumentsContract.Document.MIME_TYPE_DIR, name,
            ) ?: throw IllegalStateException("no se pudo crear la carpeta $name")
            return DocumentsContract.getDocumentId(made)
        }

        /** Write a new file and answer its URI. The provider renames on a clash ("… (1).mp3"). */
        fun write(source: File, folder: String, name: String): Uri {
            val mime = if (name.endsWith(".mp3", ignoreCase = true)) "audio/mpeg" else "video/mp4"
            val target = DocumentsContract.createDocument(resolver, documentUri(folderId(folder)), mime, name)
                ?: throw IllegalStateException("no se pudo crear $name en la carpeta elegida")
            try {
                overwrite(context, target, source)
            } catch (error: Throwable) {
                runCatching { DocumentsContract.deleteDocument(resolver, target) }
                throw error
            }
            return target
        }

        override fun <T> peek(item: Organizer.Item, block: (String) -> T): T = peekUri(context, Uri.parse(item.id), block)
        override fun copyOut(item: Organizer.Item, target: File) = copyOut(context, Uri.parse(item.id), target)
        override fun overwrite(item: Organizer.Item, source: File) = overwrite(context, Uri.parse(item.id), source)
        override fun add(source: File, folder: String, name: String) {
            write(source, folder, name)
        }

        override fun remove(item: Organizer.Item) {
            DocumentsContract.deleteDocument(resolver, Uri.parse(item.id))
        }
    }
}
