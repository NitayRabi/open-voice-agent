package ai.openvoice.client

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.*
import android.view.View
import android.view.animation.LinearInterpolator
import kotlin.math.min
import kotlin.math.sin

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
            duration = 3000
            repeatCount = ValueAnimator.INFINITE
            interpolator = LinearInterpolator()
            addUpdateListener {
                phase = it.animatedValue as Float
                invalidate()
            }
            start()
        }
        contentDescription = "Toggle voice"
        isClickable = true
    }

    override fun onDraw(c: Canvas) {
        val s = min(width, height).toFloat()
        val cx = width / 2f
        val cy = height / 2f

        val (r, g, b) = when (state) {
            "listening", "user_speaking" -> Triple(50, 230, 190)  // Vivid mint / teal
            "speaking" -> Triple(165, 120, 255)                   // Radiant purple
            "thinking", "delegating" -> Triple(210, 130, 255)     // Shimmering violet aurora
            "connecting" -> Triple(255, 190, 80)                  // Bright amber gold
            "error" -> Triple(255, 80, 80)                        // Vivid coral warning
            else -> Triple(180, 160, 220)                         // Soft idle glow
        }

        val isIdle = (state == "idle")
        val breath = sin(phase * 6.283f)

        // Layer 1: Core Radial Aura Glow
        val baseGlowAlpha = if (isIdle) 0.28f else 0.65f
        val glowAlpha = ((baseGlowAlpha + level * 0.35f).coerceIn(0f, 1f) * 255).toInt()
        val glowRadius = if (isIdle) {
            (s * 0.44f) + (breath * 3f)
        } else {
            (s * 0.46f) + (breath * 5f) + (level * 22f)
        }

        if (glowRadius > 0) {
            val coreColor = Color.argb(glowAlpha, r, g, b)
            val midColor = Color.argb(glowAlpha / 2, r, g, b)
            val transparentColor = Color.argb(0, r, g, b)

            paint.shader = RadialGradient(
                cx, cy, glowRadius,
                intArrayOf(coreColor, midColor, transparentColor),
                floatArrayOf(0f, 0.55f, 1f),
                Shader.TileMode.CLAMP
            )
            c.drawCircle(cx, cy, glowRadius, paint)
            paint.shader = null
        }

        // Layer 2: Animated Concentric Ripple Waves (Active States)
        if (!isIdle) {
            paint.style = Paint.Style.STROKE

            // Primary expanding ripple wave
            val wave1 = (phase * 1.6f) % 1f
            val radius1 = (s * 0.30f) + (wave1 * s * 0.19f) + (level * 10f)
            val alpha1 = ((1f - wave1) * (0.6f + level * 0.4f)).coerceIn(0f, 1f)
            paint.color = Color.argb((alpha1 * 255).toInt(), r, g, b)
            paint.strokeWidth = 3f + (level * 3f)
            c.drawCircle(cx, cy, radius1, paint)

            // Secondary trailing wave for dense continuous pulse
            val wave2 = ((phase * 1.6f) + 0.5f) % 1f
            val radius2 = (s * 0.30f) + (wave2 * s * 0.19f) + (level * 8f)
            val alpha2 = ((1f - wave2) * (0.45f + level * 0.3f)).coerceIn(0f, 1f)
            paint.color = Color.argb((alpha2 * 255).toInt(), r, g, b)
            paint.strokeWidth = 2f + (level * 2f)
            c.drawCircle(cx, cy, radius2, paint)

            paint.style = Paint.Style.FILL
        }

        // Layer 3: Centered Brand Icon
        iconBitmap?.let { bmp ->
            val iconScale = when {
                state == "speaking" || state == "user_speaking" -> 0.54f + (level * 0.08f)
                state == "connecting" -> 0.54f + (breath * 0.03f)
                state == "thinking" || state == "delegating" -> 0.54f + (breath * 0.02f)
                state == "listening" -> 0.54f + (level * 0.05f)
                else -> 0.52f + (breath * 0.015f)
            }
            val iconSize = s * iconScale
            val half = iconSize / 2f
            iconRect.set(cx - half, cy - half, cx + half, cy + half)
            paint.color = Color.WHITE
            c.drawBitmap(bmp, null, iconRect, paint)
        }
    }
}
