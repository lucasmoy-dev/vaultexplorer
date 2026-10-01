package dev.lucasmoy.ytpocket

import java.io.File

/**
 * "Ordenar mis MP3": every MP3 in the music folder renamed to
 * `Artist - Album - Song.mp3`, re-tagged (album, date, cover, length…) and
 * moved into a folder per artist.
 *
 * The lookup itself is native (`jni/src/tagging.rs`); this is the part that
 * decides what to do with each file, and it is kept free of Android types so
 * the JVM tests can drive it with a fake shelf -- which is how the order of
 * operations below is checked:
 *
 * * **Read before copying.** Tags are read through a descriptor first, so a
 *   file that is already where it belongs costs no copy and no request. That
 *   is what makes a second run quick.
 * * **Write the new file, then delete the old one.** Never the other way
 *   round, and never a rewrite in place when the file moves: a failure half
 *   way through leaves the original untouched, not a truncated song.
 * * **One file failing is one file failing.** It is counted and the run goes
 *   on; the first error is kept so the summary can say what it was.
 */
class Organizer(
    private val shelf: Shelf,
    private val tagger: Tagger,
    private val work: File,
    /**
     * Only files carrying this app's mark. On for a folder the user picked
     * (it may be their whole music library); off for the app's own
     * `Music/YT Pocket`, where every file is ours by construction -- including
     * the ones from before the mark existed.
     */
    private val onlyOurs: Boolean,
) {
    /** One MP3 on the shelf. `folder` is relative to the shelf's root, "" for the root. */
    data class Item(val id: String, val name: String, val folder: String)

    /** Where the files are: `MediaStore`'s `Music/YT Pocket`, or a picked folder. */
    interface Shelf {
        fun list(): List<Item>
        /**
         * Let `block` read the file without copying it, through a source
         * string the [Tagger] understands (an open descriptor, `fd:N`, for
         * the real shelves).
         */
        fun <T> peek(item: Item, block: (String) -> T): T
        fun copyOut(item: Item, target: File)
        fun overwrite(item: Item, source: File)
        fun add(source: File, folder: String, name: String)
        fun remove(item: Item)
    }

    interface Tagger {
        fun read(path: String): Native.Existing
        fun tag(path: String, hint: Native.Hint): Native.Tagged
    }

    data class Summary(
        val total: Int = 0,
        /** Renamed or moved, or re-tagged where it stood. */
        val organised: Int = 0,
        /** Of those, found in a music store (or YouTube Music's own data). */
        val matched: Int = 0,
        /** Of those, tagged from the video title alone: no store knew the song. */
        val fromTitle: Int = 0,
        val unchanged: Int = 0,
        /** Not made by this app, in a folder the user picked: left alone. */
        val foreign: Int = 0,
        val failed: Int = 0,
        val firstError: String? = null,
    )

    fun run(onProgress: (done: Int, total: Int, name: String) -> Unit = { _, _, _ -> }): Summary {
        val items = shelf.list()
        var summary = Summary(total = items.size)
        items.forEachIndexed { index, item ->
            onProgress(index, items.size, item.name)
            summary = try {
                organise(item, summary)
            } catch (error: Throwable) {
                summary.copy(
                    failed = summary.failed + 1,
                    firstError = summary.firstError ?: "${item.name}: ${error.message ?: error::class.java.simpleName}",
                )
            }
        }
        onProgress(items.size, items.size, "")
        return summary
    }

    /**
     * `Song (1).mp3` counts as `Song.mp3`: it is what storage calls a second
     * copy of the same song, and without this every run would move it again
     * -- copy, delete, and get "(1)" once more.
     */
    private fun sameName(actual: String, wanted: String): Boolean {
        if (actual == wanted) return true
        val stem = wanted.substringBeforeLast('.')
        val ext = wanted.substringAfterLast('.', "")
        return Regex(Regex.escape(stem) + " \\(\\d+\\)\\." + Regex.escape(ext)).matches(actual)
    }

    private fun organise(item: Item, summary: Summary): Summary {
        val temp = File(work, "organise-${System.nanoTime()}.mp3")
        var copied = false
        fun local(): String {
            if (!copied) {
                shelf.copyOut(item, temp)
                copied = true
            }
            return temp.absolutePath
        }
        try {
            // Through the descriptor when the provider gives a real one; a
            // provider that streams (a pipe, a cloud folder) cannot be read
            // in place, and the copy is read instead.
            val existing = runCatching { shelf.peek(item) { source -> tagger.read(source) } }
                .getOrElse { tagger.read(local()) }
            if (onlyOurs && !existing.madeByUs) return summary.copy(foreign = summary.foreign + 1)
            if (existing.organised && sameName(item.name, existing.fileName) && existing.folder == item.folder) {
                return summary.copy(unchanged = summary.unchanged + 1)
            }

            local()
            val tagged = tagger.tag(temp.absolutePath, Native.Hint(name = item.name.removeSuffix(".mp3")))
            if (tagged.folder == item.folder && sameName(item.name, tagged.fileName)) {
                // Same place, same name: only the tags changed.
                if (tagged.rewritten) shelf.overwrite(item, temp)
            } else {
                shelf.add(temp, tagged.folder, tagged.fileName)
                shelf.remove(item)
            }
            val fromStore = tagged.source == "itunes" || tagged.source == "youtube_music"
            return summary.copy(
                organised = summary.organised + 1,
                matched = summary.matched + if (fromStore) 1 else 0,
                fromTitle = summary.fromTitle + if (tagged.source == "youtube") 1 else 0,
            )
        } finally {
            temp.delete()
        }
    }
}
