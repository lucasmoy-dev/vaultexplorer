package dev.lucasmoy.ytpocket

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.File

/**
 * Downloads, in a foreground service.
 *
 * Not on the activity's coroutine scope, deliberately: a 200MB 1080p video
 * takes minutes, and people put the phone in their pocket. Android kills
 * background work for an app whose UI is gone unless it is a foreground
 * service, so the download lives here and the notification shows the
 * progress even with the app closed.
 *
 * One at a time (a `Mutex`): two concurrent downloads on a phone connection
 * finish later than two sequential ones, and the progress notification of
 * "3 things at 40%" tells nobody anything useful.
 */
class DownloadService : LifecycleService() {

    private val queue = Mutex()

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        val videoId = intent?.getStringExtra(EXTRA_VIDEO_ID)
        val kind = intent?.getStringExtra(EXTRA_KIND)
        if (videoId.isNullOrEmpty() || kind.isNullOrEmpty()) {
            stopSelf()
            return START_NOT_STICKY
        }
        val title = intent.getStringExtra(EXTRA_TITLE).orEmpty()
        startForegroundWithType(getString(R.string.download_preparing, title.ifEmpty { videoId }), 0f)

        lifecycleScope.launch(Dispatchers.IO) {
            queue.withLock { run(videoId, kind == KIND_MP3, title) }
            // Nothing else waiting: let the service go rather than sitting
            // in the shade with a stale notification.
            if (!queue.isLocked) {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    private suspend fun run(videoId: String, wantMp3: Boolean, knownTitle: String) {
        val work = Downloads.workDir(this)
        val parts = mutableListOf<File>()
        val label = knownTitle.ifEmpty { videoId }
        try {
            progress.value = Progress(label, getString(R.string.download_resolving), 0f)
            update(getString(R.string.download_resolving_short, label), 0f)
            val resolved = Native.resolveVideo(videoId)
            val title = resolved.title.ifEmpty { knownTitle.ifEmpty { videoId } }
            val artist = resolved.channel

            val finished: File
            val displayName: String
            if (wantMp3) {
                val audio = resolved.audio ?: throw IllegalStateException(getString(R.string.error_no_audio))
                val source = File(work, "$videoId-audio.${audio.ext}")
                parts += source
                fetchWithProgress(audio, source, title, getString(R.string.download_audio), 0f, 0.7f)

                update(getString(R.string.download_converting, title), 0.75f)
                progress.value = Progress(title, getString(R.string.download_converting_short), 0.75f)
                val mp3 = File(work, "$videoId.mp3")
                parts += mp3
                // The transcode is the one step without real progress (see
                // the native side's note); it is also the shortest.
                Native.toMp3(source.absolutePath, mp3.absolutePath, title, artist)
                finished = mp3
                displayName = Native.nameFor(title, "mp3")
            } else {
                val video = resolved.video ?: throw IllegalStateException(getString(R.string.error_no_video))
                val audio = resolved.audio ?: throw IllegalStateException(getString(R.string.error_no_audio))
                val videoPart = File(work, "$videoId-video.${video.ext}")
                val audioPart = File(work, "$videoId-audio.${audio.ext}")
                parts += videoPart
                parts += audioPart
                // Video first and audio second, weighted by how big they
                // are: the video is 20x the audio, and a bar that jumps
                // from 5% to 95% is worse than no bar.
                fetchWithProgress(video, videoPart, title, getString(R.string.download_video, video.height), 0f, 0.8f)
                fetchWithProgress(audio, audioPart, title, getString(R.string.download_audio), 0.8f, 0.9f)

                update(getString(R.string.download_muxing, title), 0.92f)
                progress.value = Progress(title, getString(R.string.download_muxing_short), 0.92f)
                val muxed = File(work, "$videoId.mp4")
                parts += muxed
                Downloads.mux(videoPart, audioPart, muxed)
                finished = muxed
                displayName = Native.nameFor(title, "mp4")
            }

            update(getString(R.string.download_saving, displayName), 0.97f)
            val uri = Downloads.publish(this, finished, displayName, audio = wantMp3)
            progress.value = null
            lastResult.value = Result(displayName, uri, null)
            notifyDone(displayName, uri, wantMp3)
        } catch (error: Throwable) {
            progress.value = null
            val message = error.message ?: error::class.java.simpleName
            lastResult.value = Result(label, null, message)
            notifyFailed(label, message)
        } finally {
            // The parts are an implementation detail; leaving them behind
            // would quietly fill the cache with hundreds of MB.
            parts.forEach { it.delete() }
        }
    }

    private fun fetchWithProgress(
        stream: Native.Stream,
        target: File,
        title: String,
        step: String,
        from: Float,
        to: Float,
    ) {
        Downloads.fetch(stream.url, target, stream.size) { fraction ->
            val overall = from + (to - from) * fraction
            update("$step · $title", overall)
            progress.value = Progress(title, step, overall)
        }
    }

    // ---- notification -------------------------------------------------

    private fun createChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_PROGRESS,
                getString(R.string.channel_downloads),
                // Low: a progress bar that sits in the shade for minutes is
                // not an alert.
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) }
        )
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_DONE,
                getString(R.string.channel_finished),
                NotificationManager.IMPORTANCE_DEFAULT,
            )
        )
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        this,
        0,
        Intent(this, MainActivity::class.java),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun startForegroundWithType(text: String, fraction: Float) {
        val notification = NotificationCompat.Builder(this, CHANNEL_PROGRESS)
            .setSmallIcon(R.drawable.ic_download)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setProgress(100, (fraction * 100).toInt(), fraction <= 0f)
            .setOngoing(true)
            .setSilent(true)
            .setContentIntent(openApp())
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            // Android 14 insists a foreground service declare what it is
            // for; a download is dataSync.
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_PROGRESS,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
            )
        } else {
            startForeground(NOTIFICATION_PROGRESS, notification)
        }
    }

    private fun update(text: String, fraction: Float) {
        val notification = NotificationCompat.Builder(this, CHANNEL_PROGRESS)
            .setSmallIcon(R.drawable.ic_download)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setProgress(100, (fraction * 100).toInt(), false)
            .setOngoing(true)
            .setSilent(true)
            .setContentIntent(openApp())
            .build()
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_PROGRESS, notification)
    }

    private fun notifyDone(name: String, uri: Uri, audio: Boolean) {
        // Tapping it opens the file in whatever plays that kind of thing --
        // the point of the download, one tap away.
        val open = PendingIntent.getActivity(
            this,
            name.hashCode(),
            Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, if (audio) "audio/mpeg" else "video/mp4")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            },
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(this, CHANNEL_DONE)
            .setSmallIcon(R.drawable.ic_download)
            .setContentTitle(getString(R.string.download_done))
            .setContentText(name)
            .setStyle(NotificationCompat.BigTextStyle().bigText(name))
            .setAutoCancel(true)
            .setContentIntent(open)
            .build()
        getSystemService(NotificationManager::class.java).notify(name.hashCode(), notification)
    }

    private fun notifyFailed(label: String, message: String) {
        val notification = NotificationCompat.Builder(this, CHANNEL_DONE)
            .setSmallIcon(R.drawable.ic_download)
            .setContentTitle(getString(R.string.download_failed))
            .setContentText("$label · $message")
            .setStyle(NotificationCompat.BigTextStyle().bigText("$label\n$message"))
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        getSystemService(NotificationManager::class.java).notify(label.hashCode(), notification)
    }

    data class Progress(val title: String, val step: String, val fraction: Float)
    data class Result(val name: String, val uri: Uri?, val error: String?)

    companion object {
        private const val CHANNEL_PROGRESS = "downloads"
        private const val CHANNEL_DONE = "finished"
        private const val NOTIFICATION_PROGRESS = 1
        private const val EXTRA_VIDEO_ID = "video_id"
        private const val EXTRA_KIND = "kind"
        private const val EXTRA_TITLE = "title"
        const val KIND_MP3 = "mp3"
        const val KIND_MP4 = "mp4"

        /** What is downloading right now, for the UI to mirror. */
        private val progress = MutableStateFlow<Progress?>(null)
        val current: StateFlow<Progress?> get() = progress

        /** The last thing that finished or failed, so the screen can say so. */
        private val lastResult = MutableStateFlow<Result?>(null)
        val last: StateFlow<Result?> get() = lastResult

        fun start(context: Context, videoId: String, title: String, kind: String) {
            val intent = Intent(context, DownloadService::class.java)
                .putExtra(EXTRA_VIDEO_ID, videoId)
                .putExtra(EXTRA_TITLE, title)
                .putExtra(EXTRA_KIND, kind)
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
