package dev.lucasmoy.recpocket

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.util.DisplayMetrics
import androidx.core.app.NotificationCompat
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.LifecycleService
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * The recorder's home: a foreground service that holds the screen-capture
 * permission and does the recording.
 *
 * Why a service at all, and why it holds the projection even when idle:
 *
 *  * A recording has to survive the app being closed -- that is the point.
 *  * `MediaProjection` consent is a **dialog per grant**: an app cannot ask
 *    for it in the background. So "record automatically when a WhatsApp
 *    call starts" is only possible if the permission was granted *before*
 *    the call, and kept. The service therefore keeps the projection alive
 *    while armed, which is also why arming shows a notification saying so:
 *    a permission this powerful should be visible, not quiet.
 *  * Android 14 requires a `mediaProjection` foreground-service type, and
 *    requires the service to be in the foreground *before* the projection
 *    is used at all -- hence `startForeground` first, projection second.
 */
class CaptureService : LifecycleService() {
    private var projection: android.media.projection.MediaProjection? = null
    private var recorder: Recorder? = null
    private var currentFile: File? = null
    private var currentKind: Naming.Kind = Naming.Kind.MIC
    private var startedAt = 0L

    override fun onCreate() {
        super.onCreate()
        createChannels()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        // Foreground first, always: using a projection from a background
        // service is a hard error on Android 14+.
        startForeground(NOTIFICATION_ID, notification(idleText()))

        when (intent?.action) {
            ACTION_ARM -> arm(intent)
            ACTION_START -> startRecording(kindFor(intent), intent?.getStringExtra(EXTRA_NOTE).orEmpty())
            ACTION_STOP -> stopRecording()
            ACTION_SHOT -> takeScreenshot()
            ACTION_QUIT -> {
                stopRecording()
                Overlay.hide(this)
                projection?.stop()
                projection = null
                armed.value = false
                stopSelf()
            }
        }
        return START_STICKY
    }

