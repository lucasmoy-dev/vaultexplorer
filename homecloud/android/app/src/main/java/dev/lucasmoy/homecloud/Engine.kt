package dev.lucasmoy.homecloud

import android.content.Context
import android.util.Log
import java.io.File
import java.net.ServerSocket
import kotlin.random.Random

/**
 * Owns the Syncthing process on Android.
 *
 * The engine is shipped as `libsyncthing.so` and run out of `nativeLibraryDir`
 * for one reason: since Android 10 the system refuses to execute anything from
 * an app's own data directory, and the native library directory is the one place
 * left where an app may keep a real executable.
 */
class Engine(private val context: Context) {

    private var process: Process? = null
    var baseUrl: String? = null
        private set

    val isRunning: Boolean get() = process?.isAlive == true

    private val binary: File
        get() = File(context.applicationInfo.nativeLibraryDir, "libsyncthing.so")

    private val home: File
        get() = File(context.filesDir, "engine")

    fun start(): Result<Unit> = runCatching {
        if (isRunning) return@runCatching
        check(binary.exists()) { "the sync engine is missing from this build" }
        home.mkdirs()

        if (!File(home, "config.xml").exists()) {
            generateIdentity()
        }

        val port = freePort()
        val apiKey = randomSecret()
        val url = "http://127.0.0.1:$port"

        process = ProcessBuilder(
            binary.absolutePath, "serve",
            "--home", home.absolutePath,
            "--gui-address", "127.0.0.1:$port",
            "--gui-apikey", apiKey,
            "--no-browser",
            // The engine must never replace the binary that was signed and
            // installed alongside this app.
            "--no-upgrade",
            "--no-restart",
            "--log-level", "WARN",
        ).redirectErrorStream(true).start()

        drainOutput(process!!)
        baseUrl = url
        Native.connect(url, apiKey)
        waitUntilReady()
    }

    /**
     * First run only: mints this device's certificate and identity. The web
     * interface gets a random password nobody keeps, because HomeCloud
     * authenticates with the API key and an unprotected interface on localhost
     * would be reachable by every other app on the phone.
     */
    private fun generateIdentity() {
        val generate = ProcessBuilder(
            binary.absolutePath, "generate",
            "--home", home.absolutePath,
            "--gui-user", "homecloud",
            "--gui-password", randomSecret(),
        ).redirectErrorStream(true).start()
        drainOutput(generate)
        check(generate.waitFor() == 0) { "could not create this device's identity" }
    }

    private fun waitUntilReady() {
        val deadline = System.currentTimeMillis() + STARTUP_TIMEOUT_MS
        while (System.currentTimeMillis() < deadline) {
            runCatching { Native.request("ping") }.onSuccess { return }
            if (process?.isAlive != true) error("the sync engine stopped while starting")
            Thread.sleep(200)
        }
        error("the sync engine started but never answered")
    }

    fun stop() {
        process?.destroy()
        process?.waitFor()
        process = null
        baseUrl = null
    }

    /**
     * A process whose output nobody reads eventually blocks on a full pipe and
     * stops syncing, which would look like a mysterious hang rather than a
     * forgotten stream.
     */
    private fun drainOutput(target: Process) {
        Thread {
            target.inputStream.bufferedReader().useLines { lines ->
                lines.forEach { Log.i(TAG, it) }
            }
        }.apply { isDaemon = true }.start()
    }

    private fun freePort(): Int = ServerSocket(0).use { it.localPort }

    private fun randomSecret(): String {
        val chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        return (1..40).map { chars[Random.nextInt(chars.length)] }.joinToString("")
    }

    private companion object {
        const val TAG = "HomeCloudEngine"
        const val STARTUP_TIMEOUT_MS = 40_000L
    }
}
