package dev.lucasmoy.livesubs

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The app while it is working: a foreground service that captures, listens,
 * draws the overlay and writes the transcript.
 *
 * This is the Android answer to the desktop version's tray icon. A phone
 * has no tray, and an app that must keep recording with its window closed
 * must be a foreground service with a notification -- so the notification
 * *is* the control surface: pause/resume, settings, stop. Same three
 * actions as the tray menu.
 *
 * Threading: one thread per audio source, each running capture -> VAD ->
 * whisper -> translation for its own stream. Two threads rather than one
 * mixed stream is what keeps "you" and "them" separable, which is the whole
 * point of the two colours.
 */
class CaptionService : LifecycleService() {

    private lateinit var prefs: Prefs
    private lateinit var overlay: OverlayController
    private val translations = Translations()
    private val main = Handler(Looper.getMainLooper())

    private var engineHandle = 0L
    private var projection: MediaProjection? = null
    private var projectionResult: Intent? = null
    private val workers = mutableListOf<CaptureWorker>()
    private var appliedSettings: Settings? = null

    override fun onCreate() {
        super.onCreate()
        prefs = Prefs.get(this)
        overlay = OverlayController(this)
        createChannel()

        // Visual settings apply live; structural ones (which sources, which
        // model, which spoken language) restart capture -- the same split
        // the desktop app makes, and for the same reason: restarting costs
        // a second of deafness, and dragging a colour slider shouldn't.
        lifecycleScope.launch {
            prefs.settings.collect { settings ->
                val previous = appliedSettings
                appliedSettings = settings
                overlay.apply(settings)
                workers.forEach { it.setSensitivity(settings.sensitivity) }
                if (previous != null && Prefs.needsRestart(previous, settings)) {
                    restartCapture(settings)
                }
                updateNotification(settings)
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        when (intent?.action) {
            ACTION_STOP -> {
                stopEverything()
                stopSelf()
                return START_NOT_STICKY
            }

            ACTION_TOGGLE_PAUSE -> {
                prefs.update { it.copy(paused = !it.paused) }
                return START_STICKY
            }

            ACTION_PREVIEW -> {
                // A caption of each colour, so position and readability can
                // be checked without waiting for someone to speak.
                overlay.attach(prefs.current)
                overlay.show("This is how the other side will look.", null, micSource = false)
                main.postDelayed({
                    overlay.show("Así se verán tus propias palabras.", null, micSource = true)
                }, 700)
                return START_STICKY
            }
        }

        val settings = prefs.current
        projectionResult = intent?.getParcelableExtra(EXTRA_PROJECTION_RESULT) ?: projectionResult

        // Foreground first, then MediaProjection: from Android 14 the
        // projection can only be obtained by a service that is *already*
        // foreground with the mediaProjection type.
        startForegroundWithType(settings, wantsProjection = settings.captureSystem && projectionResult != null)
        running.value = true
        restartCapture(settings)
        return START_STICKY
    }

    override fun onDestroy() {
        stopEverything()
        super.onDestroy()
    }

    // ---- capture ------------------------------------------------------

    private fun restartCapture(settings: Settings) {
        stopWorkers()
        overlay.attach(settings)

        if (settings.paused) {
            status.value = getString(R.string.status_paused)
            updateNotification(settings)
            return
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED
        ) {
            status.value = getString(R.string.status_no_mic_permission)
            updateNotification(settings)
            return
        }
        if (!ModelStore.isDownloaded(this, settings.model)) {
            status.value = getString(R.string.status_model_missing, settings.model)
            updateNotification(settings)
            return
        }

        lifecycleScope.launch(Dispatchers.IO) {
            val engine = ensureEngine(settings.model)
            if (engine == 0L) {
                status.value = getString(R.string.status_model_failed)
                main.post { updateNotification(settings) }
                return@launch
            }
            if (settings.captureMic) {
                startWorker(engine, settings, mic = true)
            }
            if (settings.captureSystem) {
                val projection = ensureProjection()
                if (projection == null) {
                    // Not fatal: the microphone half still works, and the
                    // reason (no screen-capture consent yet) is something
                    // only the user can fix.
                    status.value = getString(R.string.status_no_projection)
                } else {
                    startWorker(engine, settings, mic = false, projection = projection)
                }
            }
            if (workers.isNotEmpty() && status.value != getString(R.string.status_no_projection)) {
                status.value = getString(R.string.status_listening)
            }
            main.post { updateNotification(prefs.current) }

            // Pull the translation models down before the first caption
            // needs them, so the first thing anyone says isn't the sentence
            // that waits on a download.
            val target = settings.targetLanguage
            if (target != "off") {
                for (from in listOf("en", "es", "fr")) {
                    if (from != target) translations.prepare(from, target)
                }
            }
        }
    }

    private fun ensureEngine(model: String): Long {
        if (engineHandle != 0L) return engineHandle
        val file = ModelStore.file(this, model)
        engineHandle = runCatching { NativeEngine.loadModel(file.absolutePath, model) }.getOrDefault(0L)
        return engineHandle
    }

    private fun ensureProjection(): MediaProjection? {
        projection?.let { return it }
        val result = projectionResult ?: return null
        val manager = getSystemService(MediaProjectionManager::class.java)
        val created = runCatching { manager.getMediaProjection(android.app.Activity.RESULT_OK, result) }
            .getOrNull() ?: return null
        // Required from Android 14 before starting capture; also how we
        // learn that the user revoked the permission from the status bar.
        created.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() {
                projection = null
                projectionResult = null
                status.value = getString(R.string.status_projection_stopped)
                main.post {
                    stopWorkers()
                    restartCapture(prefs.current)
                }
            }
        }, main)
        projection = created
        return created
    }

