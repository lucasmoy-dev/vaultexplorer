package dev.lucasmoy.homecloud

import org.json.JSONArray
import org.json.JSONObject

/**
 * The same vocabulary the desktop shows, parsed from what homecore returns.
 * These mirror `homecore::model`; the JSON is produced by serde on that side.
 */

sealed interface FolderState {
    data object UpToDate : FolderState
    data class Syncing(val percent: Int) : FolderState
    data object Paused : FolderState
    data object Disconnected : FolderState
    data class Problem(val detail: String) : FolderState

    companion object {
        fun from(json: JSONObject): FolderState = when (json.optString("kind")) {
            "upToDate" -> UpToDate
            "syncing" -> Syncing(json.optInt("percent"))
            "paused" -> Paused
            "disconnected" -> Disconnected
            "problem" -> Problem(json.optString("detail"))
            else -> Disconnected
        }
    }
}

data class Peer(
    val id: String,
    val name: String,
    val connected: Boolean,
    /** How much of the folder that device has, 0-100, or null if unknown. */
    val completion: Int?,
) {
    companion object {
        fun from(json: JSONObject) = Peer(
            id = json.getString("id"),
            name = json.getString("name"),
            connected = json.getBoolean("connected"),
            completion = if (json.isNull("completion")) null else json.optInt("completion"),
        )
    }
}

data class SharedFolder(
    val id: String,
    val label: String,
    val path: String,
    val state: FolderState,
    val peers: List<Peer>,
    val bytes: Long,
    val files: Long,
    val conflicts: Long,
    val bytesPerSecond: Long,
    val readOnly: Boolean,
    val freeBytes: Long?,
    val pendingBytes: Long,
    val wifiOnly: Boolean,
    val pausedByNetwork: Boolean,
    val hasPassword: Boolean,
    /** Seconds left at the speed measured just now, null when nothing moves. */
    val etaSeconds: Long?,
) {
    companion object {
        fun from(json: JSONObject) = SharedFolder(
            id = json.getString("id"),
            label = json.getString("label"),
            path = json.getString("path"),
            state = FolderState.from(json.getJSONObject("state")),
            peers = json.getJSONArray("peers").map { Peer.from(it) },
            bytes = json.getLong("bytes"),
            files = json.getLong("files"),
            conflicts = json.getLong("conflicts"),
            bytesPerSecond = json.optLong("bytesPerSecond"),
            readOnly = json.optBoolean("readOnly"),
            freeBytes = if (json.isNull("freeBytes")) null else json.optLong("freeBytes"),
            pendingBytes = json.optLong("pendingBytes"),
            wifiOnly = json.optBoolean("wifiOnly"),
            pausedByNetwork = json.optBoolean("pausedByNetwork"),
            hasPassword = json.optBoolean("hasPassword"),
            etaSeconds = if (json.isNull("etaSeconds")) null else json.optLong("etaSeconds"),
        )
    }
}

data class OfferedFolder(val id: String, val label: String)

data class Invitation(
    val fromDeviceId: String,
    val fromDeviceName: String,
    val folder: OfferedFolder?,
) {
    /** Sent straight back to the core, which expects its own shape. */
    fun toJson(): JSONObject = JSONObject().apply {
        put("fromDeviceId", fromDeviceId)
        put("fromDeviceName", fromDeviceName)
        put("folder", folder?.let { JSONObject().put("id", it.id).put("label", it.label) })
    }

    companion object {
        fun from(json: JSONObject) = Invitation(
            fromDeviceId = json.getString("fromDeviceId"),
            fromDeviceName = json.getString("fromDeviceName"),
            folder = json.optJSONObject("folder")?.let {
                OfferedFolder(it.getString("id"), it.getString("label"))
            },
        )
    }
}

data class Settings(
    val deviceName: String,
    val deviceId: String,
    val localNetworkOnly: Boolean,
    val uploadLimitKbps: Int,
    val downloadLimitKbps: Int,
    /**
     * Where a file goes when another device deletes or replaces it: "bin" is
     * the desktop's recycle bin, "copies" the hidden folder beside the files —
     * which is what a phone gets, since Android has no bin an app may write to
     * on its own — and "nothing" destroys it.
     */
    val deletionPolicy: String,
    val keepVersions: Int,
    val engineVersion: String,
    val language: String,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("deviceName", deviceName)
        put("deviceId", deviceId)
        put("localNetworkOnly", localNetworkOnly)
        put("uploadLimitKbps", uploadLimitKbps)
        put("downloadLimitKbps", downloadLimitKbps)
        put("deletionPolicy", deletionPolicy)
        put("keepVersions", keepVersions)
        put("engineVersion", engineVersion)
        put("language", language)
    }

    companion object {
        fun from(json: JSONObject) = Settings(
            deviceName = json.getString("deviceName"),
            deviceId = json.getString("deviceId"),
            localNetworkOnly = json.getBoolean("localNetworkOnly"),
            uploadLimitKbps = json.getInt("uploadLimitKbps"),
            downloadLimitKbps = json.getInt("downloadLimitKbps"),
            deletionPolicy = json.optString("deletionPolicy", "copies"),
            keepVersions = json.getInt("keepVersions"),
            engineVersion = json.optString("engineVersion"),
            language = json.optString("language", "es"),
        )
    }
}

