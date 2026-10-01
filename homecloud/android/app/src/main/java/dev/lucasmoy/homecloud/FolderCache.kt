package dev.lucasmoy.homecloud

import java.io.File

/**
 * The folder list as it was last seen, kept on disk.
 *
 * Opening the app used to mean a blank "Arrancando…" until the engine had
 * started and answered a full listing — seconds on a phone — even though
 * nothing about the folders had changed since the last time. The saved list
 * is shown at once, marked as not yet confirmed, and replaced by the live one
 * the moment it arrives.
 */
class FolderCache(private val file: File) {

    /** The last saved listing, or null when there is none or it is unreadable. */
    fun read(): List<SharedFolder>? = runCatching {
        if (!file.exists()) null else parseFolders(file.readText())
    }.getOrNull()

    /** Saves `json` if it differs from what is there: polls repeat themselves. */
    fun write(json: String) {
        runCatching {
            if (file.exists() && file.readText() == json) return
            val tmp = File(file.parentFile, file.name + ".tmp")
            tmp.writeText(json)
            // A rename is atomic: a crash mid-write never leaves half a list.
            tmp.renameTo(file)
        }
    }
}
