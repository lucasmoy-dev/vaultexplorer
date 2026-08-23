package dev.lucasmoy.recpocket

import java.util.TimeZone
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * File names. Worth testing because the whole point of the reverse date is
 * that the names sort chronologically -- a format that is merely "readable"
 * loses that, and nobody notices until the folder has two hundred files.
 */
class NamingTest {
    private val madrid = TimeZone.getTimeZone("Europe/Madrid")

    @Test
    fun `single digits keep their width`() {
        // 2026-01-02 03:04:05 UTC.
        assertEquals("2026-01-02_03-04-05", Naming.stamp(1767323045000L, TimeZone.getTimeZone("UTC")))
    }

    @Test
    fun `names sort in the order they were recorded`() {
        val hour = 3_600_000L
        val base = 1767323045000L
        val shuffled = listOf(base + 2 * hour, base, base + hour)
            .map { Naming.fileName(Naming.Kind.CALL_AUDIO, it, zone = madrid) }
        val chronological = listOf(base, base + hour, base + 2 * hour)
            .map { Naming.fileName(Naming.Kind.CALL_AUDIO, it, zone = madrid) }
        // The claim the whole convention rests on: sorting the strings gives
        // chronological order.
        assertEquals(chronological, shuffled.sorted())
    }

    @Test
    fun `the name says what kind of recording it is`() {
        val millis = 1767323045000L
        val stamp = Naming.stamp(millis, madrid)
        assertEquals("$stamp llamada.m4a", Naming.fileName(Naming.Kind.CALL_AUDIO, millis, zone = madrid))
        assertEquals("$stamp videollamada.mp4", Naming.fileName(Naming.Kind.CALL_VIDEO, millis, zone = madrid))
        assertEquals("$stamp captura.jpg", Naming.fileName(Naming.Kind.SCREENSHOT, millis, zone = madrid))
        assertEquals("$stamp pantalla.mp4", Naming.fileName(Naming.Kind.SCREEN, millis, zone = madrid))
        assertTrue(
            Naming.fileName(Naming.Kind.MIC, millis, note = "reunión", zone = madrid)
                .endsWith("voz reunión.m4a")
        )
    }

    @Test
    fun `the stamp follows the phone timezone`() {
        val millis = 1767323045000L
        assertEquals("2026-01-02_03-04-05", Naming.stamp(millis, TimeZone.getTimeZone("UTC")))
        assertEquals("2026-01-02_04-04-05", Naming.stamp(millis, madrid))
    }

    @Test
    fun `without the capture permission a recording is named for what it is`() {
        // A video call recorded with no screen-capture permission is a voice
        // recording: naming it .mp4 would produce a video file with no video
        // track in it.
        with(Naming) {
            assertEquals(Naming.Kind.CALL_AUDIO, Naming.Kind.CALL_VIDEO.audioOnly())
            assertEquals(Naming.Kind.MIC, Naming.Kind.SCREEN.audioOnly())
            // The audio-only kinds are already what they claim to be.
            assertEquals(Naming.Kind.BOTH, Naming.Kind.BOTH.audioOnly())
            assertEquals(Naming.Kind.CALL_AUDIO, Naming.Kind.CALL_AUDIO.audioOnly())
        }
    }

    @Test
    fun `a note cannot break the file name`() {
        // Slashes become dashes rather than vanishing: "AC/DC" should read
        // "AC-DC", not "ACDC".
        assertEquals("AC-DC", Naming.sanitize("AC/DC"))
        assertEquals("a b", Naming.sanitize("a \tb"))
        assertEquals("cliente - Juan", Naming.sanitize("  cliente : Juan  "))
        assertEquals("", Naming.sanitize("   "))
        val name = Naming.fileName(Naming.Kind.MIC, 1767323045000L, note = "a/b:c", zone = madrid)
        assertEquals(0, name.count { it == '/' })
    }
}