data class CodePreview(val deviceName: String, val folderLabel: String, val bytes: Long?)

/** Kept free so filling a disk does not take the rest of the phone with it. */
const val DISK_RESERVE = 1_000_000_000L

/** How much more room a folder of [needed] bytes wants. Zero when it fits. */
fun shortfall(needed: Long, free: Long): Long =
    maxOf(0L, needed - maxOf(0L, free - DISK_RESERVE))

/**
 * Whether something is about to run out of room. An unknown size or an
 * unreadable disk is never a warning: one that fires without knowing is one
 * people learn to ignore.
 */
fun doesNotFit(needed: Long?, free: Long?): Boolean {
    if (needed == null || free == null || needed <= 0) return false
    return shortfall(needed, free) > 0
}

/**
 * What picking a directory would actually do with a folder arriving from a code.
 *
 * The phone used to append the folder's name to whatever the user chose, so
 * choosing `/sdcard/cloud` for a folder called `cloud` synced an empty
 * `/sdcard/cloud/cloud` beside the real one. Now the decision is named, shown
 * as a sentence, and reversible before anything is written.
 */
data class Destination(
    val path: String,
    val pick: String,
    val explanation: String,
    val freeBytes: Long?,
) {
    val putsItInside: Boolean get() = pick == "inside"

    companion object {
        fun from(json: JSONObject) = Destination(
            path = json.getString("path"),
            pick = json.getString("pick"),
            explanation = json.getString("explanation"),
            freeBytes = if (json.isNull("freeBytes")) null else json.optLong("freeBytes"),
        )
    }
}

/**
 * A transfer rate a person can read, e.g. "2,4 MB/s".
 *
 * The separator is fixed rather than the phone's: `String.format` follows the
 * device's language, so on a phone set to English the speed read "2.4 MB/s"
 * next to a size that says "5,0 GB" — the app contradicting itself in the
 * same line.
 */
fun formatRate(bytesPerSecond: Long): String {
    if (bytesPerSecond <= 0) return ""
    val mb = bytesPerSecond / 1_000_000.0
    return if (mb >= 1) {
        String.format(java.util.Locale.ROOT, "%.1f", mb).replace('.', ',') + " MB/s"
    } else {
        "${bytesPerSecond / 1000} kB/s"
    }
}

/**
 * How long is left, in the roundest terms still worth reading: "2 h 15 min",
 * "8 min", "45 s". A percentage on its own never answers the question people
 * actually have, which is whether to wait for it.
 */
fun formatEta(seconds: Long): String {
    if (seconds >= 36 * 3600) return "más de un día"
    val hours = seconds / 3600
    val minutes = ((seconds % 3600) + 30) / 60
    return when {
        hours > 0 && minutes > 0 -> "$hours h $minutes min"
        hours > 0 -> "$hours h"
        seconds >= 60 -> "${maxOf(1, (seconds + 30) / 60)} min"
        else -> "${maxOf(1, seconds)} s"
    }
}

/** What is left of a sync, as one line: "faltan 2 h 15 min · 15,0 MB/s". */
fun remaining(folder: SharedFolder): String? {
    val parts = buildList {
        folder.etaSeconds?.let { add("faltan ${formatEta(it)}") }
        formatRate(folder.bytesPerSecond).takeIf { it.isNotEmpty() }?.let { add(it) }
    }
    return parts.joinToString(" · ").ifEmpty { null }
}

/**
 * What the notification says while the app is closed.
 *
 * The notification is the only thing most people see of a sync that takes
 * hours, and it used to read "Sincronizando tus carpetas" whether it was
 * moving a gigabyte, waiting for wifi or finished an hour ago. Same words in
 * every state means the words say nothing.
 */
data class Progress(val title: String, val detail: String?, val percent: Int?)

