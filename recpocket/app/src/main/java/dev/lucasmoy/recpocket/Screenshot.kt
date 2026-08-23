package dev.lucasmoy.recpocket

import android.graphics.Bitmap
import android.hardware.display.DisplayManager
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.util.DisplayMetrics
import java.io.File

/**
 * One frame of the screen, as a JPEG.
 *
 * The same `MediaProjection` the recorder uses, pointed at an `ImageReader`
 * instead of an encoder. Two details that are easy to get wrong:
 *
 *  * **Row padding.** A captured image's rows are padded to a hardware
 *    stride, so the buffer is wider than the screen. Copying it straight
 *    into a bitmap produces the familiar diagonal-smear screenshot; the
 *    padding has to be accounted for and then cropped.
 *  * **The first frame is not ready.** The virtual display takes a moment
 *    to produce anything, so this waits for a frame rather than grabbing
 *    whatever is there (which is nothing).
 *
 * The floating controls do not appear in the result: their window is
 * `FLAG_SECURE`, which keeps them off any non-secure display -- and a
 * projection's virtual display is exactly that. See [Overlay].
 */
object Screenshot {
    fun capture(
        projection: MediaProjection,
        metrics: DisplayMetrics,
        quality: Int,
        into: File,
        timeoutMs: Long = 2500,
    ): File {
        val width = metrics.widthPixels
        val height = metrics.heightPixels
        val reader = ImageReader.newInstance(width, height, android.graphics.PixelFormat.RGBA_8888, 2)
        val display = projection.createVirtualDisplay(
            "recpocket-shot",
            width,
            height,
            metrics.densityDpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            reader.surface,
            null,
            null,
        )
        try {
            val deadline = System.currentTimeMillis() + timeoutMs
            var image = reader.acquireLatestImage()
            while (image == null && System.currentTimeMillis() < deadline) {
                Thread.sleep(40)
                image = reader.acquireLatestImage()
            }
            requireNotNull(image) { "la pantalla no entregó ninguna imagen" }

            image.use { frame ->
                val plane = frame.planes[0]
                val pixelStride = plane.pixelStride
                val rowStride = plane.rowStride
                val padding = rowStride - pixelStride * width
                val padded = Bitmap.createBitmap(
                    width + padding / pixelStride,
                    height,
                    Bitmap.Config.ARGB_8888,
                )
                padded.copyPixelsFromBuffer(plane.buffer)
                val cropped = Bitmap.createBitmap(padded, 0, 0, width, height)
                into.outputStream().use { out ->
                    cropped.compress(Bitmap.CompressFormat.JPEG, quality, out)
                }
                cropped.recycle()
                padded.recycle()
            }
            return into
        } finally {
            display.release()
            reader.close()
        }
    }
}
