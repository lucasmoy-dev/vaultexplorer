package dev.lucasmoy.livesubs

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Plain JVM tests for the parts that are pure logic: what the overlay plate
 * colour ends up being, when capture has to restart, and what a caption
 * coming back from the native side parses into. No device involved -- the
 * device-side behaviour is the service, and the recognition itself is
 * covered by the Rust core's own tests plus the JNI harness (see the
 * README).
 */
class SettingsTest {

    @Test
    fun `opacity is folded into the plate colour`() {
        val opaque = Settings(backgroundColor = 0xFF000000.toInt(), backgroundOpacity = 1f)
        assertEquals(0xFF000000.toInt(), opaque.plateColor)

        val half = Settings(backgroundColor = 0xFF102030.toInt(), backgroundOpacity = 0.5f)
        assertEquals(0x7F102030, half.plateColor)

        // Fully transparent plate: the text shadow is what keeps it legible,
        // and the colour must not fall back to opaque.
        val clear = Settings(backgroundOpacity = 0f)
        assertEquals(0x00000000, clear.plateColor)
    }

    @Test
    fun `out of range opacity is clamped rather than wrapping around`() {
        assertEquals(0xFF000000.toInt(), Settings(backgroundOpacity = 4f).plateColor)
        assertEquals(0x00000000, Settings(backgroundOpacity = -2f).plateColor)
    }

    @Test
    fun `auto means whisper detects the language`() {
        assertNull(Settings(sourceLanguage = "auto").whisperLanguage)
        assertEquals("fr", Settings(sourceLanguage = "fr").whisperLanguage)
    }

    @Test
    fun `only structural changes restart capture`() {
        val base = Settings()
        assertFalse(Prefs.needsRestart(base, base.copy(fontSize = 40)))
        assertFalse(Prefs.needsRestart(base, base.copy(micColor = 0xFF00FF00.toInt())))
        assertFalse(Prefs.needsRestart(base, base.copy(targetLanguage = "es")))
        assertFalse(Prefs.needsRestart(base, base.copy(logEnabled = true)))

        assertTrue(Prefs.needsRestart(base, base.copy(captureMic = false)))
        assertTrue(Prefs.needsRestart(base, base.copy(captureSystem = false)))
        assertTrue(Prefs.needsRestart(base, base.copy(model = "small")))
        assertTrue(Prefs.needsRestart(base, base.copy(sourceLanguage = "en")))
        assertTrue(Prefs.needsRestart(base, base.copy(paused = true)))
    }

    @Test
    fun `captions parse, and non-captions do not become empty lines`() {
        val parsed = CaptionJson.parse("""{"text":" Hola a todos ","language":"es"}""")
        assertEquals("Hola a todos", parsed?.text)
        assertEquals("es", parsed?.language)

        assertNull(CaptionJson.parse(null))
        assertNull(CaptionJson.parse(""))
        assertNull(CaptionJson.parse("""{"text":"   ","language":"es"}"""))
        assertNull(CaptionJson.parse("not json at all"))
    }
}
