package dev.lucasmoy.ytpocket

import android.os.Build
import android.view.HapticFeedbackConstants
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import kotlinx.coroutines.delay

/**
 * One search result: thumbnail, title, and the two download buttons on the
 * right -- MP4 first, MP3 last, so the one used most sits under the thumb.
 *
 * Top level (not inside the activity) so a Compose test can draw it on its
 * own and tap it.
 */
@Composable
internal fun ResultRow(hit: Native.Hit, onQueue: (kind: String) -> Unit) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(10.dp)) {
            Row(verticalAlignment = Alignment.Top) {
                AsyncImage(
                    model = hit.thumbnail,
                    contentDescription = null,
                    modifier = Modifier
                        .width(120.dp)
                        .height(68.dp)
                        .clip(RoundedCornerShape(6.dp)),
                )
                Column(Modifier.padding(start = 10.dp)) {
                    Text(
                        hit.title,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Medium,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        listOfNotNull(
                            hit.channel.takeIf { it.isNotEmpty() },
                            formatDuration(hit.duration),
                            formatViews(hit.views).takeIf { it.isNotEmpty() },
                            hit.published,
                        ).joinToString(" · "),
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                    )
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // A livestream has no file to download -- YouTube serves it as
                // a segment playlist, not a media file -- so the buttons say
                // so instead of failing later.
                if (hit.isLive) {
                    Text(
                        stringResource(R.string.live_not_downloadable),
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                }
                DownloadButton(
                    label = stringResource(R.string.action_mp4),
                    enabled = !hit.isLive,
                    modifier = Modifier.testTag("download-mp4"),
                ) { onQueue(DownloadService.KIND_MP4) }
                DownloadButton(
                    label = stringResource(R.string.action_mp3),
                    enabled = !hit.isLive,
                    modifier = Modifier.testTag("download-mp3"),
                ) { onQueue(DownloadService.KIND_MP3) }
            }
        }
    }
}

/**
 * A download button that visibly takes the tap.
 *
 * The plain Material ripple was too faint to register -- "no parece que haya
 * hecho clic". So a tap now does four things at once: the button sinks while
 * pressed (a springy scale), the phone gives a short haptic tick, and for a
 * moment the button turns the accent colour and reads "✓ En cola". The
 * screen's snackbar says the rest (which file, and what is ahead of it).
 *
 * While it shows "En cola" a second tap is ignored: two fast taps were two
 * identical downloads.
 */
@Composable
internal fun DownloadButton(
    label: String,
    enabled: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    var queued by remember { mutableStateOf(false) }
    val view = LocalView.current
    val scale by animateFloatAsState(
        targetValue = if (pressed) 0.88f else 1f,
        animationSpec = spring(dampingRatio = Spring.DampingRatioMediumBouncy, stiffness = Spring.StiffnessMedium),
        label = "press",
    )
    val container by animateColorAsState(
        if (queued) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.primary,
        label = "queued",
    )
    LaunchedEffect(queued) {
        if (queued) {
            delay(1800)
            queued = false
        }
    }
    Button(
        onClick = {
            if (queued) return@Button
            view.performHapticFeedback(
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) HapticFeedbackConstants.CONFIRM
                else HapticFeedbackConstants.VIRTUAL_KEY
            )
            queued = true
            onClick()
        },
        enabled = enabled,
        interactionSource = interaction,
        colors = ButtonDefaults.buttonColors(containerColor = container),
        contentPadding = ButtonDefaults.ButtonWithIconContentPadding,
        // Wide enough for "En cola" either way, so the row does not jump
        // when the label changes -- and both buttons are the same width.
        modifier = modifier
            .widthIn(min = 108.dp)
            .graphicsLayer {
                scaleX = scale
                scaleY = scale
            },
    ) {
        if (queued) {
            Icon(Icons.Filled.Check, contentDescription = null, modifier = Modifier.size(18.dp))
        } else {
            Icon(painterResource(R.drawable.ic_download), contentDescription = null, modifier = Modifier.size(18.dp))
        }
        Spacer(Modifier.width(6.dp))
        Text(if (queued) stringResource(R.string.queued_short) else label)
    }
}
