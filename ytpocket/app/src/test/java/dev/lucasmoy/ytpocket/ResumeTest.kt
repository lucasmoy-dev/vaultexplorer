package dev.lucasmoy.ytpocket

import java.io.File
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * The retry loop in [Downloads.fetch], which is the part of the 403 fix that
 * runs on the phone.
 *
 * Worth a test of its own because the failure it handles cannot be provoked
 * on demand: googlevideo refuses a transfer part way through when the address
 * the URL was signed for stops being the address we are calling from (a phone
 * rotating its IPv6 privacy address). The native side is checked against real
 * YouTube in `jni`; what those live tests cannot check is that *this* loop
 * resumes rather than restarts, respects its own retry ceiling, and refuses
 * to pass off a truncated file as a finished one. So the chunk fetcher is
 * injected and made to refuse on cue.
 */
class ResumeTest {
    private val total = 10L * 1024 * 1024
    private val chunk = 4L * 1024 * 1024

    /** Records what was asked for, and writes the bytes a real fetch would. */
    private class Recorder(val refuseAt: MutableList<Long>) : Downloads.ChunkFetcher {
        val offsets = mutableListOf<Long>()
        val agents = mutableListOf<String>()
        val urls = mutableListOf<String>()

        override fun fetch(
            url: String,
            path: String,
            offset: Long,
            length: Long,
            userAgent: String,
        ): Long {
            offsets += offset
            urls += url
            agents += userAgent
            if (refuseAt.remove(offset)) throw IllegalStateException("YouTube rechazó (403)")
            File(path).appendBytes(ByteArray(length.toInt()))
            return length
        }
    }

    private fun target() = File.createTempFile("resume", ".bin").also { it.delete() }

    @Test
    fun `a refusal part way through continues instead of starting over`() {
        val file = target()
        // Refused once at the second chunk -- the shape of the real failure.
        val fetcher = Recorder(mutableListOf(chunk))
        var refreshes = 0
        val progress = mutableListOf<Float>()

        Downloads.fetch(
            url = "https://first/stream",
            target = file,
            userAgent = "agent-one",
            expectedSize = total,
            onProgress = { progress += it },
            refresh = { refreshes++; "https://fresh/stream" to "agent-two" },
            fetcher = fetcher,
            sizeOf = { _, _ -> total },
        )

        assertEquals("resolved again exactly once", 1, refreshes)
        // The retried chunk starts where it stopped, not at zero: the 4MB that
        // already landed are not downloaded twice.
        assertArrayEquals(
            longArrayOf(0, chunk, chunk, 2 * chunk),
            fetcher.offsets.toLongArray(),
        )
        // And it retries with the *fresh* URL and its agent, not the stale one.
        assertEquals("https://first/stream", fetcher.urls[0])
        assertEquals("https://fresh/stream", fetcher.urls[2])
        assertEquals("agent-two", fetcher.agents[2])
        assertEquals("the whole file, once", total, file.length())
        assertEquals("finished", 1f, progress.last(), 0f)
        file.delete()
    }

    @Test
    fun `a network that refuses everything ends in the refusal, not a loop`() {
        val file = target()
        val fetcher = object : Downloads.ChunkFetcher {
            var calls = 0
            override fun fetch(u: String, p: String, o: Long, l: Long, a: String): Long {
                calls++
                throw IllegalStateException("YouTube rechazó (403)")
            }
        }
        var refreshes = 0
        try {
            Downloads.fetch("https://s", file, "agent", total, {},
                refresh = { refreshes++; "https://fresh" to "agent" },
                fetcher = fetcher, sizeOf = { _, _ -> total })
            fail("expected the refusal to surface")
        } catch (expected: IllegalStateException) {
            assertTrue(expected.message!!.contains("403"))
        }
        assertEquals("bounded number of retries", 6, refreshes)
        assertEquals("one attempt, then one per refresh", 7, fetcher.calls)
        file.delete()
    }

    @Test
    fun `with no way to re-resolve, the refusal surfaces immediately`() {
        val file = target()
        val fetcher = Recorder(mutableListOf(0L))
        try {
            Downloads.fetch("https://s", file, "agent", total, {},
                refresh = null, fetcher = fetcher, sizeOf = { _, _ -> total })
            fail("expected the refusal to surface")
        } catch (expected: IllegalStateException) {
            assertTrue(expected.message!!.contains("403"))
        }
        assertEquals(1, fetcher.offsets.size)
        file.delete()
    }

    @Test
    fun `a transfer that stops short is an error, not a short file`() {
        val file = target()
        // Answers the first chunk and then nothing, the way a cut connection
        // looks: no exception, no bytes.
        val fetcher = Downloads.ChunkFetcher { _, path, _, length, _ ->
            if (File(path).length() == 0L) {
                File(path).appendBytes(ByteArray(length.toInt())); length
            } else 0L
        }
        try {
            Downloads.fetch("https://s", file, "agent", total, {},
                fetcher = fetcher, sizeOf = { _, _ -> total })
            fail("expected an incomplete download to be rejected")
        } catch (expected: IllegalStateException) {
            assertTrue(expected.message!!.contains("incompleta"))
        }
        file.delete()
    }

    @Test
    fun `a download that brings nothing back is an error`() {
        val file = target()
        try {
            Downloads.fetch("https://s", file, "agent", total, {},
                fetcher = { _, _, _, _, _ -> 0L }, sizeOf = { _, _ -> total })
            fail("expected an empty download to be rejected")
        } catch (expected: IllegalStateException) {
            assertTrue(expected.message!!.contains("vacía"))
        }
        file.delete()
    }

    @Test
    fun `a leftover partial file is not downloaded on top of`() {
        val file = target()
        file.writeBytes(ByteArray(1234))
        val fetcher = Recorder(mutableListOf())
        Downloads.fetch("https://s", file, "agent", total, {},
            fetcher = fetcher, sizeOf = { _, _ -> total })
        // Exactly the stream's size: the stale 1234 bytes were dropped, not
        // prepended to it.
        assertEquals(total, file.length())
        file.delete()
    }

    @Test
    fun `an unknown size still downloads, and reports incomplete only when known`() {
        val file = target()
        var served = 0
        // No Content-Length available (sizeOf fails), so the loop runs until a
        // request brings nothing back.
        val fetcher = Downloads.ChunkFetcher { _, path, _, length, _ ->
            if (served++ < 2) { File(path).appendBytes(ByteArray(length.toInt())); length } else 0L
        }
        Downloads.fetch("https://s", file, "agent", 0L,
            {}, fetcher = fetcher, sizeOf = { _, _ -> throw RuntimeException("no size") })
        assertEquals(2 * chunk, file.length())
        file.delete()
    }
}
