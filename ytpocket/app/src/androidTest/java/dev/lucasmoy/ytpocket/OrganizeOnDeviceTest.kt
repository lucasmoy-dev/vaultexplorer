package dev.lucasmoy.ytpocket

import android.content.Context
import android.media.MediaMetadataRetriever
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * "Ordenar mis MP3" and the tagging of a new download, on a real Android:
 * the real native library, real `MediaStore` (renaming, moving into an
 * artist folder, deleting the old entry), the real iTunes API and the real
 * download service.
 *
 *     gradle :app:connectedDebugAndroidTest -PrustAbis=x86_64
 */
@RunWith(AndroidJUnit4::class)
class OrganizeOnDeviceTest {
    private lateinit var context: Context

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        Native.prepare(context)
        Settings.setTree(context, audio = true, uri = null)
        clearShelf()
    }

    @After
    fun tearDown() = clearShelf()

    /** This app's own MP3s only -- it cannot see anyone else's. */
    private fun clearShelf() {
        val shelf = Library.MediaStoreShelf(context)
        shelf.list().forEach { shelf.remove(it) }
    }

    private fun metadata(uri: Uri): Map<String, String?> {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(context, uri)
            return mapOf(
                "title" to retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_TITLE),
                "artist" to retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_ARTIST),
                "album" to retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_ALBUM),
                "year" to retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_YEAR),
                "duration" to retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION),
                "cover" to retriever.embeddedPicture?.size?.toString(),
            )
        } finally {
            retriever.release()
        }
    }

    /**
     * A file saved the way 0.1.x saved them -- video title as title, channel
     * as artist, in the root of `Music/YT Pocket`, no mark -- comes out
     * renamed, re-tagged and in its artist's folder; the old entry is gone;
     * and a second run touches nothing.
     */
    @Test
    fun an_old_download_is_renamed_tagged_and_filed_under_its_artist() {
        val source = File(Downloads.workDir(context), "old-download.mp3")
        InstrumentationRegistry.getInstrumentation().context.assets.open("old-download.mp3").use { input ->
            source.outputStream().use { input.copyTo(it) }
        }
        Downloads.publish(context, source, "Queen – Bohemian Rhapsody (Official Video Remastered).mp3", audio = true)
        source.delete()
        val shelf = Library.MediaStoreShelf(context)
        assertEquals("", shelf.list().single().folder)

        val organizer = Organizer(shelf, Library.NativeTagger("ES"), Downloads.workDir(context), onlyOurs = false)
        val first = organizer.run()
        println("first run: $first")
        assertEquals(1, first.organised)
        assertEquals(1, first.matched)
        assertEquals(0, first.failed)

        val item = shelf.list().single()
        println("now: ${item.folder}/${item.name}")
        assertEquals("Queen", item.folder)
        assertEquals("Queen - A Night At The Opera - Bohemian Rhapsody.mp3", item.name)
        val tags = metadata(Uri.parse(item.id))
        println("tags: $tags")
        assertEquals("Bohemian Rhapsody", tags["title"])
        assertEquals("Queen", tags["artist"])
        assertEquals("A Night At The Opera", tags["album"])
        assertEquals("1975", tags["year"])
        assertTrue("no cover: ${tags["cover"]}", (tags["cover"]?.toInt() ?: 0) > 10_000)
        assertTrue("audio damaged", (tags["duration"]?.toLong() ?: 0) > 350_000)

        val second = organizer.run()
        println("second run: $second")
        assertEquals(1, second.unchanged)
        assertEquals(0, second.organised)
    }

    /**
     * The whole service path for a new MP3: queue it the way the button does,
     * wait for the service to finish, and find it named, tagged and filed.
     */
    @Test
    fun a_new_mp3_download_lands_tagged_in_its_artist_folder() {
        Settings.setAutoOrganise(context, true)
        val hit = Native.searchVideos("queen bohemian rhapsody official video", 10)
            .first { (it.duration ?: 0) in 200..600 }
        println("downloading ${hit.title} (${hit.duration}s) by ${hit.channel}")
        val before = DownloadService.last.value
        val ahead = DownloadService.start(context, hit.id, hit.title, DownloadService.KIND_MP3)
        assertEquals(0, ahead)

        val deadline = System.currentTimeMillis() + 5 * 60_000
        while (DownloadService.last.value === before && System.currentTimeMillis() < deadline) {
            Thread.sleep(500)
        }
        val result = DownloadService.last.value
        assertNotNull("the service never finished", result)
        println("result: ${result!!.name}")
        assertNull("download failed: ${result.error}", result.error)
        assertEquals(0, DownloadService.queued.value)

        val item = Library.MediaStoreShelf(context).list().single()
        println("saved: ${item.folder}/${item.name}")
        assertEquals("Queen", item.folder)
        assertTrue(item.name, item.name.startsWith("Queen - ") && item.name.endsWith(" - Bohemian Rhapsody.mp3"))
        val tags = metadata(Uri.parse(item.id))
        println("tags: $tags")
        assertEquals("Bohemian Rhapsody", tags["title"])
        assertTrue("no album", !tags["album"].isNullOrEmpty())
        assertTrue("no year: ${tags["year"]}", !tags["year"].isNullOrEmpty())
        assertTrue("no cover", (tags["cover"]?.toInt() ?: 0) > 10_000)
    }
}
