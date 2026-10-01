package dev.lucasmoy.homecloud

import android.os.SystemClock

/**
 * When this process started, for the startup timings logged under
 * `HomeCloudStartup`. A slow launch is only fixable once it is known which
 * step is slow, and "it says Arrancando for ten seconds" names none of them.
 */
object Startup {
    val t0: Long = System.currentTimeMillis() -
        (SystemClock.elapsedRealtime() - android.os.Process.getStartElapsedRealtime())
}
