package dev.lucasmoy.homecloud

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.File
import java.nio.file.Files

class FolderCacheTest {

    /** The shape the core really sends, route included. */
    private val listing = """
        [{"id":"cloud-lfwhxy","label":"cloud","path":"/storage/emulated/0/cloud",
          "state":{"kind":"upToDate"},
          "peers":[{"id":"MAB5WMU","name":"Portatil de Lucas","connected":true,"completion":100,"route":"lan"},
                   {"id":"Q4XJBIZ","name":"Pixel","connected":false,"completion":null,"route":null}],
          "bytes":1024,"files":3,"conflicts":0,"bytesPerSecond":0,"mode":"twoWay","extraBytes":0,
          "freeBytes":null,"pendingBytes":0,"wifiOnly":false,"pausedByNetwork":false,"hasPassword":false,
          "etaSeconds":null}]
    """.trimIndent()

    @Test
    fun a_saved_listing_comes_back_as_it_was() {
        val dir = Files.createTempDirectory("hc").toFile()
        val cache = FolderCache(File(dir, "last-folders.json"))
        assertNull("nothing saved yet", cache.read())
        cache.write(listing)
        val back = cache.read()!!
        assertEquals(1, back.size)
        assertEquals("cloud", back[0].label)
        assertEquals("lan", back[0].peers[0].route)
        assertNull(back[0].peers[1].route)
        assertEquals(" · en tu red", routeWords(back[0].peers[0].route))
        assertEquals("", routeWords(null))
        dir.deleteRecursively()
    }

    @Test
    fun a_corrupt_file_is_no_listing_rather_than_a_crash() {
        val dir = Files.createTempDirectory("hc").toFile()
        val file = File(dir, "last-folders.json")
        file.writeText("[{\"id\":")
        assertNull(FolderCache(file).read())
        dir.deleteRecursively()
    }

    @Test
    fun an_older_core_without_routes_still_parses() {
        val old = listing.replace(",\"route\":\"lan\"", "").replace(",\"route\":null", "")
        assertNull(parseFolders(old)[0].peers[0].route)
    }
}
