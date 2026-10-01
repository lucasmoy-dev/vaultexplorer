package dev.lucasmoy.homecloud

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.util.Log
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

/**
 * Keeps the engine alive.
 *
 * Syncing is only worth anything if it happens while the app is closed, and on
 * Android the only way to keep a process running for that is a foreground
 * service with a visible notification. The notification is not an inconvenience
 * to be hidden — it is the honest signal that something is running.
 */
class SyncService : Service() {

    private val engine by lazy { Engine(this) }

    /**
     * Watches the connection so a change of network is acted on rather than
     * waited out.
     *
     * Moving between wifi and mobile data leaves sockets that one side still
     * believes in and the other has given up on — the desktop saying
     * "connected" while the phone says "disconnected". Nothing recovers from
     * that on its own quickly, so a change dials again.
     *
     * Only the *default* network counts. This used to listen to every network
     * with internet, so mobile data coming up or dropping in the background —
     * which a phone sitting on home wifi does constantly — tore down a
     * perfectly good LAN connection each time, and registering the callback
     * at startup did it once more for each network already up. Android
     * reports the current default once on registration; that first report is
     * where we are, not a change. [lastMetered] still makes a flip of
     * "metered" on the same network count, for the wifi-only folders.
     */
    private var lastMetered: Boolean? = null
    private var lastDefault: Network? = null
    private var sawFirstDefault = false
    private var lostDefault = false

    /** Cleared on the way out, so the notification loop stops with the service. */
    @Volatile
    private var watching = true

    /**
     * Without this the phone never hears the other devices' "I'm here"
     * broadcasts: Android drops multicast and broadcast packets for apps that
     * do not hold one, to save battery. The permission was declared and the
     * lock never taken, so the phone could only find the computer on the
     * same wifi through the internet discovery servers — and when those
     * came back late, a relay got there first.
     */
    private var multicastLock: WifiManager.MulticastLock? = null

