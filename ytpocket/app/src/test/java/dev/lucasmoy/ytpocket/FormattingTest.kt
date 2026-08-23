package dev.lucasmoy.ytpocket

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The pure-logic half of the app on a plain JVM: how a result row reads,
 * and version comparison for the updater. The filename rules live in Rust
 * (with their own tests) because they are the app's promise; these are the
 * bits Kotlin owns.
 */
class FormattingTest {

    @Test
    fun `durations read like a video player`() {
        assertEquals("0:07", formatDuration(7))
        assertEquals("4:07", formatDuration(247))
        assertEquals("1:02:03", formatDuration(3723))
        // No duration means a livestream, which has nothing to download --
        // the row says so rather than showing "0:00".
        assertEquals("en directo", formatDuration(null))
        assertEquals("en directo", formatDuration(0))
    }

    @Test
    fun `view counts are shortened, not dumped`() {
        assertEquals("", formatViews(null))
        assertEquals("", formatViews(0))
        assertEquals("742", formatViews(742))
        assertEquals("340 mil", formatViews(340_500))
        assertEquals("1,2 M", formatViews(1_234_567))
        assertEquals("1642,0 M", formatViews(1_642_000_000))
    }

    @Test
    fun `update versions compare numerically`() {
        assertEquals(listOf(0, 1, 0), Updater.semver("0.1.0"))
        assertEquals(listOf(1, 2, 3), Updater.semver("1.2.3-beta"))
        // The mistake this guards against: comparing as text would put
        // 0.9.0 above 0.10.0 and the update would never be offered.
        assertTrue(Updater.newer(Updater.semver("0.10.0"), Updater.semver("0.9.0")))
        assertFalse(Updater.newer(Updater.semver("0.1.0"), Updater.semver("0.1.0")))
        assertTrue(Updater.newer(Updater.semver("1.0.0"), Updater.semver("0.99.99")))
    }
}
