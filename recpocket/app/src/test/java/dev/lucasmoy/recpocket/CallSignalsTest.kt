package dev.lucasmoy.recpocket

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Reading a notification to decide that a WhatsApp call is happening.
 *
 * This is text matching against another app's wording, so it is exactly the
 * kind of code that rots quietly -- these are the wordings WhatsApp posts in
 * the languages this phone might be set to, plus the ones that must *not*
 * trigger: a missed call (recording it captures silence), a message, and any
 * other app.
 */
class CallSignalsTest {
    private fun classify(text: String, pkg: String = "com.whatsapp", ongoing: Boolean = true) =
        CallSignals.classify(pkg, text, ongoing)

    @Test
    fun `an ongoing voice call is a voice call`() {
        assertEquals(CallSignals.Call.VOICE, classify("Llamada de voz en curso"))
        assertEquals(CallSignals.Call.VOICE, classify("Ongoing voice call"))
        assertEquals(CallSignals.Call.VOICE, classify("Appel vocal en cours"))
        assertEquals(CallSignals.Call.VOICE, classify("Chamada de voz em curso"))
        assertEquals(CallSignals.Call.VOICE, classify("Laufender Anruf"))
    }

    @Test
    fun `video is recognised however it is spelled`() {
        assertEquals(CallSignals.Call.VIDEO, classify("Videollamada en curso"))
        assertEquals(CallSignals.Call.VIDEO, classify("Ongoing video call"))
        assertEquals(CallSignals.Call.VIDEO, classify("Videochamada em curso"))
        assertEquals(CallSignals.Call.VIDEO, classify("Videoanruf"))
        // Accented spellings are the normal case in Spanish and Portuguese.
        assertEquals(CallSignals.Call.VIDEO, classify("Vídeollamada en curso"))
    }

    @Test
    fun `a missed call is not a call to record`() {
        assertEquals(CallSignals.Call.NONE, classify("Llamada perdida"))
        assertEquals(CallSignals.Call.NONE, classify("Missed voice call"))
        assertEquals(CallSignals.Call.NONE, classify("2 missed video calls"))
    }

    @Test
    fun `messages and other apps are ignored`() {
        assertEquals(CallSignals.Call.NONE, classify("Juan: mensaje nuevo"))
        assertEquals(CallSignals.Call.NONE, classify("3 mensajes de Ana"))
        assertEquals(CallSignals.Call.NONE, classify("Ongoing voice call", pkg = "org.telegram.messenger"))
        assertEquals(CallSignals.Call.NONE, classify(""))
    }

    @Test
    fun `whatsapp business counts, and a banner that is not ongoing does not`() {
        assertEquals(CallSignals.Call.VOICE, classify("Ongoing call", pkg = "com.whatsapp.w4b"))
        // An incoming-call banner is not yet a call: recording would start
        // before it was answered, and stop when the banner is replaced.
        assertEquals(CallSignals.Call.NONE, classify("Incoming voice call", ongoing = false))
    }

    @Test
    fun `accents are flattened so one pattern matches every locale`() {
        assertEquals("videollamada", CallSignals.flatten("Vídeollamada"))
        assertEquals("chamada", CallSignals.flatten("Chamada"))
        assertEquals("anruf", CallSignals.flatten("Anruf"))
    }
}