    private fun startWorker(
        engine: Long,
        settings: Settings,
        mic: Boolean,
        projection: MediaProjection? = null,
    ) {
        val record = runCatching {
            if (mic) Capture.microphone() else Capture.playback(projection!!)
        }.getOrNull() ?: run {
            status.value = getString(R.string.status_capture_failed)
            return
        }
        val stream = runCatching { NativeEngine.createStream(engine, settings.sensitivity) }
            .getOrDefault(0L)
        if (stream == 0L) {
            record.release()
            status.value = getString(R.string.status_model_failed)
            return
        }
        val worker = CaptureWorker(record, stream, mic)
        workers += worker
        worker.start()
    }

    private fun stopWorkers() {
        workers.forEach { it.stopAndJoin() }
        workers.clear()
    }

    private fun stopEverything() {
        running.value = false
        stopWorkers()
        overlay.detach()
        projection?.stop()
        projection = null
        if (engineHandle != 0L) {
            NativeEngine.freeModel(engineHandle)
            engineHandle = 0L
        }
        translations.close()
    }

    /**
     * One audio source, start to finish. Reads frames, hands them to the
     * Rust core, and turns whatever comes back into a subtitle, a
     * translation and a transcript line.
     */
    private inner class CaptureWorker(
        private val record: AudioRecord,
        private val streamHandle: Long,
        private val mic: Boolean,
    ) {
        private val stopping = AtomicBoolean(false)
        private val thread = Thread({ run() }, if (mic) "livesubs-mic" else "livesubs-system")

        fun start() {
            record.startRecording()
            thread.start()
        }

        fun setSensitivity(value: Float) {
            if (streamHandle != 0L) NativeEngine.setSensitivity(streamHandle, value)
        }

        fun stopAndJoin() {
            stopping.set(true)
            runCatching { record.stop() }
            // Bounded join: the thread can be inside whisper, and waiting
            // forever would hang the service's own shutdown.
            thread.join(4_000)
            runCatching { record.release() }
            if (streamHandle != 0L) NativeEngine.freeStream(streamHandle)
        }

        private fun run() {
            val frame = FloatArray(NativeEngine.frameSize())
            while (!stopping.get()) {
                var filled = 0
                while (filled < frame.size && !stopping.get()) {
                    val read = record.read(frame, filled, frame.size - filled, AudioRecord.READ_BLOCKING)
                    if (read <= 0) break
                    filled += read
                }
                if (filled < frame.size) continue
                val json = runCatching {
                    NativeEngine.feed(streamHandle, frame, prefs.current.whisperLanguage)
                }.getOrNull()
                CaptionJson.parse(json)?.let { emit(it) }
            }
            // Whatever was mid-sentence when capture stopped still deserves
            // to reach the transcript.
            runCatching { NativeEngine.flush(streamHandle, prefs.current.whisperLanguage) }
                .getOrNull()
                ?.let { CaptionJson.parse(it) }
                ?.let { emit(it) }
        }

        private fun emit(recognition: Recognition) {
            val settings = prefs.current
            val detected = recognition.language.ifEmpty { settings.sourceLanguage }
            var shown = recognition.text
            var original: String? = null
            if (settings.targetLanguage != "off" && detected != settings.targetLanguage) {
                val translated = translations.translate(recognition.text, detected, settings.targetLanguage)
                if (translated != null && translated.trim() != recognition.text.trim()) {
                    original = recognition.text
                    shown = translated
                }
            }
            overlay.show(shown, if (settings.showOriginal) original else null, mic)
            val uri = settings.logUri
            if (settings.logEnabled && uri != null) {
                Transcript.append(
                    context = this@CaptionService,
                    uri = Uri.parse(uri),
                    source = if (mic) "mic" else "system",
                    language = detected,
                    text = original ?: shown,
                    translated = if (original != null) shown else null,
                ).onFailure { status.value = getString(R.string.status_transcript_failed, it.message ?: "") }
            }
        }
    }

