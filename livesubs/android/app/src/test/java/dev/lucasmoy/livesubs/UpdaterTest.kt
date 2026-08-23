package dev.lucasmoy.livesubs

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Version comparison, which is the part of the updater that can be wrong
 * quietly: an app that thinks 0.10.0 is older than 0.9.0 never offers the
 * update, and one that gets it backwards offers a downgrade forever.
 */
class UpdaterTest {

    @Test
    fun `versions parse, suffixes and junk included`() {
        assertEquals(listOf(0, 1, 0), Updater.semver("0.1.0"))
        assertEquals(listOf(1, 2, 3), Updater.semver("1.2.3-beta1"))
        assertEquals(listOf(1, 2, 3), Updater.semver("1.2.3+build7"))
        assertEquals(listOf(2, 0, 0), Updater.semver("2"))
        assertEquals(listOf(0, 0, 0), Updater.semver("nonsense"))
    }

    @Test
    fun `double digit components compare numerically, not alphabetically`() {
        assertTrue(Updater.newer(Updater.semver("0.10.0"), Updater.semver("0.9.0")))
        assertFalse(Updater.newer(Updater.semver("0.9.0"), Updater.semver("0.10.0")))
        assertTrue(Updater.newer(Updater.semver("1.0.0"), Updater.semver("0.99.99")))
    }

    @Test
    fun `the same version is not an update`() {
        assertFalse(Updater.newer(Updater.semver("0.1.0"), Updater.semver("0.1.0")))
        assertFalse(Updater.newer(Updater.semver("0.1.0"), Updater.semver("0.1.1")))
        assertTrue(Updater.newer(Updater.semver("0.1.1"), Updater.semver("0.1.0")))
    }
}