    /** Keep the granted projection, and put the floating button up. */
    private fun arm(intent: Intent) {
        val resultCode = intent.getIntExtra(EXTRA_RESULT_CODE, android.app.Activity.RESULT_CANCELED)
        val data: Intent? = intent.getParcelableExtra(EXTRA_RESULT_DATA)
        if (resultCode != android.app.Activity.RESULT_OK || data == null) return
        val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE)
            as android.media.projection.MediaProjectionManager
        projection = manager.getMediaProjection(resultCode, data).apply {
            // The system can revoke it (another app starts a projection, the
            // user taps "stop sharing"): everything downstream has to know.
            registerCallback(
                object : android.media.projection.MediaProjection.Callback() {
                    override fun onStop() {
                        stopRecording()
                        this@CaptureService.projection = null
                        armed.value = false
                        update(getString(R.string.state_disarmed))
                    }
                },
                null,
            )
        }
        armed.value = true
        if (Settings(this).overlay) Overlay.show(this)
        update(idleText())
    }

    private fun kindFor(intent: Intent?): Naming.Kind {
        intent?.getStringExtra(EXTRA_KIND)?.let { name ->
            runCatching { return Naming.Kind.valueOf(name) }
        }
        val settings = Settings(this)
        return when {
            settings.screen -> Naming.Kind.SCREEN
            settings.sources == Settings.Sources.MIC -> Naming.Kind.MIC
            settings.sources == Settings.Sources.PLAYBACK -> Naming.Kind.PLAYBACK
            else -> Naming.Kind.BOTH
        }
    }

    private fun startRecording(requested: Naming.Kind, note: String) {
        if (recording.value) return
        val settings = Settings(this)
        // Without the capture permission there is no video and no playback
        // audio, so the recording becomes microphone-only -- and says so in
        // its name rather than producing an .mp4 with no video track.
        val kind = if (projection == null) with(Naming) { requested.audioOnly() } else requested
        val wantsVideo = kind == Naming.Kind.SCREEN || kind == Naming.Kind.CALL_VIDEO
        // A call recording is about the voices, whatever the general setting
        // says; anything else follows the configuration.
        val sources = when (kind) {
            Naming.Kind.CALL_AUDIO, Naming.Kind.CALL_VIDEO -> Settings.Sources.BOTH
            Naming.Kind.MIC -> Settings.Sources.MIC
            Naming.Kind.PLAYBACK -> Settings.Sources.PLAYBACK
            else -> settings.sources
        }
        val usable = if (projection == null) Settings.Sources.MIC else sources
        if (usable != sources) {
            update(getString(R.string.state_mic_only))
        }

        val file = File(Output.workDir(this), "current.${kind.extension}")
        file.delete()
        val config = Recorder.Config(
            sources = usable,
            audio = settings.audioQuality,
            video = if (wantsVideo && projection != null) settings.videoQuality else null,
            metrics = if (wantsVideo) displayMetrics() else null,
        )
        val started = runCatching {
            Recorder(config, projection).also {
                it.start(file)
                recorder = it
            }
        }
        started.onFailure { error ->
            lastResult.value = Result(null, "", error.message ?: error.toString())
            update(idleText())
            return
        }
        currentFile = file
        currentKind = kind
        startedAt = System.currentTimeMillis()
        recording.value = true
        update(getString(R.string.state_recording, kind.label))
    }

    private fun stopRecording() {
        val active = recorder ?: return
        val file = currentFile
        recorder = null
        currentFile = null
        recording.value = false
        val kind = currentKind
        val startedAtMillis = startedAt

        lifecycleScope.launch {
            val outcome = withContext(Dispatchers.IO) {
                runCatching {
                    active.stop()
                    active.failure?.let { throw IllegalStateException(it) }
                    val source = requireNotNull(file) { "no había archivo" }
                    require(source.length() > 0) { "la grabación salió vacía" }
                    val name = Naming.fileName(kind, startedAtMillis)
                    Output.publish(this@CaptureService, source, name) to name
                }
            }
            outcome.fold(
                onSuccess = { (uri, name) ->
                    lastResult.value = Result(uri, name, null)
                    notifySaved(name, uri)
                },
                onFailure = { lastResult.value = Result(null, "", it.message ?: it.toString()) },
            )
            update(idleText())
        }
    }

    private fun takeScreenshot() {
        val active = projection
        if (active == null) {
            lastResult.value = Result(null, "", getString(R.string.error_needs_arming))
            return
        }
        val at = System.currentTimeMillis()
        lifecycleScope.launch {
            val outcome = withContext(Dispatchers.IO) {
                runCatching {
                    val file = File(Output.workDir(this@CaptureService), "shot.jpg")
                    Screenshot.capture(active, displayMetrics(), quality = 85, into = file)
                    val name = Naming.fileName(Naming.Kind.SCREENSHOT, at)
                    Output.publish(this@CaptureService, file, name) to name
                }
            }
            outcome.fold(
                onSuccess = { (uri, name) ->
                    lastResult.value = Result(uri, name, null)
                    notifySaved(name, uri)
                },
                onFailure = { lastResult.value = Result(null, "", it.message ?: it.toString()) },
            )
        }
    }

    @Suppress("DEPRECATION")
    private fun displayMetrics(): DisplayMetrics {
        val metrics = DisplayMetrics()
        val window = getSystemService(Context.WINDOW_SERVICE) as android.view.WindowManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val bounds = window.currentWindowMetrics.bounds
            metrics.widthPixels = bounds.width()
            metrics.heightPixels = bounds.height()
            metrics.densityDpi = resources.configuration.densityDpi
        } else {
            window.defaultDisplay.getRealMetrics(metrics)
        }
        return metrics
    }

    // ---- notifications --------------------------------------------------

    private fun createChannels() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Grabación", NotificationManager.IMPORTANCE_LOW)
        )
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_DONE, "Listo", NotificationManager.IMPORTANCE_DEFAULT)
        )
    }

    private fun idleText(): String =
        if (armed.value) getString(R.string.state_armed) else getString(R.string.state_idle)

    private fun notification(text: String): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = NotificationCompat.Builder(this, CHANNEL)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setSmallIcon(android.R.drawable.presence_audio_online)
            .setOngoing(true)
            .setContentIntent(open)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)

        if (recording.value) {
            builder.addAction(
                0,
                getString(R.string.action_stop),
                action(ACTION_STOP),
            )
        } else {
            builder.addAction(0, getString(R.string.action_record), action(ACTION_START))
        }
        builder.addAction(0, getString(R.string.action_screenshot), action(ACTION_SHOT))
        builder.addAction(0, getString(R.string.action_quit), action(ACTION_QUIT))
        return builder.build()
    }

    private fun action(name: String): PendingIntent = PendingIntent.getService(
        this,
        name.hashCode(),
        Intent(this, CaptureService::class.java).setAction(name),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun update(text: String) {
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, notification(text))
    }

    private fun notifySaved(name: String, uri: Uri) {
        val view = PendingIntent.getActivity(
            this,
            name.hashCode(),
            Intent(Intent.ACTION_VIEW).setData(uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        getSystemService(NotificationManager::class.java).notify(
            name.hashCode(),
            NotificationCompat.Builder(this, CHANNEL_DONE)
                .setContentTitle(getString(R.string.saved))
                .setContentText(name)
                .setSmallIcon(android.R.drawable.stat_sys_download_done)
                .setAutoCancel(true)
                .setContentIntent(view)
                .build(),
        )
    }

    data class Result(val uri: Uri?, val name: String, val error: String?)

    companion object {
        private const val CHANNEL = "capture"
        private const val CHANNEL_DONE = "saved"
        private const val NOTIFICATION_ID = 1

        const val ACTION_ARM = "arm"
        const val ACTION_START = "start"
        const val ACTION_STOP = "stop"
        const val ACTION_SHOT = "screenshot"
        const val ACTION_QUIT = "quit"
        const val EXTRA_RESULT_CODE = "resultCode"
        const val EXTRA_RESULT_DATA = "resultData"
        const val EXTRA_KIND = "kind"
        const val EXTRA_NOTE = "note"

        /** Whether the screen-capture permission is held right now. Also
         *  what the call trigger checks before trying to record video. */
        val armed = MutableStateFlow(false)
        val recording = MutableStateFlow(false)
        val lastResult = MutableStateFlow<Result?>(null)

        val isArmed: StateFlow<Boolean> get() = armed
        val isRecording: StateFlow<Boolean> get() = recording

        fun send(context: Context, action: String, kind: Naming.Kind? = null) {
            val intent = Intent(context, CaptureService::class.java).setAction(action)
            if (kind != null) intent.putExtra(EXTRA_KIND, kind.name)
            context.startForegroundService(intent)
        }
    }
}
