package dev.lucasmoy.homecloud

import org.json.JSONArray
import org.json.JSONObject

/**
 * Every question the interface asks, in the app's own words. Nothing above this
 * layer knows that the answers come from a Rust bridge, or that a Syncthing is
 * involved at all.
 */
object Repo {

    fun folders(): List<SharedFolder> =
        (Native.request("folders") as JSONArray).map { SharedFolder.from(it) }

    fun invitations(): List<Invitation> =
        (Native.request("invitations") as JSONArray).map { Invitation.from(it) }

    fun settings(): Settings = Settings.from(Native.request("settings") as JSONObject)

    fun saveSettings(settings: Settings) {
        Native.request("saveSettings", JSONObject().put("settings", settings.toJson()))
    }

    fun shareFolder(path: String, label: String): String =
        Native.request("shareFolder", JSONObject().put("path", path).put("label", label)) as String

    fun codeFor(folderId: String): String =
        Native.request("codeFor", JSONObject().put("folderId", folderId)) as String

    fun previewCode(code: String): CodePreview {
        val json = Native.request("previewCode", JSONObject().put("code", code)) as JSONObject
        return CodePreview(
            deviceName = json.getString("deviceName"),
            folderLabel = json.getString("folderLabel"),
            bytes = if (json.isNull("bytes")) null else json.optLong("bytes"),
        )
    }

    fun redeemCode(code: String, localPath: String) {
        Native.request("redeemCode", JSONObject().put("code", code).put("localPath", localPath))
    }

    fun accept(invitation: Invitation, localPath: String?) {
        Native.request(
            "accept",
            JSONObject().put("invitation", invitation.toJson()).put("localPath", localPath),
        )
    }

    fun decline(invitation: Invitation) {
        Native.request("decline", JSONObject().put("invitation", invitation.toJson()))
    }

    fun setFolderPaused(folderId: String, paused: Boolean) {
        Native.request(
            "setFolderPaused",
            JSONObject().put("folderId", folderId).put("paused", paused),
        )
    }

    fun stopSharing(folderId: String) {
        Native.request("stopSharing", JSONObject().put("folderId", folderId))
    }

    /**
     * Names this device if the engine gave it one that says nothing.
     *
     * Android reports its hostname as `localhost`, so without this every phone
     * introduces itself to every other device by the same meaningless word.
     */
    fun ensureDeviceName(fallback: String): String =
        Native.request("ensureDeviceName", JSONObject().put("fallback", fallback)) as String

    fun forgetUnusedDevices(): List<String> {
        val dropped = Native.request("forgetUnusedDevices") as JSONArray
        return (0 until dropped.length()).map { dropped.getString(it) }
    }

    fun setFolderReadOnly(folderId: String, readOnly: Boolean) {
        Native.request(
            "setFolderReadOnly",
            JSONObject().put("folderId", folderId).put("readOnly", readOnly),
        )
    }

    /** Tells the core where it may put folders it accepts on its own. */
    fun rescan(folderId: String) {
        Native.request("rescan", JSONObject().put("folderId", folderId))
    }

    fun setFolderWifiOnly(folderId: String, wifiOnly: Boolean) {
        Native.request(
            "setFolderWifiOnly",
            JSONObject().put("folderId", folderId).put("wifiOnly", wifiOnly),
        )
    }

    /** Pauses or resumes the wifi-only folders as the connection changes. */
    fun applyMeteredPolicy(metered: Boolean) {
        Native.request("applyMeteredPolicy", JSONObject().put("metered", metered))
    }

    /** Tears every connection down and dials again. */
    fun reconnectAll() {
        Native.request("reconnectAll")
    }

    fun setRoots(autoAcceptRoot: String, preferences: String) {
        Native.request(
            "setRoots",
            JSONObject().put("autoAcceptRoot", autoAcceptRoot).put("preferences", preferences),
        )
    }

    /** What choosing `chosen` for a folder called `label` would actually do. */
    fun resolveDestination(chosen: String, label: String, pick: String? = null): Destination {
        val args = JSONObject().put("chosen", chosen).put("label", label)
        if (pick != null) args.put("pick", pick)
        return Destination.from(Native.request("resolveDestination", args) as JSONObject)
    }
}
