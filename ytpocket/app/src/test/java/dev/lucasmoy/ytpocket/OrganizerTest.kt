package dev.lucasmoy.ytpocket

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What "Ordenar mis MP3" does to each file, with the storage and the lookup
 * faked -- the real ones are exercised on the emulator
 * (`OrganizeOnDeviceTest`). Here the point is the order of operations: a
 * file is never deleted before its replacement exists, a file that is fine is
 * not copied, and one failure does not stop the run.
 */
class OrganizerTest {
    private val work: File = Files.createTempDirectory("organizer-test").toFile()

    /** An in-memory shelf that records every operation. */
    private class FakeShelf(items: List<Organizer.Item>) : Organizer.Shelf {
        val files = items.associate { it.id to "id=${it.id} bytes of ${it.name}" }.toMutableMap()
        val entries = items.toMutableList()
        val log = mutableListOf<String>()
        var failAdd = false
        var failPeek = false

        override fun list() = entries.toList()
        override fun <T> peek(item: Organizer.Item, block: (String) -> T): T {
            log += "peek ${item.name}"
            if (failPeek) throw IllegalStateException("not a real descriptor")
            return block(item.id)
        }
        override fun copyOut(item: Organizer.Item, target: File) {
            log += "copy ${item.name}"
            target.writeText(files.getValue(item.id))
        }
        override fun overwrite(item: Organizer.Item, source: File) {
            log += "overwrite ${item.name}"
            files[item.id] = source.readText()
        }
        override fun add(source: File, folder: String, name: String) {
            if (failAdd) throw IllegalStateException("disk full")
            log += "add $folder/$name"
            val id = "new:$folder/$name"
            files[id] = source.readText()
            entries += Organizer.Item(id, name, folder)
        }
        override fun remove(item: Organizer.Item) {
            log += "remove ${item.name}"
            files.remove(item.id)
            entries.remove(item)
        }
    }

    /** Tags keyed by item id; "tagging" appends a marker to the temp file. */
    private class FakeTagger(
        val existing: Map<String, Native.Existing>,
        val results: Map<String, Native.Tagged>,
    ) : Organizer.Tagger {
        var lookups = 0
        override fun read(path: String) =
            existing[path] ?: existing.getValue(File(path).readText().substringAfter("id=").substringBefore(" "))
        override fun tag(path: String, hint: Native.Hint): Native.Tagged {
            lookups++
            File(path).appendText(" +tags")
            return results[hint.name] ?: throw IllegalStateException("lookup exploded")
        }
    }

    private fun existing(madeByUs: Boolean = true, organised: Boolean = false, name: String = "", folder: String = "") =
        Native.Existing("t", "a", "", madeByUs, organised, name, folder)

    private fun tagged(name: String, folder: String, source: String = "itunes", rewritten: Boolean = true) =
        Native.Tagged("Queen", "Bohemian Rhapsody", "A Night At The Opera", "1975", source, name, folder, true, rewritten)

    private val target = "Queen - A Night At The Opera - Bohemian Rhapsody.mp3"

    @Test
    fun `an old file is copied, tagged, written to its artist folder, and only then removed`() {
        val old = Organizer.Item("1", "Queen – Bohemian Rhapsody (Official Video).mp3", "")
        val shelf = FakeShelf(listOf(old))
        val tagger = FakeTagger(
            mapOf("1" to existing()),
            mapOf("Queen – Bohemian Rhapsody (Official Video)" to tagged(target, "Queen")),
        )
        val summary = Organizer(shelf, tagger, work, onlyOurs = false).run()

        assertEquals(
            listOf("peek ${old.name}", "copy ${old.name}", "add Queen/$target", "remove ${old.name}"),
            shelf.log,
        )
        assertEquals("id=1 bytes of ${old.name} +tags", shelf.files["new:Queen/$target"])
        assertFalse("old entry still there", shelf.files.containsKey("1"))
        assertEquals(1, summary.organised)
        assertEquals(1, summary.matched)
        assertTrue("temp files left behind", work.listFiles().orEmpty().isEmpty())
    }

    @Test
    fun `a file already in place is neither copied nor looked up`() {
        val done = Organizer.Item("1", target, "Queen")
        val shelf = FakeShelf(listOf(done))
        val tagger = FakeTagger(mapOf("1" to existing(organised = true, name = target, folder = "Queen")), emptyMap())
        val summary = Organizer(shelf, tagger, work, onlyOurs = false).run()

        assertEquals(listOf("peek $target"), shelf.log)
        assertEquals(0, tagger.lookups)
        assertEquals(1, summary.unchanged)
        assertEquals(0, summary.organised)
    }

    @Test
    fun `a numbered duplicate is in place, not moved again on every run`() {
        val copy = target.replace(".mp3", " (1).mp3")
        val shelf = FakeShelf(listOf(Organizer.Item("1", copy, "Queen")))
        val tagger = FakeTagger(mapOf("1" to existing(organised = true, name = target, folder = "Queen")), emptyMap())
        val summary = Organizer(shelf, tagger, work, onlyOurs = false).run()
        assertEquals(listOf("peek $copy"), shelf.log)
        assertEquals(1, summary.unchanged)
    }

