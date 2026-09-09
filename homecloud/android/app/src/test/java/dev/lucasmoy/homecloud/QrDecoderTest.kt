package dev.lucasmoy.homecloud

import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The scanner, without a camera.
 *
 * A real pairing code is drawn as a QR, laid into a frame the shape and size
 * of what the camera hands over — padding on the rows included — and read
 * back. It is the one part of scanning that can be checked before an APK is
 * on a phone, so it is worth checking properly.
 */
class QrDecoderTest {

    private val code =
        "HC2v9eumj8_VHhyUBO6FTlU3drFD9rhOZ_3Mv31HkGqXXwRUG9ydGF0aWwgZGUgTHVjYXMMZm90b3Mt" +
            "YTFiMmMzBUZvdG9zAhl0Y3A6Ly8xOTIuMTY4LjEuMTUxOjIyMDAwGnF1aWM6Ly8xOTIuMTY4LjEu" +
            "MTUxOjIyMDAwAA"

    @Test
    fun `reads a pairing code out of a camera-shaped frame`() {
        val frame = frameWith(code, scale = 4, rowStride = 1280 + 32, width = 1280, height = 720)
        assertEquals(code, QrDecoder.decode(frame, 1280 + 32, 720))
    }

    @Test
    fun `reads a code drawn light on dark, as a phone in dark mode shows it`() {
        val frame = frameWith(code, scale = 4, rowStride = 1280, width = 1280, height = 720, invert = true)
        assertEquals(code, QrDecoder.decode(frame, 1280, 720))
    }

    @Test
    fun `a frame with no code in it finds none`() {
        val frame = ByteArray(1280 * 720) { (it % 251).toByte() }
        assertNull(QrDecoder.decode(frame, 1280, 720))
    }

    /** A QR drawn in the middle of an otherwise empty frame of brightness. */
    private fun frameWith(
        content: String,
        scale: Int,
        rowStride: Int,
        width: Int,
        height: Int,
        invert: Boolean = false,
    ): ByteArray {
        val matrix = QRCodeWriter().encode(
            content,
            BarcodeFormat.QR_CODE,
            0,
            0,
            mapOf(
                EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L,
                EncodeHintType.MARGIN to 2,
            ),
        )
        val background = if (invert) 0 else 255
        val ink = if (invert) 255 else 0
        val frame = ByteArray(rowStride * height) { background.toByte() }
        val drawn = matrix.width * scale
        require(drawn <= minOf(width, height)) { "the code does not fit in the frame" }
        val left = (width - drawn) / 2
        val top = (height - drawn) / 2
        for (y in 0 until drawn) {
            for (x in 0 until drawn) {
                if (matrix[x / scale, y / scale]) {
                    frame[(top + y) * rowStride + left + x] = ink.toByte()
                }
            }
        }
        return frame
    }
}
