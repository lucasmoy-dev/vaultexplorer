package dev.lucasmoy.livesubs

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.core.content.FileProvider
import org.json.JSONArray
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * In-app updates from GitHub Releases -- the same mechanism the other
 * Android apps in this monorepo use (see `life-framework/app/src-tauri/src/
 * update.rs`), so there is one convention to remember instead of one per
 * app.
 *
 * How the convention works: the repo holds several projects, so a release
 * belongs to an app by its **tag prefix**, and the built APK is attached to
 * that release as an asset. This app looks for `livesubs-vX.Y.Z`, takes the
 * highest version among them, and compares it with its own.
 *
 * Installing an APK from inside an app is a three-step dance Android
 * insists on, and each step can fail on its own:
 *
 * 1. `REQUEST_INSTALL_PACKAGES` in the manifest, plus the user allowing
 *    "install unknown apps" for *this* app (a per-app toggle in Settings,
 *    not a runtime dialog) -- so [canInstall] is checked before downloading
 *    anything, and [installPermissionIntent] opens the right screen.
 * 2. A `content://` URI from a `FileProvider`: a `file://` URI to an APK
 *    has been rejected by the installer since Android 7.
 * 3. The install itself is the *system* installer's confirmation screen.
 *    This app never installs anything silently, and can't.
 */
object Updater {
    /**
     * The monorepo's current name on GitHub. Releases (and their APK
     * assets) live here. If it is ever renamed to `personal-projects`,
     * GitHub's redirect keeps this working without a rebuild -- the same
     * note the sibling app's updater carries.
     */
    private const val REPO = "lucasmoy-dev/vaultexplorer"
    private const val TAG_PREFIX = "livesubs-v"

    data class Available(
        val current: String,
        val latest: String,
        val notes: String,
        val apkUrl: String,
        val pageUrl: String,
        val hasUpdate: Boolean,
    )

    val releasesPage: String get() = "https://github.com/$REPO/releases"

    /**
     * Ask GitHub what the newest release for this app is. Blocking: callers
     * run it off the main thread.
     */
    fun check(currentVersion: String): Result<Available> = runCatching {
        val url = URL("https://api.github.com/repos/$REPO/releases?per_page=30")
        val connection = (url.openConnection() as HttpURLConnection).apply {
            setRequestProperty("Accept", "application/vnd.github+json")
            setRequestProperty("User-Agent", "livesubs-updater")
            connectTimeout = 20_000
            readTimeout = 20_000
        }
        val body = try {
            if (connection.responseCode !in 200..299) {
                throw IllegalStateException("HTTP ${connection.responseCode}")
            }
            connection.inputStream.bufferedReader().readText()
        } finally {
            connection.disconnect()
        }

        val releases = JSONArray(body)
        var bestVersion: List<Int>? = null
        var best: org.json.JSONObject? = null
        for (index in 0 until releases.length()) {
            val release = releases.optJSONObject(index) ?: continue
            if (release.optBoolean("draft", false)) continue
            val tag = release.optString("tag_name")
            if (!tag.startsWith(TAG_PREFIX)) continue
            val version = semver(tag.removePrefix(TAG_PREFIX))
            if (bestVersion == null || newer(version, bestVersion)) {
                bestVersion = version
                best = release
            }
        }

        if (best == null || bestVersion == null) {
            return@runCatching Available(
                current = currentVersion,
                latest = currentVersion,
                notes = "",
                apkUrl = "",
                pageUrl = releasesPage,
                hasUpdate = false,
            )
        }

        // The APK among the release's assets. A release can carry other
        // files (a desktop bundle, checksums); the .apk is the one this app
        // can install.
        val assets = best.optJSONArray("assets")
        var apkUrl = ""
        if (assets != null) {
            for (index in 0 until assets.length()) {
                val asset = assets.optJSONObject(index) ?: continue
                if (asset.optString("name").lowercase().endsWith(".apk")) {
                    apkUrl = asset.optString("browser_download_url")
                    break
                }
            }
        }
        Available(
            current = currentVersion,
            latest = bestVersion.joinToString("."),
            notes = best.optString("body").take(400),
            apkUrl = apkUrl,
            pageUrl = best.optString("html_url").ifEmpty { releasesPage },
            hasUpdate = newer(bestVersion, semver(currentVersion)),
        )
    }

    /** Download the APK into the cache, reporting a 0..1 fraction. */
    fun download(context: Context, url: String, onProgress: (Float) -> Unit): File {
        val dir = File(context.cacheDir, "updates").apply { mkdirs() }
        // One file, overwritten: keeping old APKs around would quietly eat
        // tens of MB of the user's storage for nothing.
        val target = File(dir, "livesubs-update.apk")
        val part = File(dir, "livesubs-update.apk.part")
        val connection = (URL(url).openConnection() as HttpURLConnection).apply {
            instanceFollowRedirects = true
            connectTimeout = 30_000
            readTimeout = 60_000
        }
        try {
            if (connection.responseCode !in 200..299) {
                throw IllegalStateException("HTTP ${connection.responseCode}")
            }
            val total = connection.contentLengthLong
            connection.inputStream.use { input ->
                part.outputStream().use { output ->
                    val buffer = ByteArray(256 * 1024)
                    var done = 0L
                    while (true) {
                        val read = input.read(buffer)
                        if (read <= 0) break
                        output.write(buffer, 0, read)
                        done += read
                        if (total > 0) onProgress(done.toFloat() / total)
                    }
                }
            }
        } finally {
            connection.disconnect()
        }
        if (target.exists()) target.delete()
        if (!part.renameTo(target)) throw IllegalStateException("no se pudo guardar el APK")
        onProgress(1f)
        return target
    }

    /** Whether this app may ask the system to install a package. */
    fun canInstall(context: Context): Boolean =
        context.packageManager.canRequestPackageInstalls()

    /** The per-app "install unknown apps" screen for this app. */
    fun installPermissionIntent(context: Context): Intent =
        Intent(
            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:${context.packageName}"),
        )

    /**
     * Hand the downloaded APK to the system installer. The user still
     * confirms on the installer's own screen -- this only opens it.
     */
    fun installIntent(context: Context, apk: File): Intent {
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.updates", apk)
        return Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
    }

    /** `"0.2.10"` to `[0, 2, 10]`, ignoring any `-beta`/`+build` suffix. */
    internal fun semver(text: String): List<Int> {
        val core = text.trim().substringBefore('-').substringBefore('+')
        val parts = core.split('.').map { it.trim().toIntOrNull() ?: 0 }
        return listOf(
            parts.getOrElse(0) { 0 },
            parts.getOrElse(1) { 0 },
            parts.getOrElse(2) { 0 },
        )
    }

    /** Strictly newer, component by component. */
    internal fun newer(candidate: List<Int>, than: List<Int>): Boolean {
        for (index in 0 until 3) {
            val a = candidate.getOrElse(index) { 0 }
            val b = than.getOrElse(index) { 0 }
            if (a != b) return a > b
        }
        return false
    }
}
