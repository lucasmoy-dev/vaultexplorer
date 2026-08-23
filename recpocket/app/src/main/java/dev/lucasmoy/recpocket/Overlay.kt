package dev.lucasmoy.recpocket

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.provider.Settings as AndroidSettings
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/**
 * The floating controls: record, stop, screenshot -- on top of whatever app
 * is in front, so a call can be recorded without going back to this one.
 *
 * **It does not appear in the recording.** That is the `FLAG_SECURE` on the
 * window: a secure window is never composited onto a non-secure display, and
 * a `MediaProjection`'s virtual display is exactly that. So the buttons are
 * on the phone's screen and absent from both the video and the screenshots,
 * which is the behaviour asked for and the reason this is not simply hidden
 * and re-shown around each capture (that leaves a visible flicker, and races
 * with the frame you are trying to take).
 *
 * Drawn with plain views rather than Compose: an overlay window has no
 * lifecycle owner of its own, and giving one to Compose is more machinery
 * than three buttons deserve.
 */
object Overlay {
    private var view: View? = null
    private var watcher: Job? = null

    fun canShow(context: Context): Boolean =
        AndroidSettings.canDrawOverlays(context)

    @SuppressLint("ClickableViewAccessibility")
    fun show(context: Context) {
        if (view != null || !canShow(context)) return
        val windows = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager

        val record = pill(context, "●", Color.parseColor("#e5484d"))
        val shot = pill(context, "⛶", Color.parseColor("#3b82f6"))
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(10, 10, 10, 10)
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = 999f
                setColor(Color.parseColor("#cc111111"))
            }
            addView(record)
            addView(shot)
        }

        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            // FLAG_SECURE is the one that matters (see above).
            // NOT_FOCUSABLE keeps the keyboard and the app underneath
            // behaving normally -- an overlay that takes focus breaks
            // typing in whatever is behind it.
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_SECURE,
            android.graphics.PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = 24
            y = 220
        }

        record.setOnClickListener {
            if (CaptureService.recording.value) {
                CaptureService.send(context, CaptureService.ACTION_STOP)
            } else {
                CaptureService.send(context, CaptureService.ACTION_START)
            }
        }
        shot.setOnClickListener { CaptureService.send(context, CaptureService.ACTION_SHOT) }

        // Draggable, because a fixed button always ends up over the one
        // control the user needs.
        var downX = 0f
        var downY = 0f
        var startX = 0
        var startY = 0
        row.setOnTouchListener { _, event ->
            when (event.action) {
                MotionEvent.ACTION_DOWN -> {
                    downX = event.rawX
                    downY = event.rawY
                    startX = params.x
                    startY = params.y
                    false
                }
                MotionEvent.ACTION_MOVE -> {
                    val movedX = (event.rawX - downX).toInt()
                    val movedY = (event.rawY - downY).toInt()
                    if (kotlin.math.abs(movedX) > 8 || kotlin.math.abs(movedY) > 8) {
                        params.x = startX + movedX
                        params.y = startY + movedY
                        runCatching { windows.updateViewLayout(row, params) }
                        true
                    } else {
                        false
                    }
                }
                else -> false
            }
        }

        runCatching { windows.addView(row, params) }.onFailure { return }
        view = row

        // The dot turns into a square while recording, so the state is
        // readable at a glance from another app.
        watcher = CoroutineScope(Dispatchers.Main).launch {
            combine(CaptureService.recording, CaptureService.armed) { rec, _ -> rec }
                .collect { isRecording ->
                    record.text = if (isRecording) "■" else "●"
                }
        }
    }

    fun hide(context: Context) {
        val current = view ?: return
        val windows = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        runCatching { windows.removeView(current) }
        view = null
        watcher?.cancel()
        watcher = null
    }

    private fun pill(context: Context, glyph: String, tint: Int): TextView =
        TextView(context).apply {
            text = glyph
            setTextColor(tint)
            textSize = 20f
            gravity = Gravity.CENTER
            val size = (46 * context.resources.displayMetrics.density).toInt()
            layoutParams = LinearLayout.LayoutParams(size, size)
        }
}
