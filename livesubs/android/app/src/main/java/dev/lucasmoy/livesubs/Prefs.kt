package dev.lucasmoy.livesubs

import android.content.Context
import android.graphics.Color
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Everything the user can change. Deliberately the same field set as the
 * desktop app's `settings.rs`, down to the defaults, so the two apps behave
 * the same and a setting means the same thing on both.
 *
 * Phone-specific departures from the desktop defaults, and why:
 * - `widthPercent` 96: a phone screen is narrow, and 80% of it wastes room
 *   a subtitle needs.
 * - `fontSize` 18sp: measured in sp (so it follows the system font scale),
 *   where the desktop counts pixels.
 * - `model` "base": tiny is noticeably worse in Spanish and French, and
 *   base still keeps up on an arm64 phone for utterance-sized chunks.
 */
data class Settings(
    val captureMic: Boolean = true,
    val captureSystem: Boolean = true,
    val model: String = "base",
    /** "auto", or a fixed "en"/"es"/"fr". */
    val sourceLanguage: String = "auto",
    /** "off", or the language every subtitle should end up in. */
    val targetLanguage: String = "off",
    val showOriginal: Boolean = false,
    val sensitivity: Float = 1f,
    /** "bottom" | "center" | "top" */
    val anchor: String = "bottom",
    val margin: Int = 96,
    val widthPercent: Int = 96,
    val fontSize: Int = 18,
    val backgroundColor: Int = Color.BLACK,
    val backgroundOpacity: Float = 0.62f,
    val micColor: Int = 0xFF7AD7FF.toInt(),
    val systemColor: Int = Color.WHITE,
    val maxLines: Int = 2,
    val hideAfterMs: Int = 6000,
    val logEnabled: Boolean = false,
    /** A SAF document Uri, as a string, or null when nothing was picked. */
    val logUri: String? = null,
    val paused: Boolean = false,
) {
    /** The plate colour with the opacity slider applied. */
    val plateColor: Int
        get() = (backgroundColor and 0x00FFFFFF) or
            ((backgroundOpacity.coerceIn(0f, 1f) * 255).toInt() shl 24)

    /** What to pass to whisper: null means "detect it". */
    val whisperLanguage: String?
        get() = sourceLanguage.takeIf { it != "auto" }
}

/**
 * The settings store: SharedPreferences underneath, a `StateFlow` on top so
 * the service and the settings screen react to the same value instead of
 * each reading the file at different times.
 *
 * A singleton because the service and the UI are in one process and must
 * not hold two copies of this -- the desktop app has the same rule, for the
 * same reason (see the `SettingsState` it manages once).
 */
class Prefs private constructor(context: Context) {
    private val store = context.applicationContext.getSharedPreferences("livesubs", Context.MODE_PRIVATE)
    private val state = MutableStateFlow(read())

    val settings: StateFlow<Settings> get() = state
    val current: Settings get() = state.value

    private fun read(): Settings {
        val defaults = Settings()
        return Settings(
            captureMic = store.getBoolean(KEY_MIC, defaults.captureMic),
            captureSystem = store.getBoolean(KEY_SYSTEM, defaults.captureSystem),
            model = store.getString(KEY_MODEL, defaults.model) ?: defaults.model,
            sourceLanguage = store.getString(KEY_SOURCE_LANG, defaults.sourceLanguage) ?: defaults.sourceLanguage,
            targetLanguage = store.getString(KEY_TARGET_LANG, defaults.targetLanguage) ?: defaults.targetLanguage,
            showOriginal = store.getBoolean(KEY_SHOW_ORIGINAL, defaults.showOriginal),
            sensitivity = store.getFloat(KEY_SENSITIVITY, defaults.sensitivity),
            anchor = store.getString(KEY_ANCHOR, defaults.anchor) ?: defaults.anchor,
            margin = store.getInt(KEY_MARGIN, defaults.margin),
            widthPercent = store.getInt(KEY_WIDTH, defaults.widthPercent),
            fontSize = store.getInt(KEY_FONT_SIZE, defaults.fontSize),
            backgroundColor = store.getInt(KEY_BG_COLOR, defaults.backgroundColor),
            backgroundOpacity = store.getFloat(KEY_BG_OPACITY, defaults.backgroundOpacity),
            micColor = store.getInt(KEY_MIC_COLOR, defaults.micColor),
            systemColor = store.getInt(KEY_SYSTEM_COLOR, defaults.systemColor),
            maxLines = store.getInt(KEY_MAX_LINES, defaults.maxLines),
            hideAfterMs = store.getInt(KEY_HIDE_AFTER, defaults.hideAfterMs),
            logEnabled = store.getBoolean(KEY_LOG_ENABLED, defaults.logEnabled),
            logUri = store.getString(KEY_LOG_URI, null),
            paused = store.getBoolean(KEY_PAUSED, defaults.paused),
        )
    }

