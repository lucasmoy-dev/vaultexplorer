package dev.lucasmoy.recpocket

import android.content.Context

/**
 * What to record, how small it should end up, and when to start on its own.
 *
 * Kept in `SharedPreferences` because every one of these has to be readable
 * from a service that may be started by a notification at 3am with no
 * activity alive -- there is nothing to pass state through.
 *
 * The defaults are the stated purpose of the app: "compressed, acceptable
 * quality, for keeping calls". That is [AudioQuality.VOICE] and
 * [VideoQuality.LOW] -- roughly 15MB an hour for audio, and a video call
 * that fits in a phone rather than filling it.
 */
class Settings(context: Context) {
    private val prefs = context.getSharedPreferences("recpocket", Context.MODE_PRIVATE)

    /** Which audio goes into the file. */
    enum class Sources {
        /** The microphone: your side of a call, the room. */
        MIC,

        /** What the phone is playing: the video, the meeting, the far end
         *  of a call *when the other app allows it* (see README). */
        PLAYBACK,

        /** Both, mixed into one track. */
        BOTH,
    }

    enum class AudioQuality(val label: String, val sampleRate: Int, val channels: Int, val bitrate: Int) {
        // 32 kbps mono at 22.05kHz: speech stays perfectly intelligible and
        // an hour costs about 14MB.
        VOICE("Voz (chica)", 22_050, 1, 32_000),
        NORMAL("Normal", 44_100, 1, 64_000),
        HIGH("Alta (música)", 44_100, 2, 160_000),
    }

    enum class VideoQuality(val label: String, val maxHeight: Int, val bitrate: Int, val fps: Int) {
        LOW("Baja (480p)", 480, 1_200_000, 24),
        NORMAL("Normal (720p)", 720, 2_500_000, 30),
        HIGH("Alta (1080p)", 1080, 5_000_000, 30),
    }

    var sources: Sources
        get() = enumValue(prefs.getString("sources", null), Sources.MIC)
        set(value) = prefs.edit().putString("sources", value.name).apply()

    /** Record the screen as well, not only audio. */
    var screen: Boolean
        get() = prefs.getBoolean("screen", false)
        set(value) = prefs.edit().putBoolean("screen", value).apply()

    var audioQuality: AudioQuality
        get() = enumValue(prefs.getString("audioQuality", null), AudioQuality.VOICE)
        set(value) = prefs.edit().putString("audioQuality", value.name).apply()

    var videoQuality: VideoQuality
        get() = enumValue(prefs.getString("videoQuality", null), VideoQuality.LOW)
        set(value) = prefs.edit().putString("videoQuality", value.name).apply()

    /** Start recording by itself when a WhatsApp call begins. */
    var callTrigger: Boolean
        get() = prefs.getBoolean("callTrigger", false)
        set(value) = prefs.edit().putBoolean("callTrigger", value).apply()

    /** Show the floating button, so recording can be started and stopped
     *  without coming back to the app. */
    var overlay: Boolean
        get() = prefs.getBoolean("overlay", true)
        set(value) = prefs.edit().putBoolean("overlay", value).apply()

    private inline fun <reified T : Enum<T>> enumValue(stored: String?, fallback: T): T =
        runCatching { enumValueOf<T>(stored ?: return fallback) }.getOrDefault(fallback)
}
