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
import android.net.NetworkRequest
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
     * that on its own quickly, so every change dials again.
     */
    private val networkWatcher = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = onNetworkChanged()
        override fun onLost(network: Network) = onNetworkChanged()
        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) =
            onNetworkChanged()
    }

    override fun onCreate() {
        super.onCreate()
        createChannel()
        startForeground(NOTIFICATION_ID, buildNotification("Arrancando…"))
        Thread {
            engine.start()
                .onSuccess {
                    tellCoreWhereThingsGo()
                    nameThisPhone()
                    watchTheNetwork()
                    notify("Sincronizando tus carpetas")
                }
                .onFailure { notify(it.message ?: "El motor de sincronización no arrancó") }
        }.start()
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
        runCatching {
            getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(networkWatcher)
        }
        engine.stop()
        super.onDestroy()
    }

    /**
     * Reacts to the connection changing: reconnects, and applies each folder's
     * "only on wifi" preference to the network there is now.
     */
    private fun onNetworkChanged() {
        Thread {
            runCatching {
                val manager = getSystemService(ConnectivityManager::class.java)
                val capabilities = manager.getNetworkCapabilities(manager.activeNetwork)
                val metered = capabilities?.hasCapability(
                    NetworkCapabilities.NET_CAPABILITY_NOT_METERED
                ) != true
                Repo.applyMeteredPolicy(metered)
                Repo.reconnectAll()
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
            val helper = java.io.File(applicationInfo.nativeLibraryDir, "libhcshare.so")
            val home = java.io.File(filesDir, "zrok").apply { mkdirs() }
            Repo.linkSetup(helper.absolutePath, home.absolutePath)
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
            val request = NetworkRequest.Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .build()
            getSystemService(ConnectivityManager::class.java)
                .registerNetworkCallback(request, networkWatcher)
        }
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

    private fun buildNotification(text: String): Notification {
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
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("HomeCloud")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentIntent(open)
            .setOngoing(true)
            .setSilent(true)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Detener HomeCloud", stop)
            .build()
    }

    private fun notify(text: String) {
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, buildNotification(text))
    }

    companion object {
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
    }
}
