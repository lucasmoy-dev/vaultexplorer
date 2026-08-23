package dev.lucasmoy.livesubs

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The subtitles themselves: a window that floats over every other app.
 *
 * Three flags do the work:
 *
 * - `TYPE_APPLICATION_OVERLAY` -- the only window type a normal app may use
 *   above others, and the reason the app needs "Display over other apps"
 *   granted by hand in Settings.
 * - `FLAG_NOT_TOUCHABLE` -- every touch passes through to whatever is
 *   underneath. Without it a strip across the screen would swallow taps on
 *   the app you are actually using, which is far worse than having no
 *   subtitles.
 * - `FLAG_NOT_FOCUSABLE` -- it never takes the keyboard, so typing in the
 *   app below is unaffected.
 *
 * Plain Views rather than Compose: this has to be attached to a
 * `WindowManager` from a Service with no lifecycle owner, where Compose
 * needs scaffolding that buys nothing for three TextViews.
 */
class OverlayController(private val context: Context) {
    private val windowManager = context.getSystemService(WindowManager::class.java)
    private val handler = Handler(Looper.getMainLooper())
    private val container = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        // Newest line at the bottom, growing upwards -- how every subtitle
        // track works, and it keeps the eye where the next line appears.
        gravity = Gravity.CENTER_HORIZONTAL or Gravity.BOTTOM
    }
    private val lines = ArrayDeque<View>()
    private var attached = false
    private var settings = Settings()

    fun attach(settings: Settings) {
        this.settings = settings
        if (attached) {
            apply(settings)
            return
        }
        runCatching {
            windowManager.addView(container, layoutParams(settings))
            attached = true
        }
    }

    fun detach() {
        if (!attached) return
        runCatching { windowManager.removeView(container) }
        attached = false
        lines.clear()
        container.removeAllViews()
    }

    /** Re-apply size, position and colours -- called on every settings change. */
    fun apply(settings: Settings) {
        this.settings = settings
        if (!attached) return
        runCatching { windowManager.updateViewLayout(container, layoutParams(settings)) }
        // Existing lines keep their own colours (they belong to a source),
        // but font size and the plate follow the new settings immediately.
        for (view in lines) {
            (view as? TextView)?.let { style(it, it.currentTextColor) }
        }
        while (lines.size > settings.maxLines.coerceAtLeast(1)) removeOldest()
    }

    /**
     * Show one caption. `micSource` picks the colour: the whole point of
     * keeping the two audio sources apart is being able to tell your own
     * voice from everyone else's at a glance.
     */
    fun show(text: String, original: String?, micSource: Boolean) {
        handler.post {
            if (!attached) return@post
            val color = if (micSource) settings.micColor else settings.systemColor
            val view = TextView(context).apply {
                this.text = if (original.isNullOrBlank()) text else "$text\n$original"
                style(this, color)
            }
            container.addView(view)
            lines.addLast(view)
            while (lines.size > settings.maxLines.coerceAtLeast(1)) removeOldest()
            handler.postDelayed({ fadeOut(view) }, settings.hideAfterMs.coerceAtLeast(1000).toLong())
        }
    }

    private fun removeOldest() {
        val oldest = lines.removeFirstOrNull() ?: return
        fadeOut(oldest)
    }

    private fun fadeOut(view: View) {
        // Fading rather than vanishing: a line that disappears between two
        // frames reads as a glitch.
        view.animate().alpha(0f).setDuration(320).withEndAction {
            container.removeView(view)
            lines.remove(view)
        }.start()
    }

    private fun style(view: TextView, color: Int) {
        view.setTextColor(color)
        view.setTextSize(TypedValue.COMPLEX_UNIT_SP, settings.fontSize.toFloat())
        view.gravity = Gravity.CENTER
        val padding = dp(10)
        view.setPadding(dp(14), padding, dp(14), padding)
        view.background = GradientDrawable().apply {
            cornerRadius = dp(14).toFloat()
            setColor(settings.plateColor)
        }
        // A dark plate is not always enough: a bright video frame behind a
        // semi-transparent plate washes the text out. The shadow keeps it
        // legible at any opacity, including zero.
        view.setShadowLayer(6f, 0f, 2f, Color.argb(220, 0, 0, 0))
        val params = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.WRAP_CONTENT,
            LinearLayout.LayoutParams.WRAP_CONTENT,
        ).apply { topMargin = dp(4) }
        view.layoutParams = params
    }

    private fun layoutParams(settings: Settings): WindowManager.LayoutParams {
        val metrics = context.resources.displayMetrics
        val width = (metrics.widthPixels * settings.widthPercent.coerceIn(30, 100) / 100)
        return WindowManager.LayoutParams(
            width,
            WindowManager.LayoutParams.WRAP_CONTENT,
            // The only window type a normal app may put above others. No
            // version check: this app's minimum is Android 10, well past
            // the API 26 where TYPE_PHONE was replaced.
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            android.graphics.PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = when (settings.anchor) {
                "top" -> Gravity.TOP or Gravity.CENTER_HORIZONTAL
                "center" -> Gravity.CENTER
                else -> Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
            }
            // `y` is a distance from the anchored edge for top/bottom, and
            // an offset from the middle for center -- where 0 is what you
            // want.
            y = if (settings.anchor == "center") 0 else dp(settings.margin)
        }
    }

    private fun dp(value: Int): Int =
        (value * context.resources.displayMetrics.density).toInt()
}