fun notificationProgress(folders: List<SharedFolder>): Progress {
    if (folders.isEmpty()) {
        return Progress("HomeCloud en marcha", "Todavía no compartes ninguna carpeta", null)
    }

    // Something a person has to fix comes before anything else.
    folders.firstOrNull { it.state is FolderState.Problem }?.let {
        return Progress("«${it.label}» necesita que mires", (it.state as FolderState.Problem).detail, null)
    }

    val syncing = folders.filter { it.state is FolderState.Syncing }
    if (syncing.isNotEmpty()) {
        val total = syncing.sumOf { it.bytes }
        val pending = syncing.sumOf { it.pendingBytes }
        val percent = if (total > 0) (100 - pending * 100 / total).coerceIn(0, 100).toInt() else 0
        val rate = syncing.sumOf { it.bytesPerSecond }
        val eta = if (rate > 0 && pending > 0) pending / rate else null
        val title = if (syncing.size == 1) {
            "Sincronizando «${syncing.first().label}» $percent%"
        } else {
            "Sincronizando ${syncing.size} carpetas · $percent%"
        }
        val detail = listOfNotNull(
            eta?.let { "faltan ${formatEta(it)}" },
            formatRate(rate).ifEmpty { null },
        ).joinToString(" · ").ifEmpty { formatBytes(pending) + " por bajar" }
        return Progress(title, detail, percent)
    }

    if (folders.any { it.pausedByNetwork }) {
        return Progress("En pausa hasta que haya wifi", "Se reanuda solo al conectarte", null)
    }
    if (folders.all { it.state == FolderState.Paused }) {
        return Progress("En pausa", "Nada se está sincronizando ahora mismo", null)
    }
    if (folders.all { it.state == FolderState.Disconnected }) {
        return Progress("Sin conexión con tus dispositivos", "Se pondrá al día en cuanto aparezcan", null)
    }
    return Progress(
        "Todo al día",
        "${folders.size} ${if (folders.size == 1) "carpeta" else "carpetas"} · " +
            formatBytes(folders.sumOf { it.bytes }),
        null,
    )
}

/** One file that was deleted and can still be brought back. */
data class DeletedFile(
    val id: String,
    val name: String,
    val originalPath: String,
    /** Seconds since the epoch. */
    val deletedAt: Long,
    val bytes: Long,
    val inSystemBin: Boolean,
) {
    companion object {
        fun from(json: JSONObject) = DeletedFile(
            id = json.getString("id"),
            name = json.getString("name"),
            originalPath = json.getString("originalPath"),
            deletedAt = json.optLong("deletedAt"),
            bytes = json.optLong("bytes"),
            inSystemBin = json.optBoolean("inSystemBin"),
        )
    }
}

/** "hace 3 días", "hace 2 h": when something was deleted, in words. */
fun timeAgo(seconds: Long, now: Long = System.currentTimeMillis() / 1000): String {
    val elapsed = maxOf(0L, now - seconds)
    return when {
        elapsed < 90 -> "hace un momento"
        elapsed < 3600 -> "hace ${(elapsed + 30) / 60} min"
        elapsed < 86400 -> "hace ${(elapsed + 1800) / 3600} h"
        elapsed < 172800 -> "ayer"
        else -> "hace ${(elapsed + 43200) / 86400} días"
    }
}

/** A folder being served as a public link, and when that stops on its own. */
data class LinkStatus(val url: String, val expiresAt: Long) {
    companion object {
        fun from(json: JSONObject) = LinkStatus(
            url = json.getString("url"),
            expiresAt = json.getLong("expiresAt"),
        )
    }
}

/** The head of a device ID: what tells two devices with the same name apart. */
fun shortId(id: String): String = id.substringBefore('-')

inline fun <T> JSONArray.map(transform: (JSONObject) -> T): List<T> =
    (0 until length()).map { transform(getJSONObject(it)) }

/** Sizes people read, not sizes computers like. */
fun formatBytes(bytes: Long): String {
    if (bytes < 1000) return "$bytes B"
    val units = listOf("kB", "MB", "GB", "TB")
    var value = bytes / 1000.0
    var unit = 0
    while (value >= 1000 && unit < units.lastIndex) {
        value /= 1000
        unit++
    }
    val text = if (value < 10) {
        String.format(java.util.Locale.ROOT, "%.1f", value)
    } else {
        String.format(java.util.Locale.ROOT, "%.0f", value)
    }
    return "${text.replace('.', ',')} ${units[unit]}"
}

fun peerSummary(peers: List<Peer>): String = when {
    peers.isEmpty() -> "Sin dispositivos todavía"
    peers.size == 1 -> peers.first().name
    else -> "${peers.size} dispositivos · ${peers.count { it.connected }} conectados"
}

fun stateLabel(state: FolderState): String = when (state) {
    FolderState.UpToDate -> "Al día"
    is FolderState.Syncing -> "Sincronizando ${state.percent}%"
    FolderState.Paused -> "En pausa"
    FolderState.Disconnected -> "Sin conexión"
    is FolderState.Problem -> state.detail
}
