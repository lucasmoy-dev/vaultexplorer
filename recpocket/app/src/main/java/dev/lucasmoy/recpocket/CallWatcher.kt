package dev.lucasmoy.recpocket

import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification

/**
 * Starts a recording when a WhatsApp call starts, and stops it when the call
 * ends.
 *
 * A notification listener is the only way to know: WhatsApp broadcasts
 * nothing, and no API tells one app about another's calls. What it does do
 * is post an ongoing notification for the duration of a call -- so this
 * watches for it appearing and disappearing, and [CallSignals] decides from
 * the wording whether there is video.
 *
 * Requires the user to grant "notification access" in system settings; the
 * app asks for it only when the trigger is switched on, and the trigger is
 * off by default. Nothing here reads notifications from anything but
 * WhatsApp, and nothing is stored.
 *
 * Consent to *recording a call* is the user's business and varies by
 * country; this app records what its owner tells it to and says so in the
 * open, with a permanent notification while it runs.
 */
class CallWatcher : NotificationListenerService() {
    /** The call this service started a recording for, so an unrelated
     *  notification disappearing does not stop it. */
    private var recordingFor: String? = null

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        val notification = sbn ?: return
        if (!Settings(this).callTrigger) return

        val call = CallSignals.classify(
            notification.packageName,
            textOf(notification),
            isOngoing = notification.isOngoing,
        )
        if (call == CallSignals.Call.NONE) return
        if (recordingFor != null) return

        recordingFor = notification.key
        // A video call is worth the screen; a voice call is not, and would
        // cost ten times the file for a black rectangle.
        val kind = when (call) {
            CallSignals.Call.VIDEO -> Naming.Kind.CALL_VIDEO
            else -> Naming.Kind.CALL_AUDIO
        }
        CaptureService.send(this, CaptureService.ACTION_START, kind)
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        val notification = sbn ?: return
        if (notification.key != recordingFor) return
        recordingFor = null
        CaptureService.send(this, CaptureService.ACTION_STOP)
    }

    /** Everything the notification shows, joined: more words can only help
     *  the match, and different WhatsApp versions put the useful one in
     *  different fields. */
    private fun textOf(notification: StatusBarNotification): String {
        val extras = notification.notification.extras
        val fields = listOf(
            android.app.Notification.EXTRA_TITLE,
            android.app.Notification.EXTRA_TEXT,
            android.app.Notification.EXTRA_SUB_TEXT,
            android.app.Notification.EXTRA_BIG_TEXT,
        )
        val parts = fields.mapNotNull { extras.getCharSequence(it)?.toString() }
        return (parts + (notification.notification.channelId ?: "")).joinToString(" ")
    }
}
