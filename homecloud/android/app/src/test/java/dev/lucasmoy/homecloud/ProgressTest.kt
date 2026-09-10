package dev.lucasmoy.homecloud

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the notification says, which for a sync that runs for hours with the
 * app closed is the only thing anyone reads.
 */
class ProgressTest {

    private fun folder(
        label: String,
        state: FolderState,
        bytes: Long = 0,
        pending: Long = 0,
        rate: Long = 0,
        pausedByNetwork: Boolean = false,
    ) = SharedFolder(
        id = label.lowercase(),
        label = label,
        path = "/sdcard/$label",
        state = state,
        peers = emptyList(),
        bytes = bytes,
        files = 0,
        conflicts = 0,
        bytesPerSecond = rate,
        mode = "twoWay",
        extraBytes = 0,
        freeBytes = null,
        pendingBytes = pending,
        wifiOnly = false,
        pausedByNetwork = pausedByNetwork,
        hasPassword = false,
        etaSeconds = null,
    )

    @Test
    fun `one folder syncing says which, how far and how long`() {
        val progress = notificationProgress(
            listOf(
                folder("Fotos", FolderState.Syncing(9), bytes = 100_000_000, pending = 91_000_000, rate = 5_000_000),
            ),
        )
        assertEquals("Sincronizando «Fotos» 9%", progress.title)
        assertEquals(9, progress.percent)
        assertEquals("faltan 18 s · 5,0 MB/s", progress.detail)
    }

    @Test
    fun `several folders are counted together, not one of them picked`() {
        val progress = notificationProgress(
            listOf(
                folder("Fotos", FolderState.Syncing(50), bytes = 100, pending = 50, rate = 10),
                folder("Vídeos", FolderState.Syncing(50), bytes = 100, pending = 50, rate = 10),
                folder("Documentos", FolderState.UpToDate, bytes = 100),
            ),
        )
        assertEquals("Sincronizando 2 carpetas · 50%", progress.title)
    }

    @Test
    fun `a stalled sync gives no invented estimate`() {
        val progress = notificationProgress(
            listOf(folder("Fotos", FolderState.Syncing(9), bytes = 100, pending = 91, rate = 0)),
        )
        assertTrue("no debe prometer un tiempo que no puede medir", progress.detail!!.contains("por bajar"))
    }

    @Test
    fun `waiting for wifi says so instead of looking broken`() {
        val progress = notificationProgress(
            listOf(folder("Fotos", FolderState.Paused, bytes = 100, pending = 40, pausedByNetwork = true)),
        )
        assertEquals("En pausa hasta que haya wifi", progress.title)
    }

    @Test
    fun `something that needs a person comes before anything else`() {
        val progress = notificationProgress(
            listOf(
                folder("Fotos", FolderState.Syncing(9), bytes = 100, pending = 91, rate = 10),
                folder("Vídeos", FolderState.Problem("El disco está lleno")),
            ),
        )
        assertEquals("«Vídeos» necesita que mires", progress.title)
        assertEquals("El disco está lleno", progress.detail)
    }

    @Test
    fun `nothing to do says everything is up to date`() {
        val progress = notificationProgress(listOf(folder("Fotos", FolderState.UpToDate, bytes = 2_000_000_000)))
        assertEquals("Todo al día", progress.title)
        assertNull(progress.percent)
    }

    @Test
    fun `deletion dates read as a person would say them`() {
        val now = 1_757_000_000L
        assertEquals("hace un momento", timeAgo(now - 10, now))
        assertEquals("hace 5 min", timeAgo(now - 300, now))
        assertEquals("hace 3 h", timeAgo(now - 3 * 3600, now))
        assertEquals("ayer", timeAgo(now - 30 * 3600, now))
        assertEquals("hace 4 días", timeAgo(now - 4 * 86400, now))
    }
}