    @Test
    fun `when only the tags change the file is rewritten where it stands`() {
        val item = Organizer.Item("1", target, "Queen")
        val shelf = FakeShelf(listOf(item))
        val tagger = FakeTagger(mapOf("1" to existing()), mapOf(target.removeSuffix(".mp3") to tagged(target, "Queen")))
        Organizer(shelf, tagger, work, onlyOurs = false).run()

        assertEquals(listOf("peek $target", "copy $target", "overwrite $target"), shelf.log)
        assertEquals("id=1 bytes of $target +tags", shelf.files["1"])
    }

    @Test
    fun `in a folder the user picked, files this app did not make are left alone`() {
        val theirs = Organizer.Item("1", "Their song.mp3", "")
        val shelf = FakeShelf(listOf(theirs))
        val tagger = FakeTagger(mapOf("1" to existing(madeByUs = false)), emptyMap())
        val summary = Organizer(shelf, tagger, work, onlyOurs = true).run()

        assertEquals(listOf("peek Their song.mp3"), shelf.log)
        assertEquals(1, summary.foreign)
        assertEquals(0, tagger.lookups)
    }

    @Test
    fun `one failure is counted, the original survives, and the run goes on`() {
        val broken = Organizer.Item("1", "broken.mp3", "")
        val fine = Organizer.Item("2", "fine.mp3", "")
        val shelf = FakeShelf(listOf(broken, fine))
        val tagger = FakeTagger(
            mapOf("1" to existing(), "2" to existing()),
            // No result for "broken": its lookup throws.
            mapOf("fine" to tagged("Queen - Fine.mp3", "Queen", source = "youtube")),
        )
        val summary = Organizer(shelf, tagger, work, onlyOurs = false).run()

        assertEquals(1, summary.failed)
        assertTrue(summary.firstError!!.startsWith("broken.mp3: lookup exploded"))
        assertTrue("the broken file was deleted", shelf.files.containsKey("1"))
        assertEquals(1, summary.organised)
        assertEquals(1, summary.fromTitle)
        assertTrue(work.listFiles().orEmpty().isEmpty())
    }

    @Test
    fun `if the new file cannot be written the old one is not removed`() {
        val old = Organizer.Item("1", "old.mp3", "")
        val shelf = FakeShelf(listOf(old)).apply { failAdd = true }
        val tagger = FakeTagger(mapOf("1" to existing()), mapOf("old" to tagged(target, "Queen")))
        val summary = Organizer(shelf, tagger, work, onlyOurs = false).run()

        assertEquals(1, summary.failed)
        assertFalse(shelf.log.any { it.startsWith("remove") })
        assertEquals("id=1 bytes of old.mp3", shelf.files["1"])
    }

    @Test
    fun `a provider that cannot be read in place is read from the copy`() {
        val item = Organizer.Item("1", target, "Queen")
        val shelf = FakeShelf(listOf(item)).apply { failPeek = true }
        val tagger = FakeTagger(mapOf("1" to existing(organised = true, name = target, folder = "Queen")), emptyMap())
        val summary = Organizer(shelf, tagger, work, onlyOurs = false).run()
        assertEquals(listOf("peek $target", "copy $target"), shelf.log)
        assertEquals(1, summary.unchanged)
        assertTrue(work.listFiles().orEmpty().isEmpty())
    }

    @Test
    fun `progress counts every file and ends at the total`() {
        val shelf = FakeShelf(listOf(Organizer.Item("1", target, "Queen"), Organizer.Item("2", target, "Queen")))
        val tagger = FakeTagger(
            mapOf("1" to existing(organised = true, name = target, folder = "Queen"),
                "2" to existing(organised = true, name = target, folder = "Queen")),
            emptyMap(),
        )
        val seen = mutableListOf<Pair<Int, Int>>()
        Organizer(shelf, tagger, work, onlyOurs = false).run { done, total, _ -> seen += done to total }
        assertEquals(listOf(0 to 2, 1 to 2, 2 to 2), seen)
    }

    /** The JSON the native side answers with, as `jni/src/tagging.rs` serialises `Outcome`. */
    @Test
    fun `the native tagging answer parses into names and folders`() {
        val raw = """{"artist":"Queen","title":"Bohemian Rhapsody","album":"A Night At The Opera",
            "album_artist":"Queen","date":"1975-10-31","genre":"Rock","track":11,"track_total":12,
            "source":"itunes","file_name":"$target","folder":"Queen","cover":true,"rewritten":true}"""
        val parsed = Native.parseTagged(raw)
        assertEquals(target, parsed.fileName)
        assertEquals("Queen", parsed.folder)
        assertEquals("itunes", parsed.source)
        assertEquals("1975-10-31", parsed.date)
        assertTrue(parsed.cover && parsed.rewritten)
    }
}
