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

data class Peer(val id: String, val name: String, val connected: Boolean) {
    companion object {
        fun from(json: JSONObject) = Peer(
            id = json.getString("id"),
            name = json.getString("name"),
            connected = json.getBoolean("connected"),
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
    val keepVersions: Int,
    val engineVersion: String,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("deviceName", deviceName)
        put("deviceId", deviceId)
        put("localNetworkOnly", localNetworkOnly)
        put("uploadLimitKbps", uploadLimitKbps)
        put("downloadLimitKbps", downloadLimitKbps)
        put("keepVersions", keepVersions)
        put("engineVersion", engineVersion)
    }

    companion object {
        fun from(json: JSONObject) = Settings(
            deviceName = json.getString("deviceName"),
            deviceId = json.getString("deviceId"),
            localNetworkOnly = json.getBoolean("localNetworkOnly"),
            uploadLimitKbps = json.getInt("uploadLimitKbps"),
            downloadLimitKbps = json.getInt("downloadLimitKbps"),
            keepVersions = json.getInt("keepVersions"),
            engineVersion = json.optString("engineVersion"),
        )
    }
}

data class CodePreview(val deviceName: String, val folderLabel: String)

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
    val text = if (value < 10) String.format("%.1f", value) else String.format("%.0f", value)
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
