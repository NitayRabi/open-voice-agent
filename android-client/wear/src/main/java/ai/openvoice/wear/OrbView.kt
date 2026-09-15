package ai.openvoice.wear

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.*
import android.view.View
import android.view.animation.LinearInterpolator
import kotlin.math.min

// Animated voice-state orb, mirrored from the phone client. Draws a radial
// gradient whose colour follows the voice state and whose pulse grows with
// the live audio level, so the watch reflects what the agent is doing.
class OrbView(context: Context) : View(context) {
    var state: String = "idle"
        set(value) { field = value; invalidate() }
    var level: Float = 0f
        set(value) { field = value.coerceIn(0f, 1f); invalidate() }
    private var phase = 0f
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

    init {
        ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 5000; repeatCount = ValueAnimator.INFINITE; interpolator = LinearInterpolator()
            addUpdateListener { phase = it.animatedValue as Float; invalidate() }; start()
        }
        contentDescription = "Voice orb"
        isClickable = true
    }

    override fun onDraw(c: Canvas) {
        val s = min(width, height).toFloat()
        val cx = width / 2f; val cy = height / 2f
        val color = when (state) {
            "listening", "user_speaking" -> Color.rgb(86, 205, 190)
            "speaking" -> Color.rgb(147, 118, 255)
            "thinking", "connecting", "delegating" -> Color.rgb(255, 177, 92)
            "error" -> Color.rgb(255, 103, 119)
            else -> Color.rgb(112, 137, 255)
        }
        val pulse = if (state == "idle") 0f else (6f + level * 14f)
        paint.shader = RadialGradient(cx, cy, s * .46f + pulse, intArrayOf(Color.WHITE, color, Color.argb(0, Color.red(color), Color.green(color), Color.blue(color))), floatArrayOf(0f, .48f, 1f), Shader.TileMode.CLAMP)
        c.drawCircle(cx, cy, s * .46f + pulse, paint)
        paint.shader = null
        paint.color = Color.argb(90, 255, 255, 255); paint.style = Paint.Style.STROKE; paint.strokeWidth = 2f
        c.drawCircle(cx, cy, s * (.34f + .025f * kotlin.math.sin(phase * 6.28f)), paint)
        paint.style = Paint.Style.FILL
    }
}
