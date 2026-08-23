package dev.lucasmoy.ytpocket

import android.app.Application

/**
 * Exists for one line: the native side has to be told where it may write
 * before anything asks YouTube a question, and both entry points (the
 * activity and the download service) need that to have happened.
 *
 * Doing it here rather than in each of them means it cannot be forgotten by
 * whichever one the system starts first -- a service resumed after the
 * process was killed does not go through the activity.
 */
class App : Application() {
    override fun onCreate() {
        super.onCreate()
        Native.prepare(this)
    }
}