    fun update(transform: (Settings) -> Settings) {
        val next = transform(state.value)
        store.edit().apply {
            putBoolean(KEY_MIC, next.captureMic)
            putBoolean(KEY_SYSTEM, next.captureSystem)
            putString(KEY_MODEL, next.model)
            putString(KEY_SOURCE_LANG, next.sourceLanguage)
            putString(KEY_TARGET_LANG, next.targetLanguage)
            putBoolean(KEY_SHOW_ORIGINAL, next.showOriginal)
            putFloat(KEY_SENSITIVITY, next.sensitivity)
            putString(KEY_ANCHOR, next.anchor)
            putInt(KEY_MARGIN, next.margin)
            putInt(KEY_WIDTH, next.widthPercent)
            putInt(KEY_FONT_SIZE, next.fontSize)
            putInt(KEY_BG_COLOR, next.backgroundColor)
            putFloat(KEY_BG_OPACITY, next.backgroundOpacity)
            putInt(KEY_MIC_COLOR, next.micColor)
            putInt(KEY_SYSTEM_COLOR, next.systemColor)
            putInt(KEY_MAX_LINES, next.maxLines)
            putInt(KEY_HIDE_AFTER, next.hideAfterMs)
            putBoolean(KEY_LOG_ENABLED, next.logEnabled)
            putString(KEY_LOG_URI, next.logUri)
            putBoolean(KEY_PAUSED, next.paused)
        }.apply()
        state.value = next
    }

    companion object {
        private const val KEY_MIC = "capture_mic"
        private const val KEY_SYSTEM = "capture_system"
        private const val KEY_MODEL = "model"
        private const val KEY_SOURCE_LANG = "source_language"
        private const val KEY_TARGET_LANG = "target_language"
        private const val KEY_SHOW_ORIGINAL = "show_original"
        private const val KEY_SENSITIVITY = "sensitivity"
        private const val KEY_ANCHOR = "anchor"
        private const val KEY_MARGIN = "margin"
        private const val KEY_WIDTH = "width_percent"
        private const val KEY_FONT_SIZE = "font_size"
        private const val KEY_BG_COLOR = "background_color"
        private const val KEY_BG_OPACITY = "background_opacity"
        private const val KEY_MIC_COLOR = "mic_color"
        private const val KEY_SYSTEM_COLOR = "system_color"
        private const val KEY_MAX_LINES = "max_lines"
        private const val KEY_HIDE_AFTER = "hide_after_ms"
        private const val KEY_LOG_ENABLED = "log_enabled"
        private const val KEY_LOG_URI = "log_uri"
        private const val KEY_PAUSED = "paused"

        @Volatile
        private var instance: Prefs? = null

        fun get(context: Context): Prefs =
            instance ?: synchronized(this) {
                instance ?: Prefs(context).also { instance = it }
            }

        /** Fields whose change means capture has to be torn down and restarted. */
        fun needsRestart(old: Settings, new: Settings): Boolean =
            old.captureMic != new.captureMic ||
                old.captureSystem != new.captureSystem ||
                old.model != new.model ||
                old.sourceLanguage != new.sourceLanguage ||
                old.paused != new.paused
    }
}
