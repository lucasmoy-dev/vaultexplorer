package dev.lucasmoy.homecloud

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.FocusMeteringAction
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FlashlightOff
import androidx.compose.material.icons.filled.FlashlightOn
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The camera half of pairing, rebuilt on CameraX.
 *
 * What was here before was the scanner activity that ships with
 * zxing-android-embedded, which drives the camera through the deprecated
 * Camera1 API. On a modern phone that meant a preview at a few frames a
 * second and an autofocus that hunted back and forth: five minutes of aiming
 * at a QR without ever catching it, and the code typed in by hand in the end.
 *
 * CameraX draws the preview itself, at the frame rate the screen runs at, and
 * hands frames to [QrDecoder] on a background thread — so however long a frame
 * takes to read, what the user is aiming with stays smooth. Aiming is most of
 * the job.
 */
@Composable
fun QrScannerSheet(onScanned: (String) -> Unit, onClose: () -> Unit) {
    val context = LocalContext.current
    var granted by remember { mutableStateOf(hasCamera(context)) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        granted = it
    }
    LaunchedEffect(Unit) {
        if (!granted) ask.launch(Manifest.permission.CAMERA)
    }

    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Escanea el código") },
        text = {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                // Aiming a camera takes longer than the screen timeout.
                KeepScreenReadable(bright = false)
                if (granted) {
                    CameraFrame(onScanned = onScanned)
                    Spacer(Modifier.height(10.dp))
                    Text(
                        "Apunta a la pantalla del otro dispositivo. Se lee solo.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                } else {
                    Text(
                        "Sin permiso de cámara no se puede leer el QR. Puedes darlo, " +
                            "o cerrar y pegar el código a mano.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Spacer(Modifier.height(10.dp))
                    Button(onClick = { ask.launch(Manifest.permission.CAMERA) }) {
                        Text("Permitir la cámara")
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onClose) { Text("Cerrar") } },
    )
}

@Composable
private fun CameraFrame(onScanned: (String) -> Unit) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    var torch by remember { mutableStateOf(false) }
    var camera by remember { mutableStateOf<androidx.camera.core.Camera?>(null) }
    // One decode wins. Without this the analyzer fires again while the dialog
    // is closing and the code is redeemed twice.
    val done = remember { AtomicBoolean(false) }
    val analysisThread = remember { Executors.newSingleThreadExecutor() }

    DisposableEffect(Unit) {
        onDispose { analysisThread.shutdown() }
    }

    Box(Modifier.fillMaxWidth().height(300.dp).clip(RoundedCornerShape(12.dp))) {
        AndroidView(
            modifier = Modifier.fillMaxSize(),
            factory = { ctx ->
                val view = PreviewView(ctx).apply {
                    // The picture fills the frame rather than fitting inside
                    // it: a letterboxed preview makes the code look further
                    // away than it is, and people back off instead of closing in.
                    scaleType = PreviewView.ScaleType.FILL_CENTER
                    implementationMode = PreviewView.ImplementationMode.COMPATIBLE
                }
                val provider = ProcessCameraProvider.getInstance(ctx)
                provider.addListener({
                    val cameras = provider.get()
                    val preview = Preview.Builder().build().also {
                        it.surfaceProvider = view.surfaceProvider
                    }
                    // 1280x720 is plenty to read a code off a screen and a
                    // quarter of the pixels of what the camera would rather
                    // hand over.
                    val analysis = ImageAnalysis.Builder()
                        .setResolutionSelector(
                            ResolutionSelector.Builder()
                                .setResolutionStrategy(
                                    ResolutionStrategy(
                                        android.util.Size(1280, 720),
                                        ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER,
                                    )
                                )
                                .build()
                        )
                        // Read the newest frame and drop the rest: a backlog
                        // is a scanner reading what the camera saw a second ago.
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build()
                    analysis.setAnalyzer(analysisThread) { image ->
                        val found = readFrame(image)
                        image.close()
                        if (found != null && done.compareAndSet(false, true)) {
                            ContextCompat.getMainExecutor(ctx).execute { onScanned(found) }
                        }
                    }
                    runCatching {
                        cameras.unbindAll()
                        camera = cameras.bindToLifecycle(
                            owner,
                            CameraSelector.DEFAULT_BACK_CAMERA,
                            preview,
                            analysis,
                        )
                        // Keep hunting for focus at close range instead of
                        // settling once on whatever was in front of it.
                        val middle = view.meteringPointFactory.createPoint(0.5f, 0.5f)
                        camera?.cameraControl?.startFocusAndMetering(
                            FocusMeteringAction.Builder(middle).setAutoCancelDuration(2, java.util.concurrent.TimeUnit.SECONDS).build()
                        )
                    }
                }, ContextCompat.getMainExecutor(ctx))
                view
            },
        )
        // A code on a screen in a dark room, or a printed one: the torch is
        // the difference between reading it and not.
        IconButton(
            onClick = {
                torch = !torch
                camera?.cameraControl?.enableTorch(torch)
            },
            modifier = Modifier.align(Alignment.BottomEnd).padding(8.dp),
        ) {
            Icon(
                if (torch) Icons.Filled.FlashlightOn else Icons.Filled.FlashlightOff,
                contentDescription = if (torch) "Apagar la linterna" else "Encender la linterna",
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

/** Pulls the brightness plane out of a camera frame and reads it. */
private fun readFrame(image: ImageProxy): String? {
    val plane = image.planes.firstOrNull() ?: return null
    val buffer = plane.buffer
    val luma = ByteArray(buffer.remaining())
    buffer.get(luma)
    return QrDecoder.decode(luma, plane.rowStride, image.height)
}

private fun hasCamera(context: Context): Boolean =
    ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
        PackageManager.PERMISSION_GRANTED

/**
 * Finding a QR in one frame of camera brightness.
 *
 * Kept apart from the camera so it can be tested without one, which is the
 * only way any of this gets checked before it reaches a phone.
 */
object QrDecoder {

    /**
     * How much of the frame's short side each attempt looks at. The middle is
     * where a code being aimed at ends up, and searching less of the frame is
     * both faster and less likely to lock onto something in the background;
     * the full frame is the fallback for a code that is off to one side.
     */
    private val CROPS = floatArrayOf(0.65f, 1f)

    private val HINTS = mapOf(
        // Reading a code off a screen at an angle, hand-held: worth the extra
        // work per frame, since frames are cheap and a failed scan is not.
        DecodeHintType.TRY_HARDER to true,
    )

    /**
     * @param luma one byte of brightness per pixel, rows [rowStride] apart.
     * @param rowStride how wide a row is in the buffer, padding included.
     */
    fun decode(luma: ByteArray, rowStride: Int, height: Int): String? {
        val width = minOf(rowStride, luma.size / height.coerceAtLeast(1))
        if (width <= 0 || height <= 0) return null
        val shortest = minOf(width, height)
        for (crop in CROPS) {
            val side = (shortest * crop).toInt().coerceAtLeast(1)
            val left = (width - side) / 2
            val top = (height - side) / 2
            val source = runCatching {
                PlanarYUVLuminanceSource(luma, rowStride, height, left, top, side, side, false)
            }.getOrNull() ?: continue
            // A QR drawn light-on-dark — which is what a phone in dark mode
            // shows — is invisible to a reader that only tries one polarity.
            for (candidate in listOf(source, source.invert())) {
                val reader = QRCodeReader()
                val result = runCatching {
                    reader.decode(BinaryBitmap(HybridBinarizer(candidate)), HINTS)
                }.getOrNull()
                if (result != null) return result.text.trim()
            }
        }
        return null
    }
}
