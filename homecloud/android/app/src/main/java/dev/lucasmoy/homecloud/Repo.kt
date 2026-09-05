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
}
