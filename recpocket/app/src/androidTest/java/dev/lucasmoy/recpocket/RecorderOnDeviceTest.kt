package dev.lucasmoy.recpocket

import android.media.MediaMetadataRetriever
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import androidx.test.rule.GrantPermissionRule

/**
 * The recording pipeline, on a real Android runtime.
 *
 * Everything above [Recorder] is checked on the JVM, but the pipeline itself
 * is `AudioRecord` -> mix -> `MediaCodec` -> `MediaMuxer`, and none of those
 * exist off a device. This records the microphone for a few seconds and then
 * asks the platform to play the result back: a file that `MediaMetadata`
 * reports as three seconds of AAC is a file that works, and the classic
 * failure of hand-built muxing (a zero-byte or unplayable file, because the
 * muxer was stopped before the last samples arrived) fails this test loudly.
 *
 * An emulator has no real microphone, so the audio is silence. That is fine:
 * what is being tested is the container, the timeline and the teardown, and
 * silence exercises every one of them.
 *
 *     gradle :app:connectedDebugAndroidTest
 */
@RunWith(AndroidJUnit4::class)
class RecorderOnDeviceTest {
    // Granted here rather than by hand: Gradle uninstalls the app after a
    // connected run, so any grant given from the shell is gone by the next
    // one -- which showed up as "Cannot create AudioRecord" and looks
    // exactly like a broken recorder.
    @get:Rule
    val microphone: GrantPermissionRule =
        GrantPermissionRule.grant(android.Manifest.permission.RECORD_AUDIO)

    @Test
    fun a_microphone_recording_comes_out_playable() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val output = File(Output.workDir(context), "instrumented.m4a")
        output.delete()

        val recorder = Recorder(
            Recorder.Config(
                sources = Settings.Sources.MIC,
                audio = Settings.AudioQuality.VOICE,
                video = null,
            ),
            projection = null,
        )
        recorder.start(output)
        assertTrue(recorder.isRunning)
        Thread.sleep(3_000)
        recorder.stop()

        assertNull("the recorder reported a failure", recorder.failure)
        assertTrue("nothing was written", output.length() > 1_000)

        val meta = MediaMetadataRetriever()
        try {
            meta.setDataSource(output.absolutePath)
            val millis =
                meta.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLong() ?: 0L
            val mime = meta.extractMetadata(MediaMetadataRetriever.METADATA_KEY_MIMETYPE)
            println("recorded ${output.length()} bytes, ${millis}ms, $mime")
            // Between two and five seconds: proof the timeline is real and
            // the last samples made it in before the muxer was stopped.
            assertTrue("duration was ${millis}ms", millis in 2_000..5_000)
            assertTrue("unexpected type $mime", mime?.contains("mp4") == true)
        } finally {
            meta.release()
        }

        // And it reaches the place a file manager looks.
        val name = Naming.fileName(Naming.Kind.MIC, System.currentTimeMillis())
        val uri = Output.publish(context, output, name)
        context.contentResolver.openInputStream(uri)!!.use {
            assertTrue("published file is empty", it.available() > 1_000)
        }
        context.contentResolver.delete(uri, null, null)
        // publish() moves the file: nothing is left in the cache.
        assertEquals(false, output.exists())
    }
}
