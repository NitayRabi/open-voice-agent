package ai.openvoice.client

import android.app.*
import android.content.*
import android.graphics.PixelFormat
import android.os.Build
import android.os.IBinder
import android.provider.Settings
import android.view.*
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import kotlin.math.abs

class VoiceService : Service() {
    companion object {
        const val ACTION_SHOW = "ai.openvoice.SHOW"
        const val ACTION_TOGGLE = "ai.openvoice.TOGGLE"
        const val ACTION_STOP = "ai.openvoice.STOP"
        const val ACTION_STATE = "ai.openvoice.STATE"
        const val EXTRA_STATE = "state"
        const val EXTRA_MESSAGE = "message"
        private const val CHANNEL = "voice"
        private const val NOTIFICATION = 1730
    }

    private var orb: OrbView? = null
    private var session: VoiceSession? = null
    private var state = "idle"

    override fun onCreate() {
        super.onCreate(); createChannel(); startForeground(NOTIFICATION, notification())
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_TOGGLE -> toggle()
            ACTION_STOP -> { session?.stop(); stopSelf() }
            else -> showOverlay()
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun toggle() {
        if (session != null) { session?.stop(); session = null; return }
        val prefs = getSharedPreferences("node", MODE_PRIVATE)
        val url = prefs.getString("url", "").orEmpty(); val code = prefs.getString("code", "").orEmpty()
        if (url.isBlank()) return update("error", "Open the app and pair with a node first")
        update("connecting", null)
        val node = NodeClient(url, code, prefs.getBoolean("trust_self_signed", false))
        node.settings { result -> result.fold(
            { cfg -> session = VoiceSession(this, node, cfg, ::update).also { it.start() } },
            { update("error", it.message) },
        ) }
    }

    private fun update(newState: String, message: String?) {
        if (newState == "level") {
            orb?.post { orb?.level = message?.toFloatOrNull()?.times(5f) ?: 0f }
            return
        }
        state = newState
        orb?.post {
            orb?.state = newState
        }
        sendBroadcast(Intent(ACTION_STATE).setPackage(packageName).putExtra(EXTRA_STATE, newState).putExtra(EXTRA_MESSAGE, message))
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).notify(NOTIFICATION, notification())
        if (newState == "idle") session = null
    }

    private fun showOverlay() {
        if (orb != null || !Settings.canDrawOverlays(this)) return
        val wm = getSystemService(WINDOW_SERVICE) as WindowManager
        val size = (76 * resources.displayMetrics.density).toInt()
        val params = WindowManager.LayoutParams(size, size, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT).apply { gravity = Gravity.TOP or Gravity.START; x = resources.displayMetrics.widthPixels - size - 24; y = resources.displayMetrics.heightPixels / 3 }
        orb = OrbView(this).also { view ->
            view.state = state
            var downX = 0f; var downY = 0f; var startX = 0; var startY = 0
            view.setOnTouchListener { _, e ->
                when (e.action) {
                    MotionEvent.ACTION_DOWN -> { downX = e.rawX; downY = e.rawY; startX = params.x; startY = params.y; true }
                    MotionEvent.ACTION_MOVE -> { params.x = startX + (e.rawX - downX).toInt(); params.y = startY + (e.rawY - downY).toInt(); wm.updateViewLayout(view, params); true }
                    MotionEvent.ACTION_UP -> { if (abs(e.rawX - downX) < 12 && abs(e.rawY - downY) < 12) toggle(); true }
                    else -> false
                }
            }
            wm.addView(view, params)
        }
    }

    private fun notification(): Notification {
        val toggle = PendingIntent.getService(this, 1, Intent(this, VoiceService::class.java).setAction(ACTION_TOGGLE), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop = PendingIntent.getService(this, 2, Intent(this, VoiceService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return NotificationCompat.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Open Voice Agent").setContentText(if (state == "idle") "Tap the orb to talk" else state.replace('_', ' '))
            .setOngoing(true).setOnlyAlertOnce(true).addAction(0, if (state == "idle" || state == "error") "Talk" else "Stop", toggle).addAction(0, "Close orb", stop).build()
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= 26) (getSystemService(NOTIFICATION_SERVICE) as NotificationManager)
            .createNotificationChannel(NotificationChannel(CHANNEL, "Voice orb", NotificationManager.IMPORTANCE_LOW))
    }

    override fun onDestroy() {
        session?.stop(); orb?.let { runCatching { (getSystemService(WINDOW_SERVICE) as WindowManager).removeView(it) } }; orb = null
        super.onDestroy()
    }
}
