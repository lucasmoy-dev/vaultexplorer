package dev.lucasmoy.homecloud

import org.json.JSONObject

/**
 * The bridge to homecore, the same Rust crate the desktop app links against.
 *
 * Deliberately one call: a pairing code read on this phone and one written on a
 * laptop go through the identical code, so they cannot drift apart the way two
 * hand-written implementations of the same format would.
 */
object Native {
    init {
        System.loadLibrary("homecloud")
    }

    external fun connect(baseUrl: String, apiKey: String)

    private external fun call(method: String, argsJson: String): String

    /**
     * Returns the `ok` value, or throws with the sentence the core produced —
     * which is already phrased for a person to read.
     */
    fun request(method: String, args: JSONObject = JSONObject()): Any? {
        val reply = JSONObject(call(method, args.toString()))
        if (reply.has("error")) throw EngineException(reply.getString("error"))
        return if (reply.isNull("ok")) null else reply.get("ok")
    }
}

class EngineException(message: String) : Exception(message)
