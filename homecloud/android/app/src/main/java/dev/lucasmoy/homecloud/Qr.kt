package dev.lucasmoy.homecloud

import android.graphics.Bitmap
import android.graphics.Color
import android.view.WindowManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

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
        // The lightest correction level, on purpose. This is read off a lit
        // screen a hand's width away, where spare correction buys nothing and
        // only packs the squares tighter -- and wider squares are what lets
        // the other device's camera lock on quickly.
        EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L,
        EncodeHintType.MARGIN to 2,
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
 * Keeps the screen on, and optionally at full brightness, while a dialog is up.
 *
 * A QR is only useful while it is on screen and readable. The phone showing
 * one would dim after a few seconds and then lock outright, in the middle of
 * the other device trying to read it, and a dimmed screen is exactly what a
 * camera cannot resolve. Both settings belong to this dialog's own window, so
 * closing it puts everything back with no bookkeeping.
 */
@Composable
fun KeepScreenReadable(bright: Boolean) {
    val window = (LocalView.current.parent as? DialogWindowProvider)?.window
    DisposableEffect(window, bright) {
        val before = window?.attributes?.screenBrightness
        window?.let {
            if (bright) {
                it.attributes = it.attributes.apply { screenBrightness = 1f }
            }
            it.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
        onDispose {
            window?.let {
                it.attributes = it.attributes.apply {
                    screenBrightness = before ?: WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
                }
                it.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
        }
    }
}