    private val networkWatcher = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val first = !sawFirstDefault
            sawFirstDefault = true
            val changed = lostDefault || lastDefault != network
            lastDefault = network
            lostDefault = false
            onNetworkChanged(force = !first && changed)
        }
        override fun onLost(network: Network) {
            if (network != lastDefault) return
            lostDefault = true
            onNetworkChanged(force = true)
        }
        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) =
            onNetworkChanged(force = false)
    }

    override fun onCreate() {
        super.onCreate()
        createChannel()
        startForeground(NOTIFICATION_ID, buildNotification(Progress("HomeCloud", "Arrancando…", null)))
        Log.i("HomeCloudStartup", "service created at +${System.currentTimeMillis() - Startup.t0}ms")
        hearTheLocalNetwork()
        Thread {
            engine.start()
                .onSuccess {
                    tellCoreWhereThingsGo()
                    nameThisPhone()
                    watchTheNetwork()
                    keepDeletionsRecoverable()
                    watchProgress()
                    watchTheEngine()
                }
                .onFailure { notify(it.message ?: "El motor de sincronización no arrancó") }
        }.start()
    }

    private fun hearTheLocalNetwork() {
        runCatching {
            val wifi = applicationContext.getSystemService(WifiManager::class.java) ?: return
            multicastLock = wifi.createMulticastLock("homecloud-discovery").apply {
                setReferenceCounted(false)
                acquire()
            }
        }.onFailure { Log.w("HomeCloud", "no multicast lock: ${it.message}") }
    }

    /**
     * Starts the engine again if it dies.
     *
     * It runs with `--no-restart`, so nothing else would: the service stayed
     * up, the notification said all was well, and every screen answered
     * "could not reach the sync engine" until the app was force-closed. A
     * new engine gets a new port and [Engine.start] hands it to the core, so
     * nothing above notices beyond a moment of "Conectando". Gives up after a
     * few deaths in a row instead of looping on something broken.
     */
    private fun watchTheEngine() {
        Thread {
            val recent = ArrayDeque<Long>()
            while (watching) {
                Thread.sleep(ENGINE_CHECK_MS)
                if (!watching) break
                val reason = engine.exitReason() ?: continue
                Log.w("HomeCloud", "$reason; starting it again")
                val now = System.currentTimeMillis()
                while (recent.isNotEmpty() && now - recent.first() > 120_000) recent.removeFirst()
                recent.addLast(now)
                if (recent.size > 3) {
                    notify("El motor de sincronización se detiene una y otra vez. Abre HomeCloud y pulsa Reintentar.")
                    break
                }
                engine.start()
                    .onSuccess { tellCoreWhereThingsGo() }
                    .onFailure { notify(it.message ?: "El motor de sincronización no arrancó") }
            }
        }.apply { isDaemon = true }.start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // The only way to stop syncing: the action on the notification. Closing
        // the app, or swiping it out of recents, deliberately does not.
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        // Restarted by the system after being killed: the whole point is that
        // syncing resumes without the user opening anything.
        return START_STICKY
    }

    override fun onDestroy() {
        watching = false
        runCatching { multicastLock?.release() }
        runCatching {
            getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(networkWatcher)
        }
        engine.stop()
        super.onDestroy()
    }

    /**
     * Android 15+ caps a `dataSync` foreground service at roughly six hours of
     * execution in a rolling day and calls this once that budget runs out,
     * expecting [stopSelf] soon after — a service that ignores it has its
     * foreground status revoked outright, which is indistinguishable from a
     * crash. This app is meant to run for as long as the phone is on, so
     * instead of actually stopping, a fresh instance is asked to take over
     * immediately: the restart Android's own docs describe for this limit.
     */
    override fun onTimeout(startId: Int, fgsType: Int) {
        stopSelf(startId)
        start(applicationContext)
    }

    /**
     * Reacts to the connection changing: reconnects, and applies each folder's
     * "only on wifi" preference to the network there is now.
     *
     * `force` is true for an actual network appearing or disappearing;
     * otherwise this only acts when "metered" itself flipped, so a capability
     * tick that changes nothing relevant is a no-op instead of a fresh
     * pause-then-resume of every device.
     */
    private fun onNetworkChanged(force: Boolean) {
        Thread {
            runCatching {
                val manager = getSystemService(ConnectivityManager::class.java)
                val capabilities = manager.getNetworkCapabilities(manager.activeNetwork)
                val metered = capabilities?.hasCapability(
                    NetworkCapabilities.NET_CAPABILITY_NOT_METERED
                ) != true
                val before = lastMetered
                if (!force && metered == before) return@runCatching
                lastMetered = metered
                Repo.applyMeteredPolicy(metered)
                // The first reading is where we already are: the engine has
                // just dialled everyone, and tearing that down would only make
                // startup slower.
                if (force || before != null) Repo.reconnectAll()
            }
        }.start()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    /**
     * Gives the phone a name other devices can tell apart.
     *
     * The engine falls back to the system hostname, and on Android that is
     * `localhost` on every device ever made. Two phones then look identical in
     * the one place it matters: deciding whether to trust the one asking to
     * connect. `Build.MODEL` is not perfect, but it is never a lie and never
     * the same word for everyone.
     */
    /**
     * Folders accepted without anyone being asked need somewhere to land, and
     * the app's own preferences need somewhere to live. Only the platform knows
     * either path, so it hands both to the core once the engine is up.
     */
    private fun tellCoreWhereThingsGo() {
        runCatching {
            val root = java.io.File(android.os.Environment.getExternalStorageDirectory(), "HomeCloud")
            root.mkdirs()
            Repo.setRoots(root.absolutePath, java.io.File(filesDir, "homecloud.json").absolutePath)
        }
        runCatching {
            // Same reason the engine lives here: since Android 10 nothing may
            // be executed from an app's data directory, and the native library
            // directory is the one place left that allows it.
            val nativeDir = applicationInfo.nativeLibraryDir
            Repo.linkSetup(
                java.io.File(nativeDir, "libhcshare.so").absolutePath,
                java.io.File(nativeDir, "libcloudflared.so").absolutePath,
            )
        }
    }

    private fun nameThisPhone() {
        runCatching {
            val fallback = listOf(Build.MANUFACTURER, Build.MODEL)
                .filter { it.isNotBlank() }
                .joinToString(" ")
                .trim()
                .ifEmpty { "Mi teléfono" }
            Repo.ensureDeviceName(fallback.replaceFirstChar(Char::titlecase))
        }
    }

    private fun watchTheNetwork() {
        runCatching {
            getSystemService(ConnectivityManager::class.java)
                .registerDefaultNetworkCallback(networkWatcher)
        }
    }

    /**
     * Keeps the notification saying what is actually happening.
     *
     * This is the only thing most people ever see of a sync that runs for
     * hours with the app closed, and it used to read "Sincronizando tus
     * carpetas" whether it was moving a gigabyte, waiting for wifi, or had
     * finished an hour ago. It is re-read every couple of seconds while
     * something is moving and every ten when nothing is, and only redrawn when
     * the words change: a notification that rewrites itself constantly is one
     * Android starts throttling.
     */
    private fun watchProgress() {
        Thread {
            var last: String? = null
            while (watching) {
                val progress = runCatching { notificationProgress(Repo.folders()) }.getOrNull()
                if (progress == null) {
                    Thread.sleep(SLOW_TICK_MS)
                    continue
                }
                val key = "${progress.title}|${progress.detail}|${progress.percent}"
                if (key != last) {
                    last = key
                    show(progress)
                }
                Thread.sleep(if (progress.percent != null) BUSY_TICK_MS else SLOW_TICK_MS)
            }
        }.start()
    }

    /**
     * Makes sure a deletion arriving from another device can be undone.
     *
     * On a phone that means the engine keeps its own copy beside the files:
     * Android has no recycle bin an app may write to in the background, so
     * "Ficheros borrados" reads from there instead.
     */
    private fun keepDeletionsRecoverable() {
        runCatching { Repo.ensureDeletionPolicy() }
    }

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Sincronización",
            // Low: it must be visible, but it is not news.
            NotificationManager.IMPORTANCE_LOW,
        ).apply { description = "Mantiene tus carpetas sincronizadas en segundo plano" }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun buildNotification(progress: Progress): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        // The only exit. Closing the app does not stop syncing, so there has to
        // be somewhere that does, and it has to be somewhere findable.
        val stop = PendingIntent.getService(
            this,
            1,
            Intent(this, SyncService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val builder = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(progress.title)
            .setContentText(progress.detail)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentIntent(open)
            .setOngoing(true)
            .setSilent(true)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Detener HomeCloud", stop)
        // A bar only while there is something to fill it. One stuck at a
        // number, or spinning next to "Todo al día", is worse than none.
        progress.percent?.let { builder.setProgress(100, it, false) }
        return builder.build()
    }

    private fun show(progress: Progress) {
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, buildNotification(progress))
    }

    private fun notify(text: String) = show(Progress("HomeCloud", text, null))

    companion object {
        /** How often the notification is re-read while something is moving. */
        private const val BUSY_TICK_MS = 2_000L
        /** And while nothing is, where the battery matters more than the wait. */
        private const val SLOW_TICK_MS = 10_000L
        /** How often the engine is checked for having died. */
        private const val ENGINE_CHECK_MS = 3_000L

        private const val CHANNEL_ID = "sync"
        private const val NOTIFICATION_ID = 1
        const val ACTION_STOP = "dev.lucasmoy.homecloud.STOP"

        fun start(context: Context) {
            val intent = Intent(context, SyncService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        /**
         * Stops whatever instance is running and starts a fresh one, so a stuck
         * or failed engine start gets a real second attempt instead of the user
         * having to force-close the app. A plain [start] on an instance that is
         * already alive does not do this: `onCreate`, where the engine actually
         * launches, only ever runs once per instance.
         */
        fun restart(context: Context) {
            context.stopService(Intent(context, SyncService::class.java))
            start(context)
        }
    }
}
