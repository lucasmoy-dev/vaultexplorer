package dev.lucasmoy.ytpocket

import androidx.test.core.app.ActivityScenario
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The app opens.
 *
 * This is the test that should have existed before 0.1.0 shipped: that build
 * closed itself the instant it was tapped, because a Compose
 * `ActivityResultLauncher` was launched from inside the composition that
 * registered it (which throws "Attempting to launch an unregistered
 * ActivityResultLauncher"). No unit test built an Activity, and there was no
 * device to run on, so nothing caught it.
 *
 * Robolectric builds the real Activity on the JVM and runs its lifecycle, so
 * anything that throws while the screen starts up fails here. Verified
 * against the bug: with the old `remember { launcher.launch(...) }` in place,
 * this test fails.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ActivityOpensTest {

    @Test
    fun `the screen composes without throwing`() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity -> assertNotNull(activity) }
        }
    }
}
