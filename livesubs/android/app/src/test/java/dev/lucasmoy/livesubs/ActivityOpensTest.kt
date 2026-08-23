package dev.lucasmoy.livesubs

import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The app opens.
 *
 * A test that sounds too obvious to write, and is here because the sibling
 * app shipped a version that closed itself the instant it was tapped: a
 * Compose `ActivityResultLauncher` launched from inside the composition that
 * registered it, which throws. Nothing else in this repo's test suite could
 * see that -- unit tests never build an Activity, and there was no device to
 * run on.
 *
 * Robolectric builds the real Activity on the JVM, runs its lifecycle and
 * composes the screen, so anything that throws during startup fails here.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ActivityOpensTest {

    @Test
    fun `the settings screen composes without throwing`() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity -> assertNotNull(activity) }
        }
    }

    @Test
    fun `preferences load with sane defaults`() {
        val prefs = Prefs.get(ApplicationProvider.getApplicationContext())
        val settings = prefs.current
        // Both sources on, nothing paused: the state someone expects after
        // installing and pressing start.
        assert(settings.captureMic)
        assert(settings.captureSystem)
        assert(!settings.paused)
    }
}
