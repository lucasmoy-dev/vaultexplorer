package dev.lucasmoy.homecloud

import android.graphics.Bitmap
import android.graphics.Color
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions

/**
 * The two halves of pairing by camera.
 *
 * Typing a 100-character code into a phone is the worst way to do this and was
 * the only way the phone offered: the desktop drew a QR and told the user to
 * scan it from the other device, which could not scan anything. Both halves
 * live here so neither can be added without the other.
 */

/** Draws a code as a QR, or `null` if it will not fit one. */
fun qrBitmap(content: String, sizePx: Int = 720): Bitmap? = runCatching {
    val hints = mapOf(
        // The code is read off a lit screen at short range, so the middle
        // correction level buys reliability without inflating the pattern.
        EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M,
        EncodeHintType.MARGIN to 1,
    )
    val matrix = QRCodeWriter().encode(content, BarcodeFormat.QR_CODE, sizePx, sizePx, hints)
    Bitmap.createBitmap(matrix.width, matrix.height, Bitmap.Config.ARGB_8888).apply {
        for (x in 0 until matrix.width) {
            for (y in 0 until matrix.height) {
                // Always black on white, never the theme's colours: a QR needs
                // the contrast, and a dark-mode grey on grey does not scan.
                setPixel(x, y, if (matrix[x, y]) Color.BLACK else Color.WHITE)
            }
        }
    }
}.getOrNull()

@Composable
fun rememberQr(content: String, sizePx: Int = 720): ImageBitmap? =
    remember(content, sizePx) { qrBitmap(content, sizePx)?.asImageBitmap() }

/**
 * Opens the camera and hands back whatever code was scanned, or nothing if the
 * user backed out. The permission prompt is the scanner's own.
 */
@Composable
fun rememberQrScanner(onScanned: (String) -> Unit): () -> Unit {
    val launcher = rememberLauncherForActivityResult(ScanContract()) { result ->
        result.contents?.let(onScanned)
    }
    return {
        launcher.launch(
            ScanOptions()
                .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                .setPrompt("Apunta al código del otro dispositivo")
                .setBeepEnabled(false)
                .setOrientationLocked(false),
        )
    }
}
