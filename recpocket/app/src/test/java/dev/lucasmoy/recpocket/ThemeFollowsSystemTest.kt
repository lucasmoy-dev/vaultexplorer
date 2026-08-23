package dev.lucasmoy.recpocket

import android.graphics.Color
import android.os.Build
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The window follows the phone's light/dark setting.
 *
 * Compose paints the app, but the *window* behind it comes from the platform
 * theme -- and `Theme.DeviceDefault` has no DayNight flavour, so a single
 * theme means a white window flashing behind a dark app (or the reverse).
 * The fix is a `values-night` variant, and this is what proves it resolves:
 * the same theme id, read under each qualifier, has to give a light and a
 * dark background.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [Build.VERSION_CODES.UPSIDE_DOWN_CAKE])
class ThemeFollowsSystemTest {
    private fun windowBackgroundLuminance(): Double {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        context.setTheme(R.style.Theme_RecPocket)
        val attrs = intArrayOf(android.R.attr.colorBackground)
        val values = context.obtainStyledAttributes(attrs)
        val color = values.getColor(0, Color.MAGENTA)
        values.recycle()
        return (0.299 * Color.red(color) + 0.587 * Color.green(color) + 0.114 * Color.blue(color)) / 255.0
    }

    @Test
    @Config(qualifiers = "notnight")
    fun `in light mode the window is light`() {
        val light = windowBackgroundLuminance()
        assertTrue("window background luminance was $light", light > 0.7)
    }

    @Test
    @Config(qualifiers = "night")
    fun `in dark mode the window is dark`() {
        val dark = windowBackgroundLuminance()
        assertTrue("window background luminance was $dark", dark < 0.35)
    }
}
