package ai.openvoice.wear

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.*
import android.view.View
import android.view.animation.LinearInterpolator
import kotlin.math.min
import kotlin.math.sin

// Animated voice-state orb with brand icon and layered glow rings,
// mirrored from the phone client. Low CPU and battery usage for Wear OS.
class OrbView(context: Context) : View(context) {
    var state: String = "idle"
        set(value) {
            field = value
            invalidate()
        }
    var level: Float = 0f
        set(value) {
            field = value.coerceIn(0f, 1f)
            invalidate()
        }

    private var phase = 0f
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)
    private val iconBitmap: Bitmap? by lazy {
        BitmapFactory.decodeResource(resources, R.mipmap.ic_launcher)
    }
    private val iconRect = RectF()

    init {
        ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 4000
            repeatCount = ValueAnimator.INFINITE
            interpolator = LinearInterpolator()
            addUpdateListener {
                phase = it.animatedValue as Float
                invalidate()
            }
            start()
        }
        contentDescription = "Voice orb"
        isClickable = true
    }

    override fun onDraw(c: Canvas) {
        val s = min(width, height).toFloat()
        val cx = width / 2f
        val cy = height / 2f

        val (r, g, b) = when (state) {
            "listening", "user_speaking" -> Triple(70, 215, 185) // Mint / teal
            "speaking" -> Triple(155, 115, 255) // Rich purple
            "thinking", "delegating" -> Triple(195, 130, 255) // Violet aura
            "connecting" -> Triple(255, 185, 95) // Amber / gold
            "error" -> Triple(245, 95, 95) // Coral warning
            else -> Triple(180, 160, 210) // Soft idle glow
        }

        val baseGlow = if (state == "idle") 0.22f else 0.45f
        val pulse = if (state == "idle") {
            sin(phase * 6.283f) * 2f
        } else {
            (sin(phase * 6.283f) * 3f) + (level * 18f)
        }

        val glowRadius = (s * 0.46f) + pulse
        if (glowRadius > 0) {
            // Layer 1: Ambient outer soft glow
            val glowAlpha = ((baseGlow + level * 0.4f).coerceIn(0f, 1f) * 255).toInt()
            val glowColor = Color.argb(glowAlpha, r, g, b)
            val transparentGlow = Color.argb(0, r, g, b)
            paint.shader = RadialGradient(
                cx, cy, glowRadius,
                intArrayOf(glowColor, Color.argb(glowAlpha / 2, r, g, b), transparentGlow),
                floatArrayOf(0f, 0.65f, 1f),
                Shader.TileMode.CLAMP
            )
            c.drawCircle(cx, cy, glowRadius, paint)
            paint.shader = null

            // Layer 2: Active state pulsating thin ring
            if (state != "idle") {
                val ringPhase = (phase * 1.5f) % 1f
                val ringRadius = s * (0.36f + ringPhase * 0.12f) + level * 8f
                val ringAlpha = ((1f - ringPhase) * (0.4f + level * 0.5f)).coerceIn(0f, 1f)
                paint.color = Color.argb((ringAlpha * 255).toInt(), r, g, b)
                paint.style = Paint.Style.STROKE
                paint.strokeWidth = 2f
                c.drawCircle(cx, cy, ringRadius, paint)
                paint.style = Paint.Style.FILL
            }
        }

        // Layer 3: Centered Brand Icon
        iconBitmap?.let { bmp ->
            val iconScale = if (state == "speaking" || state == "user_speaking") {
                0.66f + (level * 0.06f)
            } else if (state == "connecting" || state == "thinking" || state == "delegating") {
                0.66f + (sin(phase * 6.283f) * 0.02f)
            } else {
                0.66f
            }
            val iconSize = s * iconScale
            val half = iconSize / 2f
            iconRect.set(cx - half, cy - half, cx + half, cy + half)
            paint.color = Color.WHITE
            c.drawBitmap(bmp, null, iconRect, paint)
        }
    }
}
