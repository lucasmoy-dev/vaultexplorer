package dev.lucasmoy.ytpocket

import android.content.Context
import android.media.MediaMetadataRetriever
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The whole download, on a real Android runtime.
 *
 * Everything else in this project is checked either on the host (Rust tests,
 * JVM unit tests) or by inspection. This is the one test that runs the actual
 * cross-compiled native library, the platform's own muxer and `MediaStore`,
 * over the real network -- on an emulator, since the author has no phone
 * attached. It is what should have caught the 403 that three releases did
 * not: a *long, popular* video is chosen on purpose, because YouTube's bot
 * gate only trips on those, and the failure only appears past the first
 * chunk.
 *
 *     gradle :app:connectedDebugAndroidTest -PrustAbis=x86_64
 */
@RunWith(AndroidJUnit4::class)
class DownloadOnDeviceTest {
    private lateinit var context: Context

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        Native.prepare(context)
    }

    @Test
    fun a_long_popular_video_downloads_as_a_playable_mp3() {
        // Long and popular: exactly the shape that was failing. Anything
        // short or obscure passes even with the bug.
        val hits = Native.searchVideos("full album 1 hour", 20)
        val pick = hits
            .filter { (it.duration ?: 0) > 20 * 60 }
            .maxByOrNull { it.duration ?: 0 }
            ?: error("no long-enough search results")

        val resolved = Native.resolveVideo(pick.id)
        val audio = resolved.audio ?: error("no audio stream")
        // The token-free client is the one that can serve real chunks; if the
        // app fell back to a PO-token client the download would 403 below,
        // but naming it here says *why* when that happens.
        println("picked ${pick.title} (${pick.duration}s) via ${resolved.client}")

        val work = Downloads.workDir(context)
        val part = File(work, "test-audio.${audio.ext}")
        var lastFraction = 0f
        Downloads.fetch(audio.url, part, resolved.userAgent, audio.size, { lastFraction = it })

        assertEquals("progress finished", 1f, lastFraction, 0f)
        assertTrue("nothing downloaded", part.length() > 0)
        if (audio.size > 0) {
            assertEquals("short download", audio.size, part.length())
        }
        // Past one chunk, which is where every failed release died.
        assertTrue("suspiciously small: ${part.length()} bytes", part.length() > 8L * 1024 * 1024)

        val mp3 = File(work, "test-audio.mp3")
        Native.toMp3(part.absolutePath, mp3.absolutePath, pick.title, pick.channel)
        assertTrue("no mp3 produced", mp3.length() > 0)

        // Playable, and as long as YouTube said it was: proof the bytes are a
        // real continuous stream and not a truncated or spliced file.
        val meta = MediaMetadataRetriever()
        try {
            meta.setDataSource(mp3.absolutePath)
            val seconds =
                (meta.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)
                    ?.toLong() ?: 0L) / 1000
            println("mp3: ${mp3.length()} bytes, ${seconds}s (youtube said ${pick.duration}s)")
            val expected = (pick.duration ?: 0).toLong()
            assertTrue(
                "duration $seconds is not close to $expected",
                Math.abs(seconds - expected) <= 15,
            )
            assertEquals(pick.title, meta.extractMetadata(MediaMetadataRetriever.METADATA_KEY_TITLE))
        } finally {
            meta.release()
        }

        // And it reaches the place a music player looks.
        val uri = Downloads.publish(context, mp3, Native.nameFor(pick.title, "mp3"), audio = true)
        assertTrue("nothing published", uri.toString().isNotEmpty())
        context.contentResolver.openInputStream(uri)!!.use {
            assertTrue("published file is empty", it.available() > 0)
        }
        context.contentResolver.delete(uri, null, null)
        part.delete()
        mp3.delete()
    }
}
