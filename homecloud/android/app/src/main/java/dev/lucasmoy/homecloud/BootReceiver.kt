package dev.lucasmoy.homecloud

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Starts syncing when the phone starts.
 *
 * The manifest has asked for `RECEIVE_BOOT_COMPLETED` since the beginning and
 * nothing listened for it, so a reboot left the phone silently not syncing
 * until someone opened the app — the one state a sync app must never be in
 * without saying so.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED &&
            intent.action != "android.intent.action.QUICKBOOT_POWERON"
        ) {
            return
        }
        runCatching { SyncService.start(context) }
    }
}
