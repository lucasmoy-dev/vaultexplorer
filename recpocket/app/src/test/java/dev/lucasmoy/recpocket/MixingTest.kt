package dev.lucasmoy.recpocket

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Adding two audio streams together, which is the only way to record "both
 * sides" at once (see [Recorder]). Every case here was an audible bug in a
 * naive version of this code: overflow crackle, and one side being cut off
 * because the other stream's read was shorter.
 */
class MixingTest {
    private fun pcm(vararg samples: Int): ByteArray {
        val bytes = ByteArray(samples.size * 2)
        samples.forEachIndexed { i, value -> Mixing.writeSample(bytes, i * 2, value) }
        return bytes
    }

    private fun samplesOf(bytes: ByteArray): List<Int> =
        (0 until bytes.size / 2).map { Mixing.sampleAt(bytes, it * 2) }

    @Test
    fun `a sample survives a round trip, negatives included`() {
        val bytes = pcm(0, 1, -1, 32767, -32768, 1234, -4321)
        assertEquals(listOf(0, 1, -1, 32767, -32768, 1234, -4321), samplesOf(bytes))
    }

    @Test
    fun `mixing adds the two streams`() {
        assertEquals(listOf(150, -150), samplesOf(Mixing.mix(pcm(100, -100), 4, pcm(50, -50), 4)))
    }

    @Test
    fun `loud passages saturate instead of wrapping round`() {
        // Left to overflow, 30000 + 20000 becomes a large negative number,
        // which is heard as a crack rather than as loudness.
        val mixed = Mixing.mix(pcm(30_000, -30_000), 4, pcm(20_000, -20_000), 4)
        assertEquals(listOf(32_767, -32_768), samplesOf(mixed))
    }

    @Test
    fun `the shorter stream is treated as silence, not as an end`() {
        // The mic read three samples, playback only one: the other two mic
        // samples must still reach the file.
        assertEquals(listOf(15, 20, 30), samplesOf(Mixing.mix(pcm(10, 20, 30), 6, pcm(5), 2)))
        assertEquals(listOf(8, 2, 3), samplesOf(Mixing.mix(pcm(7), 2, pcm(1, 2, 3), 6)))
    }

    @Test
    fun `gain scales only the second stream`() {
        // Playback capture arrives far hotter than a microphone, so the far
        // end of a call would otherwise bury the near one.
        assertEquals(listOf(1500), samplesOf(Mixing.mix(pcm(1000), 2, pcm(1000), 2, gainB = 0.5f)))
    }

    @Test
    fun `a trailing half sample is left for the next read`() {
        // An AudioRecord read can end mid-sample; using that byte would
        // shift every following sample and turn the track into noise.
        assertEquals(listOf(101, 1), samplesOf(Mixing.mix(pcm(100, 200), 3, pcm(1, 1), 4)))
    }
}