    // ---- notification -------------------------------------------------

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.channel_name),
            // Low: this notification is a control panel that lives in the
            // shade for hours, not an alert.
            NotificationManager.IMPORTANCE_LOW,
        ).apply { setShowBadge(false) }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun buildNotification(settings: Settings): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val pauseLabel = if (settings.paused) R.string.action_resume else R.string.action_pause
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_captions)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(status.value)
            .setContentIntent(open)
            .setOngoing(true)
            .setSilent(true)
            .addAction(0, getString(pauseLabel), serviceAction(ACTION_TOGGLE_PAUSE))
            .addAction(0, getString(R.string.action_settings), open)
            .addAction(0, getString(R.string.action_stop), serviceAction(ACTION_STOP))
            .build()
    }

    private fun serviceAction(action: String): PendingIntent = PendingIntent.getService(
        this,
        action.hashCode(),
        Intent(this, CaptionService::class.java).setAction(action),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun startForegroundWithType(settings: Settings, wantsProjection: Boolean) {
        val notification = buildNotification(settings)
        // The declared type must match what the service actually does:
        // claiming mediaProjection without a projection is a
        // SecurityException on Android 14, and claiming only microphone
        // while capturing playback is the same mistake in reverse.
        //
        // The version split is not cosmetic: `FOREGROUND_SERVICE_TYPE_
        // MICROPHONE` only exists from API 30, so naming it on Android 10
        // (this app's minimum, because that is where playback capture
        // starts) would be a `NoSuchFieldError` at startup.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            var type = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            if (wantsProjection) type = type or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
            ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, type)
        } else if (wantsProjection) {
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun updateNotification(settings: Settings) {
        if (!running.value) return
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, buildNotification(settings))
    }

    companion object {
        const val ACTION_START = "dev.lucasmoy.livesubs.START"
        const val ACTION_STOP = "dev.lucasmoy.livesubs.STOP"
        const val ACTION_TOGGLE_PAUSE = "dev.lucasmoy.livesubs.TOGGLE_PAUSE"
        const val ACTION_PREVIEW = "dev.lucasmoy.livesubs.PREVIEW"
        const val EXTRA_PROJECTION_RESULT = "projection_result"

        private const val CHANNEL_ID = "capture"
        private const val NOTIFICATION_ID = 1

        /** Whether the service is up -- the settings screen mirrors this. */
        private val running = MutableStateFlow(false)
        val isRunning: StateFlow<Boolean> get() = running

        /** Human-readable state, shown in the notification and the UI. */
        private val status = MutableStateFlow("")
        val statusText: StateFlow<String> get() = status

        fun start(context: Context, projectionResult: Intent? = null) {
            val intent = Intent(context, CaptionService::class.java).setAction(ACTION_START)
            if (projectionResult != null) {
                intent.putExtra(EXTRA_PROJECTION_RESULT, projectionResult)
            }
            ContextCompat.startForegroundService(context, intent)
        }

        fun stop(context: Context) {
            context.startService(Intent(context, CaptionService::class.java).setAction(ACTION_STOP))
        }

        fun preview(context: Context) {
            context.startService(Intent(context, CaptionService::class.java).setAction(ACTION_PREVIEW))
        }
    }
}
