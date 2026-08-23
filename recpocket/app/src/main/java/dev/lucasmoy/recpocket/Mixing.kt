package dev.lucasmoy.recpocket

/**
 * Putting two microphones' worth of sound into one track.
 *
 * Recording "both sides" means two independent [android.media.AudioRecord]
 * streams -- the microphone and (through `MediaProjection`) what the phone
 * is playing. There is no platform API that hands you the sum, so the
 * samples have to be added by hand, and the arithmetic has two traps:
 *
 *  * **Clipping.** Two 16-bit streams added together do not fit in 16 bits.
 *    Left to overflow, a loud passage wraps from +32767 round to -32768,
 *    which is heard as a violent crackle. Saturating at the limits sounds
 *    like a loud recording instead, which is what people expect.
 *  * **Different lengths.** The two streams are read separately and never
 *    line up exactly. The shorter one is treated as silence past its end
 *    rather than truncating the other, so a moment where only one side is
 *    talking is not cut out of the file.
 *
 * PCM 16-bit little-endian throughout, because that is what `AudioRecord`
 * produces and what the AAC encoder accepts.
 */
object Mixing {
    /** Read one little-endian 16-bit sample. */
    fun sampleAt(bytes: ByteArray, index: Int): Int {
        val low = bytes[index].toInt() and 0xff
        val high = bytes[index + 1].toInt()
        return (high shl 8) or low
    }

    fun writeSample(bytes: ByteArray, index: Int, value: Int) {
        bytes[index] = (value and 0xff).toByte()
        bytes[index + 1] = ((value shr 8) and 0xff).toByte()
    }

    /**
     * `a + b`, saturating, into a buffer as long as the longer input.
     *
     * `gainB` scales the second stream before adding: playback capture comes
     * in far louder than a microphone, and a call where the far end drowns
     * out your own voice is the usual complaint about mixed recordings.
     */
    fun mix(a: ByteArray, aLength: Int, b: ByteArray, bLength: Int, gainB: Float = 1f): ByteArray {
        // Whole samples only: a trailing odd byte is half a sample and
        // belongs to the next read.
        val aSamples = aLength / 2
        val bSamples = bLength / 2
        val samples = maxOf(aSamples, bSamples)
        val out = ByteArray(samples * 2)
        for (i in 0 until samples) {
            val left = if (i < aSamples) sampleAt(a, i * 2) else 0
            val right = if (i < bSamples) (sampleAt(b, i * 2) * gainB).toInt() else 0
            val sum = (left + right).coerceIn(-32_768, 32_767)
            writeSample(out, i * 2, sum)
        }
        return out
    }
}
