package dev.lucasmoy.recpocket

import android.os.Build
import androidx.test.core.app.ActivityScenario
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The app opens, and its defaults are the ones it promises.
 *
 * The first half exists because of a real crash in a sibling app: a Compose
 * screen that throws during composition dies before drawing anything, and
 * the only symptom on a phone is the launcher icon flashing. A test that
 * composes the activity catches exactly that class of mistake without a
 * device.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [Build.VERSION_CODES.UPSIDE_DOWN_CAKE])
class ActivityOpensTest {
    @Test
    fun `the settings screen composes`() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity -> assertNotNull(activity) }
        }
    }

    @Test
    fun `the defaults are small files, no screen, nothing automatic`() {
        val context = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
        val settings = Settings(context)
        // "Compressed, acceptable quality, for keeping calls" is the stated
        // purpose, so it is what a fresh install does.
        assertEquals(Settings.AudioQuality.VOICE, settings.audioQuality)
        assertEquals(Settings.VideoQuality.LOW, settings.videoQuality)
        assertEquals(Settings.Sources.MIC, settings.sources)
        // Recording the screen, and recording on its own, are both things
        // the user has to ask for.
        assertEquals(false, settings.screen)
        assertEquals(false, settings.callTrigger)
    }

    @Test
    fun `settings survive being read back`() {
        val context = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
        val settings = Settings(context)
        settings.sources = Settings.Sources.BOTH
        settings.screen = true
        settings.audioQuality = Settings.AudioQuality.HIGH
        settings.callTrigger = true

        val reread = Settings(context)
        assertEquals(Settings.Sources.BOTH, reread.sources)
        assertEquals(true, reread.screen)
        assertEquals(Settings.AudioQuality.HIGH, reread.audioQuality)
        assertEquals(true, reread.callTrigger)
    }

    @Test
    fun `an unreadable stored value falls back instead of crashing`() {
        // A renamed enum constant (or a downgrade) must not stop the app
        // from starting -- this is read from a service that may be launched
        // by a notification with no UI to report an error to.
        val context = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
        context.getSharedPreferences("recpocket", android.content.Context.MODE_PRIVATE)
            .edit()
            .putString("audioQuality", "SOMETHING_ELSE")
            .putString("sources", "GONE")
            .apply()
        val settings = Settings(context)
        assertEquals(Settings.AudioQuality.VOICE, settings.audioQuality)
        assertEquals(Settings.Sources.MIC, settings.sources)
    }
}
