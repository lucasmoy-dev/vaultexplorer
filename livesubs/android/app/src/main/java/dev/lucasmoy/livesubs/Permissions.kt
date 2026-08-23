package dev.lucasmoy.livesubs

import android.Manifest
import android.os.Build

/**
 * Which runtime permissions still have to be asked for.
 *
 * A free function with no Android dependencies on purpose: this list being
 * wrong is what made "Empezar" a dead button -- `POST_NOTIFICATIONS` was
 * added on every Android 13+ device whether or not it was already granted,
 * so the start flow always stopped to "ask" for something the user had
 * already allowed, the system returned instantly with no dialog, and nothing
 * else happened. Now it is a pure function with tests.
 */
fun missingPermissions(
    micGranted: Boolean,
    notificationsGranted: Boolean,
    sdk: Int,
): List<String> = buildList {
    if (!micGranted) add(Manifest.permission.RECORD_AUDIO)
    // The notification permission only exists from Android 13; on anything
    // older, asking for it is both pointless and denied.
    if (sdk >= Build.VERSION_CODES.TIRAMISU && !notificationsGranted) {
        add(Manifest.permission.POST_NOTIFICATIONS)
    }
}
