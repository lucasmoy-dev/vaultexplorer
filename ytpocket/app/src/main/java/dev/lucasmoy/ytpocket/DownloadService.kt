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
        if (kind == KIND_ORGANISE) {
            startForegroundWithType(getString(R.string.organise_running), 0f)
            lifecycleScope.launch(Dispatchers.IO) {
                try {
                    queue.withLock { organise() }
                } finally {
                    waiting.value = (waiting.value - 1).coerceAtLeast(0)
                }
                if (!queue.isLocked) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                }
            }
            return START_NOT_STICKY
        }
        if (videoId.isNullOrEmpty() || kind.isNullOrEmpty()) {
            waiting.value = (waiting.value - 1).coerceAtLeast(0)
            stopSelf()
            return START_NOT_STICKY
        }
        val title = intent.getStringExtra(EXTRA_TITLE).orEmpty()
        startForegroundWithType(getString(R.string.download_preparing, title.ifEmpty { videoId }), 0f)

        lifecycleScope.launch(Dispatchers.IO) {
            try {
                queue.withLock { run(videoId, kind == KIND_MP3, title) }
            } finally {
                waiting.value = (waiting.value - 1).coerceAtLeast(0)
            }
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
            var displayName: String
            var tagged: Native.Tagged? = null
            var tagError: String? = null
            if (wantMp3) {
                val audio = resolved.audio ?: throw IllegalStateException(getString(R.string.error_no_audio))
                val source = File(work, "$videoId-audio.${audio.ext}")
                parts += source
                fetchWithProgress(
                    videoId = videoId,
                    pick = { it.audio },
                    resolved = resolved,
                    target = source,
                    title = title,
                    step = getString(R.string.download_audio),
                    from = 0f,
                    to = 0.7f,
                )

                update(getString(R.string.download_converting, title), 0.75f)
                progress.value = Progress(title, getString(R.string.download_converting_short), 0.75f)
                val mp3 = File(work, "$videoId.mp3")
                parts += mp3
                // The transcode is the one step without real progress (see
                // the native side's note); it is also the shortest.
                Native.toMp3(source.absolutePath, mp3.absolutePath, title, artist)
                finished = mp3
                displayName = Native.nameFor(title, "mp3")
                if (Settings.autoOrganise(this)) {
                    update(getString(R.string.download_tagging, title), 0.85f)
                    progress.value = Progress(title, getString(R.string.download_tagging_short), 0.85f)
                    // Best effort, but never silent: a file whose lookup
                    // failed still lands, named after the video, and the
                    // result line says the tags are YouTube's only.
                    tagged = runCatching {
                        Native.tag(
                            mp3.absolutePath,
                            Native.Hint(
                                videoId = videoId,
                                title = title,
                                channel = artist,
                                duration = resolved.duration,
                                description = resolved.description,
                            ),
                            Library.country(),
                        )
                    }.onFailure { tagError = it.message }.getOrNull()
                }
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
                fetchWithProgress(
                    videoId = videoId,
                    pick = { it.video },
                    resolved = resolved,
                    target = videoPart,
                    title = title,
                    step = getString(R.string.download_video, video.height),
                    from = 0f,
                    to = 0.8f,
                )
                fetchWithProgress(
                    videoId = videoId,
                    pick = { it.audio },
                    resolved = resolved,
                    target = audioPart,
                    title = title,
                    step = getString(R.string.download_audio),
                    from = 0.8f,
                    to = 0.9f,
                )

                update(getString(R.string.download_muxing, title), 0.92f)
                progress.value = Progress(title, getString(R.string.download_muxing_short), 0.92f)
                val muxed = File(work, "$videoId.mp4")
                parts += muxed
                Downloads.mux(videoPart, audioPart, muxed)
                finished = muxed
                displayName = Native.nameFor(title, "mp4")
            }

            tagged?.let { displayName = it.fileName }
            update(getString(R.string.download_saving, displayName), 0.97f)
            val placed = Library.publish(this, finished, displayName, audio = wantMp3, folder = tagged?.folder.orEmpty())
            progress.value = null
            val details = listOfNotNull(
                tagged?.folder?.takeIf { it.isNotEmpty() }?.let { "$it/" },
                tagged?.let { getString(R.string.tags_from, sourceLabel(it.source)) },
                tagError?.let { getString(R.string.tags_failed, it) },
                getString(R.string.saved_default_instead).takeIf { placed.fellBack },
                resolved.client,
            ).joinToString(" · ")
            lastResult.value = Result("$displayName · $details", placed.uri, null, audio = wantMp3)
            notifyDone(displayName, placed.uri, wantMp3)
        } catch (error: Throwable) {
            progress.value = null
            val message = (error.message ?: error::class.java.simpleName) + " · v" + BuildConfig.VERSION_NAME
            lastResult.value = Result(label, null, message, audio = wantMp3)
            notifyFailed(label, message)
        } finally {
            // The parts are an implementation detail; leaving them behind
            // would quietly fill the cache with hundreds of MB.
            parts.forEach { it.delete() }
        }
    }

    /** "Ordenar mis MP3", in the same queue as downloads so the two never touch one file at once. */
    private fun organise() {
        val label = getString(R.string.organise_running)
        try {
            progress.value = Progress(label, getString(R.string.organise_listing), 0f)
            val tree = Settings.tree(this, audio = true)
            val organizer = Organizer(
                shelf = Library.mp3Shelf(this),
                tagger = Library.NativeTagger(),
                work = Downloads.workDir(this),
                onlyOurs = tree != null,
            )
            val summary = organizer.run { done, total, name ->
                val fraction = if (total > 0) done.toFloat() / total else 1f
                val step = getString(R.string.organise_step, done + 1, total, name)
                if (done < total) {
                    progress.value = Progress(label, step, fraction)
                    update(step, fraction)
                }
            }
            progress.value = null
            val text = summaryText(summary)
            // Shown as a failure only when nothing worked at all; one bad
            // file among a hundred is a line in the summary, not an error.
            val allFailed = summary.total > 0 && summary.failed == summary.total
            lastResult.value = Result(text, null, text.takeIf { allFailed }, audio = true, organised = true)
            notifyText(getString(R.string.organise_done), text)
        } catch (error: Throwable) {
            progress.value = null
            val message = (error.message ?: error::class.java.simpleName) + " · v" + BuildConfig.VERSION_NAME
            lastResult.value = Result(label, null, message, audio = true, organised = true)
            notifyFailed(label, message)
        }
    }

    private fun summaryText(summary: Organizer.Summary): String = buildList {
        if (summary.total == 0) {
            add(getString(R.string.organise_empty))
            return@buildList
        }
        add(getString(R.string.organise_count, summary.organised, summary.total))
        if (summary.matched > 0) add(getString(R.string.organise_matched, summary.matched))
        if (summary.fromTitle > 0) add(getString(R.string.organise_from_title, summary.fromTitle))
        if (summary.unchanged > 0) add(getString(R.string.organise_unchanged, summary.unchanged))
        if (summary.foreign > 0) add(getString(R.string.organise_foreign, summary.foreign))
        if (summary.failed > 0) add(getString(R.string.organise_failed, summary.failed, summary.firstError.orEmpty()))
    }.joinToString(" · ")

    private fun sourceLabel(source: String): String = when (source) {
        "itunes" -> "iTunes"
        "youtube_music" -> "YouTube Music"
        else -> "YouTube"
    }

    /**
     * Fetch one stream, and if YouTube refuses part way through, resolve the
     * video again and try once with fresh URLs.
     *
     * Worth the retry rather than failing: a googlevideo URL is short-lived
     * and tied to the client that minted it, so "403 half way" is a normal
     * thing to recover from, not a reason to make the user start over. The
     * retry is bounded at one -- a second refusal is a real problem, and
     * looping would just hammer YouTube.
     */
    private fun fetchWithProgress(
        videoId: String,
        pick: (Native.Resolved) -> Native.Stream?,
        resolved: Native.Resolved,
        target: File,
        title: String,
        step: String,
        from: Float,
        to: Float,
    ) {
        val stream = pick(resolved) ?: throw IllegalStateException(getString(R.string.error_no_audio))
        val report: (Float) -> Unit = { fraction ->
            val overall = from + (to - from) * fraction
            update("$step · $title", overall)
            progress.value = Progress(title, step, overall)
        }
        // No outer restart any more: the fetch resumes in place. A rotating
        // phone IP invalidates the URL, not the bytes already on disk, so
        // re-resolving and continuing beats downloading the first 40MB again.
        Downloads.fetch(
            stream.url, target, resolved.userAgent, stream.size, report,
            refresh = {
                update(getString(R.string.download_retrying, title), from)
                val fresh = runCatching { Native.resolveVideo(videoId) }.getOrNull()
                    ?: return@fetch null
                val refreshed = pick(fresh) ?: return@fetch null
                refreshed.url to fresh.userAgent
            },
        )
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

    private fun notifyText(title: String, text: String) {
        val notification = NotificationCompat.Builder(this, CHANNEL_DONE)
            .setSmallIcon(R.drawable.ic_download)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        getSystemService(NotificationManager::class.java).notify(title.hashCode(), notification)
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
    data class Result(
        val name: String,
        val uri: Uri?,
        val error: String?,
        val audio: Boolean = true,
        /** A summary of "Ordenar mis MP3" rather than one file. */
        val organised: Boolean = false,
    )

    companion object {
        private const val CHANNEL_PROGRESS = "downloads"
        private const val CHANNEL_DONE = "finished"
        private const val NOTIFICATION_PROGRESS = 1
        private const val EXTRA_VIDEO_ID = "video_id"
        private const val EXTRA_KIND = "kind"
        private const val EXTRA_TITLE = "title"
        const val KIND_MP3 = "mp3"
        const val KIND_MP4 = "mp4"
        const val KIND_ORGANISE = "organise"

        /** What is downloading right now, for the UI to mirror. */
        private val progress = MutableStateFlow<Progress?>(null)
        val current: StateFlow<Progress?> get() = progress

        /** The last thing that finished or failed, so the screen can say so. */
        private val lastResult = MutableStateFlow<Result?>(null)
        val last: StateFlow<Result?> get() = lastResult

        /**
         * How many jobs are queued or running. The UI reads it to say "en
         * cola, va después de 2" the moment a button is tapped -- a tap that
         * only shows up as a notification minutes later feels like a tap that
         * did nothing.
         */
        private val waiting = MutableStateFlow(0)
        val queued: StateFlow<Int> get() = waiting

        /** Queue a download, and answer how many jobs are ahead of it. */
        fun start(context: Context, videoId: String, title: String, kind: String): Int {
            val intent = Intent(context, DownloadService::class.java)
                .putExtra(EXTRA_VIDEO_ID, videoId)
                .putExtra(EXTRA_TITLE, title)
                .putExtra(EXTRA_KIND, kind)
            val ahead = waiting.value
            waiting.value = ahead + 1
            ContextCompat.startForegroundService(context, intent)
            return ahead
        }

        /** Queue "Ordenar mis MP3". */
        fun organise(context: Context): Int {
            val ahead = waiting.value
            waiting.value = ahead + 1
            ContextCompat.startForegroundService(
                context,
                Intent(context, DownloadService::class.java).putExtra(EXTRA_KIND, KIND_ORGANISE),
            )
            return ahead
        }
    }
}
