package ai.openvoice.client

import android.app.*
import android.content.*
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.IBinder
import android.provider.Settings
import android.view.*
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
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
    private var overlay: View? = null
    private var dismissOverlay: View? = null
    private var dismissTextView: TextView? = null
    private var agentButton: Button? = null
    private var agents: List<NodeAgent> = emptyList()
    private var selectedAgentId: String? = null
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
        val saved = SecureNodeStore(this).load()
            ?: return update("error", "Open the app and pair with a node first")
        update("connecting", null)
        val node = NodeClient(saved.url, saved.credential, saved.trustSelfSigned)
        node.settings { result -> result.fold(
            { cfg ->
                applyAgents(cfg.agents)
                session = VoiceSession(this, node, cfg, { SecureNodeStore(this).selectedAgent()?.first ?: selectedAgentId }, ::update).also { it.start() }
            },
            {
                if (it is NodeAccessRevokedException) SecureNodeStore(this).clear()
                update("error", if (it is NodeAccessRevokedException) "Access revoked — open the app to pair again" else it.message)
            },
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

    private fun dp(n: Int) = (n * resources.displayMetrics.density).toInt()

    private fun getScreenHeight(wm: WindowManager): Int {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            wm.currentWindowMetrics.bounds.height()
        } else {
            resources.displayMetrics.heightPixels
        }
    }

    private fun createDismissBackground(active: Boolean): GradientDrawable {
        return GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dp(20).toFloat()
            if (active) {
                setColor(Color.argb(235, 220, 50, 50))
                setStroke(dp(1).coerceAtLeast(1), Color.argb(200, 255, 120, 120))
            } else {
                setColor(Color.argb(210, 30, 34, 45))
                setStroke(dp(1), Color.argb(100, 120, 140, 180))
            }
        }
    }

    private fun showDismissTarget(wm: WindowManager) {
        if (dismissOverlay != null || !Settings.canDrawOverlays(this)) return
        val pill = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER
            setPadding(dp(16), dp(10), dp(16), dp(10))
            background = createDismissBackground(false)

            val icon = TextView(this@VoiceService).apply {
                text = "✕"
                setTextColor(Color.WHITE)
                textSize = 14f
                setPadding(0, 0, dp(6), 0)
            }
            val text = TextView(this@VoiceService).apply {
                this@VoiceService.dismissTextView = this
                this.text = "Drag here to dismiss"
                setTextColor(Color.WHITE)
                textSize = 13f
            }
            addView(icon)
            addView(text)
        }

        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT
        ).apply {
            gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
            y = dp(36)
        }

        dismissOverlay = pill
        wm.addView(pill, params)
    }

    private fun updateDismissTarget(inZone: Boolean) {
        dismissOverlay?.let { view ->
            view.background = createDismissBackground(inZone)
            view.animate().scaleX(if (inZone) 1.1f else 1.0f).scaleY(if (inZone) 1.1f else 1.0f).setDuration(120).start()
        }
        dismissTextView?.text = if (inZone) "Release to dismiss" else "Drag here to dismiss"
    }

    private fun hideDismissTarget(wm: WindowManager) {
        dismissOverlay?.let {
            runCatching { wm.removeView(it) }
        }
        dismissOverlay = null
        dismissTextView = null
    }

    private fun showOverlay() {
        if (orb != null || !Settings.canDrawOverlays(this)) return
        val wm = getSystemService(WINDOW_SERVICE) as WindowManager
        val height = (76 * resources.displayMetrics.density).toInt()
        val width = (142 * resources.displayMetrics.density).toInt()
        val params = WindowManager.LayoutParams(width, height, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT).apply { gravity = Gravity.TOP or Gravity.START; x = resources.displayMetrics.widthPixels - width - 24; y = resources.displayMetrics.heightPixels / 3 }
        val root = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
        orb = OrbView(this).also { view ->
            view.state = state
            var downX = 0f; var downY = 0f; var startX = 0; var startY = 0
            var isDragging = false
            var isOverDismiss = false

            view.setOnTouchListener { _, e ->
                when (e.action) {
                    MotionEvent.ACTION_DOWN -> {
                        downX = e.rawX
                        downY = e.rawY
                        startX = params.x
                        startY = params.y
                        isDragging = false
                        isOverDismiss = false
                        true
                    }
                    MotionEvent.ACTION_MOVE -> {
                        val dx = e.rawX - downX
                        val dy = e.rawY - downY
                        if (!isDragging && (abs(dx) > 12 || abs(dy) > 12)) {
                            isDragging = true
                            showDismissTarget(wm)
                        }
                        if (isDragging) {
                            params.x = startX + dx.toInt()
                            params.y = startY + dy.toInt()
                            wm.updateViewLayout(root, params)

                            val screenHeight = getScreenHeight(wm)
                            val dismissThreshold = dp(120)
                            val inZone = e.rawY >= screenHeight - dismissThreshold || (params.y + height) >= screenHeight - dp(60)

                            if (inZone != isOverDismiss) {
                                isOverDismiss = inZone
                                updateDismissTarget(inZone)
                                root.alpha = if (inZone) 0.6f else 1.0f
                            }
                        }
                        true
                    }
                    MotionEvent.ACTION_UP -> {
                        val dx = abs(e.rawX - downX)
                        val dy = abs(e.rawY - downY)
                        hideDismissTarget(wm)
                        root.alpha = 1.0f

                        if (isDragging && isOverDismiss) {
                            session?.stop()
                            stopSelf()
                        } else if (dx < 12 && dy < 12) {
                            toggle()
                        }
                        isDragging = false
                        isOverDismiss = false
                        true
                    }
                    MotionEvent.ACTION_CANCEL -> {
                        hideDismissTarget(wm)
                        root.alpha = 1.0f
                        isDragging = false
                        isOverDismiss = false
                        true
                    }
                    else -> false
                }
            }
            root.addView(view, LinearLayout.LayoutParams(height, height))
        }
        agentButton = Button(this).apply {
            text = SecureNodeStore(this@VoiceService).selectedAgent()?.second ?: "Agent"
            textSize = 10f; isAllCaps = false; setPadding(2, 0, 2, 0)
            setOnClickListener { cycleAgent() }
            root.addView(this, LinearLayout.LayoutParams(width - height, height / 2))
        }
        overlay = root
        wm.addView(root, params)
        refreshAgents()
    }

    private fun refreshAgents() {
        val saved = SecureNodeStore(this).load() ?: return
        NodeClient(saved.url, saved.credential, saved.trustSelfSigned).settings { result ->
            result.onSuccess { applyAgents(it.agents) }
        }
    }

    @Synchronized private fun applyAgents(incoming: List<NodeAgent>) {
        agents = incoming
        if (agents.isEmpty()) { selectedAgentId = null; agentButton?.post { agentButton?.text = "Agent" }; return }
        val saved = SecureNodeStore(this).selectedAgent()?.first
        val selected = agents.find { it.id == saved } ?: agents.first()
        selectedAgentId = selected.id
        SecureNodeStore(this).selectAgent(selected.id, selected.alias)
        agentButton?.post { agentButton?.text = selected.alias }
    }

    @Synchronized private fun cycleAgent() {
        if (agents.size < 2) return
        val index = agents.indexOfFirst { it.id == selectedAgentId }.coerceAtLeast(0)
        val selected = agents[(index + 1) % agents.size]
        selectedAgentId = selected.id
        SecureNodeStore(this).selectAgent(selected.id, selected.alias)
        agentButton?.text = selected.alias
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
        session?.stop()
        val wm = getSystemService(WINDOW_SERVICE) as? WindowManager
        overlay?.let { runCatching { wm?.removeView(it) } }
        dismissOverlay?.let { runCatching { wm?.removeView(it) } }
        overlay = null
        orb = null
        dismissOverlay = null
        dismissTextView = null
        super.onDestroy()
    }
}
