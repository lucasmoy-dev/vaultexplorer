package dev.lucasmoy.recpocket

/**
 * Deciding, from a notification, that a WhatsApp call is happening -- and
 * whether it has video.
 *
 * There is no API for this. WhatsApp does not broadcast its calls, and
 * Android has not let an app watch the telephony state of another app since
 * forever. What *is* observable is the notification a call posts, which a
 * [android.service.notification.NotificationListenerService] can read: an
 * ongoing call notification exists for as long as the call does, and its
 * text says which kind it is.
 *
 * So this is text matching, and it is honest about that:
 *
 *  * The **package** must be WhatsApp (or WhatsApp Business).
 *  * The wording is matched in several languages, because a phone set to
 *    Spanish says "Llamada de voz en curso" and one set to English says
 *    "Ongoing voice call". Matching on a substring of each is far more
 *    robust than trying to know every full string.
 *  * "Video" is the discriminator, and it is spelled almost identically
 *    everywhere (`video`, `vídeo`, `videochamada`, `videoanruf`), which is
 *    why accents are stripped before matching.
 *
 * A missed match means a call that is not recorded, never a wrong file: the
 * only action taken on a match is starting a recording the user asked for.
 */
object CallSignals {
    val WHATSAPP_PACKAGES = setOf("com.whatsapp", "com.whatsapp.w4b")

    enum class Call { NONE, VOICE, VIDEO }

    /** Words that mean "a call is going on" in the languages this phone
     *  might be set to. */
    private val CALL_WORDS = listOf(
        "call", "llamada", "llamando", "chamada", "appel", "anruf", "chiamata",
    )

    /** Words that mean it is a video call. */
    private val VIDEO_WORDS = listOf("video", "videollamada", "videochamada", "videoanruf")

    /** Words that mean it is *not* a live call: a missed one, a message. */
    private val NOT_A_CALL_WORDS = listOf(
        "perdida", "missed", "perdue", "verpasst", "persa", "perdida", "mensaje", "message",
        "mensagem", "nachricht", "messaggio",
    )

    /**
     * What this notification says is happening.
     *
     * `text` should be everything the notification shows -- title, body,
     * subtext, channel name -- joined together; more words can only help.
     */
    fun classify(packageName: String, text: String, isOngoing: Boolean = true): Call {
        if (packageName !in WHATSAPP_PACKAGES) return Call.NONE
        val flat = flatten(text)
        if (flat.isBlank()) return Call.NONE
        // A missed call posts a notification too, and recording after the
        // fact records nothing but silence.
        if (NOT_A_CALL_WORDS.any { flat.contains(it) }) return Call.NONE
        if (!isOngoing) return Call.NONE
        if (CALL_WORDS.none { flat.contains(it) } && VIDEO_WORDS.none { flat.contains(it) }) {
            return Call.NONE
        }
        return if (VIDEO_WORDS.any { flat.contains(it) }) Call.VIDEO else Call.VOICE
    }

    /** Lower case, accents removed, so one pattern matches every locale's
     *  spelling of the same word. */
    fun flatten(text: String): String {
        val lower = text.lowercase()
        val builder = StringBuilder(lower.length)
        for (c in lower) {
            builder.append(
                when (c) {
                    'á', 'à', 'ä', 'â', 'ã' -> 'a'
                    'é', 'è', 'ë', 'ê' -> 'e'
                    'í', 'ì', 'ï', 'î' -> 'i'
                    'ó', 'ò', 'ö', 'ô', 'õ' -> 'o'
                    'ú', 'ù', 'ü', 'û' -> 'u'
                    'ç' -> 'c'
                    'ñ' -> 'n'
                    else -> c
                }
            )
        }
        return builder.toString()
    }
}
