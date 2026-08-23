package com.linux.vaultexplorer

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  /**
   * Keeps media playing when the app goes to the background.
   *
   * A WebView stops its media the moment the host activity pauses it, which
   * is why minimising the app cut a YouTube video dead. The activity still
   * needs the rest of `onPause` (Tauri's own lifecycle handling depends on
   * it), so this restores just the WebView afterwards: `onResume` on the
   * view resumes its media and timers without bringing the activity itself
   * back to the foreground.
   *
   * Deliberately not a foreground service: this keeps audio going while the
   * app sits in the background, which is what "minimise and keep listening"
   * means. A system that decides to reclaim the process will still stop it
   * -- a service with a media notification is the only way to outrank that,
   * and it is a much larger piece.
   */
  override fun onPause() {
    super.onPause()
    findWebView(window.decorView as? android.view.ViewGroup)?.onResume()
  }

  private fun findWebView(root: android.view.ViewGroup?): WebView? {
    if (root == null) return null
    for (i in 0 until root.childCount) {
      when (val child = root.getChildAt(i)) {
        is WebView -> return child
        is android.view.ViewGroup -> findWebView(child)?.let { return it }
      }
    }
    return null
  }
}
