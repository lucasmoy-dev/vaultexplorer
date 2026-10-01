package dev.lucasmoy.ytpocket

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import androidx.compose.ui.unit.width
import org.robolectric.RobolectricTestRunner
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The 0.2.0 screen changes, drawn and tapped on the JVM: buttons on the right
 * with MP3 last, the same style for both, a tap that visibly registers, and
 * the gear that opens the settings with the folder and music options.
 */
@RunWith(RobolectricTestRunner::class)
// A typical phone's width; Robolectric's default 320dp is narrower than any
// phone this runs on.
@Config(sdk = [34], qualifiers = "w411dp-h891dp")
class ResultRowTest {
    @get:Rule
    val compose = createComposeRule()

    private val hit = Native.Hit(
        id = "fJ9rUzIMcZQ",
        title = "Queen – Bohemian Rhapsody (Official Video Remastered)",
        channel = "Queen Official",
        duration = 359,
        views = 2_000_000_000,
        published = null,
        thumbnail = "",
    )

    @Test
    fun `mp3 is the right-most button and both look the same`() {
        compose.setContent { AppTheme { ResultRow(hit) { } } }
        val mp4 = compose.onNodeWithTag("download-mp4").getUnclippedBoundsInRoot()
        val mp3 = compose.onNodeWithTag("download-mp3").getUnclippedBoundsInRoot()
        assertTrue("MP3 must be right of MP4", mp3.left > mp4.right)
        assertEquals("same width", mp4.width, mp3.width)
        assertEquals("same height", mp4.height, mp3.height)
        // Against the right edge (card padding apart), not the left.
        val root = compose.onRoot().getUnclippedBoundsInRoot()
        assertTrue("MP3 is not at the right edge: ${mp3.right} of ${root.right}", root.right - mp3.right < 24.dp)
        assertTrue("buttons are on the left half", mp4.left > root.left + (root.right - root.left) / 3)
    }

    @Test
    fun `a tap queues the right kind and the button says so`() {
        val queued = mutableListOf<String>()
        compose.setContent { AppTheme { ResultRow(hit) { queued += it } } }
        compose.onNodeWithTag("download-mp3").performClick()
        assertEquals(listOf(DownloadService.KIND_MP3), queued)
        compose.onNodeWithText("En cola").assertIsDisplayed()
        // A second quick tap is the same download again: ignored.
        compose.onNodeWithTag("download-mp3").performClick()
        assertEquals(1, queued.size)
        // And it comes back after a moment.
        compose.mainClock.advanceTimeBy(2_500)
        compose.onNodeWithTag("download-mp3").performClick()
        assertEquals(2, queued.size)
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SettingsOpenTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    @Test
    fun `the gear opens the settings with folders and the organiser`() {
        compose.onNodeWithText("Opciones").assertDoesNotExist()
        compose.onNodeWithContentDescription("Opciones").performClick()
        compose.onNodeWithText("Carpeta para MP3").assertIsDisplayed()
        compose.onNodeWithText("Carpeta para MP4").assertIsDisplayed()
        compose.onNodeWithText("Música/YT Pocket (por defecto)").assertIsDisplayed()
        compose.onNodeWithText("Ordenar mis MP3").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Etiquetar y ordenar cada MP3 nuevo").performScrollTo().assertIsDisplayed()
    }
}
