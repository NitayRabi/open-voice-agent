package ai.openvoice.wear

import android.Manifest
import android.annotation.SuppressLint
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import kotlin.math.abs

class MainActivity : ComponentActivity() {
    private lateinit var store: SecureNodeStore
    private lateinit var container: LinearLayout
    private var client: NodeClient? = null
    private var voiceSession: VoiceSession? = null
    private var orb: OrbView? = null
    private var starting = false
    private var active = false
    private var showingTasks = false
    private var voiceStatus = "Tap to talk"
    private var agents: List<NodeAgent> = emptyList()
    private var selectedAgentId: String? = null
    private var voiceState = "idle"
    private val delegations = LinkedHashMap<String, DelegationStatus>()

    private val microphonePermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) beginTalk() else showTalkScreen("Microphone permission denied")
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        store = SecureNodeStore(this)
        container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            val side = dp(if (resources.configuration.isScreenRound) 24 else 16)
            setPadding(side, dp(14), side, dp(18))
        }
        setContentView(ScrollView(this).apply {
            isFillViewport = true
            setBackgroundColor(BACKGROUND)
            addView(container, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        })
        val saved = store.load()
        if (saved == null) showPairingScreen() else {
            showTalkScreen()
            refreshAgents(saved)
        }
    }

    // Direct entry for standalone pairing
    @SuppressLint("WearPasswordInput")
    private fun showPairingScreen(
        message: String? = null,
        initialUrl: String = "",
        initialTrust: Boolean = false,
    ) {
        stopVoice(false)
        showingTasks = false
        container.removeAllViews()
        title("Pair watch")
        container.addView(caption(message ?: "Connect directly to your Open Voice node"), match(dp(5)))
        val url = field("Node URL", initialUrl, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        val code = field(
            "Pairing code",
            "",
            InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD,
        )
        val trust = CheckBox(this).apply {
            setText(R.string.trust_self_signed)
            setTextColor(MUTED)
            textSize = 12f
            isChecked = initialTrust
        }
        container.addView(url, match(dp(12)))
        container.addView(code, match(dp(8)))
        container.addView(trust, match(dp(4)))
        container.addView(actionButton("Pair") {
            val cleanUrl = url.text.toString().trim().trimEnd('/')
            val cleanCode = code.text.toString().trim()
            if (cleanUrl.toHttpUrlOrNull() == null || cleanCode.isBlank()) {
                showPairingScreen("Enter a node URL and pairing code")
                return@actionButton
            }
            exchangePairingCode(cleanUrl, cleanCode, trust.isChecked)
        }, match(dp(10)))
    }

    private fun exchangePairingCode(url: String, code: String, trustSelfSigned: Boolean) {
        container.removeAllViews()
        title("Pairing…")
        container.addView(caption("Creating a device credential"), match(dp(5)))
        val checkingClient = NodeClient(url, "", trustSelfSigned)
        client = checkingClient
        checkingClient.pair(code) { result ->
            runOnUiThread {
                if (client !== checkingClient || isFinishing || isDestroyed) return@runOnUiThread
                result.fold(
                    onSuccess = { credential ->
                        store.save(SavedNode(url, credential.accessToken, trustSelfSigned))
                        checkingClient.close()
                        client = null
                        showTalkScreen("Paired")
                    },
                    onFailure = {
                        checkingClient.close()
                        client = null
                        showPairingScreen(it.message ?: "Could not reach node", url, trustSelfSigned)
                    },
                )
            }
        }
    }

    private fun showTalkScreen(message: String? = null) {
        showingTasks = false
        if (message != null) voiceStatus = message
        container.removeAllViews()

        val orb = orbView().apply {
            this.state = voiceState
            var swipeStart = 0f
            setOnTouchListener { view, event ->
                when (event.action) {
                    MotionEvent.ACTION_DOWN -> { swipeStart = event.rawX; true }
                    MotionEvent.ACTION_UP -> {
                        val distance = event.rawX - swipeStart
                        if (abs(distance) > dp(28)) cycleAgent(if (distance < 0) 1 else -1)
                        else view.performClick()
                        true
                    }
                    else -> true
                }
            }
        }

        // 1. Prominent Voice Orb with State Glow & Icon
        container.addView(orb, LinearLayout.LayoutParams(dp(120), dp(120)).apply {
            gravity = Gravity.CENTER_HORIZONTAL
            topMargin = dp(4)
        })

        // 2. Agent Name
        container.addView(TextView(this).apply {
            text = currentAgentAlias()
            textSize = 14f
            typeface = Typeface.DEFAULT_BOLD
            setTextColor(Color.WHITE)
            gravity = Gravity.CENTER
        }, match(dp(4)))

        // Swipe hint if multi-agent
        if (agents.size > 1) {
            container.addView(caption("Swipe left/right to change").apply { textSize = 9f }, match(dp(1)))
        }

        // 3. Status text ("Tap to talk", "Connecting…", "Listening…", etc.)
        val statusColor = when (voiceState) {
            "listening", "user_speaking" -> Color.rgb(70, 235, 195)
            "speaking" -> Color.rgb(180, 140, 255)
            "connecting" -> Color.rgb(255, 200, 100)
            "error" -> STOP
            else -> MUTED
        }
        val status = caption(message ?: voiceStatus).apply {
            setTextColor(statusColor)
            textSize = 12f
        }
        container.addView(status, match(dp(4)))

        // Delegations / Tasks indicator
        if (delegations.isNotEmpty()) {
            val running = delegations.values.count { it.state == DelegationStatus.State.RUNNING }
            val label = if (running > 0) "● $running task${if (running == 1) "" else "s"}" else "✓ Tasks"
            container.addView(actionButton(label) { showDelegations() }.apply {
                textSize = 11f
                setTextColor(if (running > 0) TASK_ACTIVE else MUTED)
                minHeight = 0
                minimumHeight = 0
                setPadding(dp(10), dp(3), dp(10), dp(3))
                background = roundedBackground(Color.rgb(27, 34, 50), dp(18).toFloat())
            }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                gravity = Gravity.CENTER_HORIZONTAL
                topMargin = dp(6)
            })
        }

        // Forget & Re-pair link at bottom
        container.addView(actionButton("Forget & re-pair") { showForgetConfirmation() }.apply {
            textSize = 11f
            setTextColor(MUTED)
            background = null
        }, match(dp(6)))
    }

    private fun orbView(): OrbView {
        orb ?: OrbView(this).apply {
            setOnClickListener { if (active || starting) stopVoice() else requestTalk() }
        }.also { orb = it }
        return orb!!
    }

    private fun showDelegations() {
        showingTasks = true
        container.removeAllViews()
        title("Agent tasks")
        if (delegations.isEmpty()) {
            container.addView(caption("No delegated tasks yet"), match(dp(10)))
        } else {
            delegations.values.toList().asReversed().forEach { task ->
                val symbol = when (task.state) {
                    DelegationStatus.State.RUNNING -> "●"
                    DelegationStatus.State.SUCCEEDED -> "✓"
                    DelegationStatus.State.FAILED -> "!"
                }
                container.addView(TextView(this).apply {
                    text = "$symbol ${task.request.take(72)}"
                    textSize = 12f
                    setTextColor(if (task.state == DelegationStatus.State.FAILED) STOP else Color.WHITE)
                    setPadding(dp(9), dp(7), dp(9), dp(7))
                    background = roundedBackground(Color.rgb(27, 34, 50), dp(10).toFloat())
                }, match(dp(7)))
                task.detail?.takeIf { it.isNotBlank() }?.let { detail ->
                    container.addView(caption(detail.take(120)).apply { textSize = 10f }, match(dp(2)))
                }
            }
        }
        container.addView(actionButton("Back") { showTalkScreen() }.apply { textSize = 12f }, match(dp(9)))
    }

    private fun showForgetConfirmation() {
        val saved = store.load() ?: return showPairingScreen()
        stopVoice(false)
        container.removeAllViews()
        title("Forget this watch?")
        container.addView(caption("The saved device token will be erased."), match(dp(5)))
        container.addView(actionButton("Cancel") { showTalkScreen() }, match(dp(14)))
        container.addView(actionButton("Forget & re-pair") {
            store.clear()
            showPairingScreen(initialUrl = saved.url, initialTrust = saved.trustSelfSigned)
        }, match(dp(6)))
    }

    private fun requestTalk() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            beginTalk()
        } else microphonePermission.launch(Manifest.permission.RECORD_AUDIO)
    }

    private fun beginTalk() {
        val node = store.load() ?: return showPairingScreen()
        if (starting || active) return
        starting = true
        voiceState = "connecting"
        orb?.state = "connecting"
        showTalkScreen("Connecting…")

        val newClient = NodeClient(node.url, node.accessToken, node.trustSelfSigned)
        client = newClient
        newClient.settings { result ->
            runOnUiThread {
                if (client !== newClient || isFinishing || isDestroyed) return@runOnUiThread
                result.fold(
                    onSuccess = { settings ->
                        applyAgents(settings.agents)
                        val session = VoiceSession(this, newClient, settings, { selectedAgentId }, ::onVoiceEvent, ::onDelegationEvent, ::onAudioLevel)
                        voiceSession = session
                        session.start()
                    },
                    onFailure = {
                        starting = false
                        voiceState = "error"
                        orb?.state = "error"
                        newClient.close()
                        client = null
                        if (it is NodeAccessRevokedException) {
                            store.clear()
                            showPairingScreen("Watch access was revoked — pair again", node.url, node.trustSelfSigned)
                        } else {
                            showTalkScreen(it.message ?: "Could not connect")
                        }
                    },
                )
            }
        }
    }

    private fun refreshAgents(node: SavedNode) {
        val probe = NodeClient(node.url, node.accessToken, node.trustSelfSigned)
        probe.settings { result ->
            runOnUiThread {
                result.onSuccess { applyAgents(it.agents); if (!active && !starting) showTalkScreen() }
                probe.close()
            }
        }
    }

    private fun applyAgents(incoming: List<NodeAgent>) {
        agents = incoming
        if (agents.isEmpty()) { selectedAgentId = null; return }
        val saved = store.selectedAgent()?.first
        val selected = agents.find { it.id == saved } ?: agents.first()
        selectedAgentId = selected.id
        store.selectAgent(selected.id, selected.alias)
    }

    private fun currentAgentAlias(): String = agents.find { it.id == selectedAgentId }?.alias
        ?: store.selectedAgent()?.second ?: "Agent"

    private fun cycleAgent(delta: Int) {
        if (agents.size < 2) return
        val current = agents.indexOfFirst { it.id == selectedAgentId }.coerceAtLeast(0)
        val selected = agents[(current + delta + agents.size) % agents.size]
        selectedAgentId = selected.id
        store.selectAgent(selected.id, selected.alias)
        showTalkScreen()
    }

    private fun onVoiceEvent(state: VoiceSession.State, detail: String?) = runOnUiThread {
        if (isFinishing || isDestroyed) return@runOnUiThread
        when (state) {
            VoiceSession.State.CONNECTING -> {
                starting = true
                voiceState = "connecting"
                orb?.state = "connecting"
                renderVoiceStatus(detail ?: "Connecting…")
            }
            VoiceSession.State.LISTENING -> {
                starting = false
                active = true
                voiceState = "listening"
                orb?.state = "listening"
                renderVoiceStatus(detail ?: "Listening…")
            }
            VoiceSession.State.THINKING -> {
                voiceState = "thinking"
                orb?.state = "thinking"
                renderVoiceStatus(detail ?: "Thinking…")
            }
            VoiceSession.State.SPEAKING -> {
                voiceState = "speaking"
                orb?.state = "speaking"
                renderVoiceStatus(detail ?: "Speaking…")
            }
            VoiceSession.State.ERROR -> {
                stopVoice(false)
                voiceState = "error"
                orb?.state = "error"
                renderVoiceStatus(detail ?: "Voice connection failed")
            }
            VoiceSession.State.IDLE -> {
                active = false
                starting = false
                voiceState = "idle"
                orb?.state = "idle"
                renderVoiceStatus(detail ?: "Tap to talk")
            }
        }
    }

    private fun onAudioLevel(level: Float) = runOnUiThread {
        orb?.level = level * 5f
    }

    private fun renderVoiceStatus(message: String) {
        voiceStatus = message
        if (showingTasks) showDelegations() else showTalkScreen(message)
    }

    private fun onDelegationEvent(status: DelegationStatus) = runOnUiThread {
        if (isFinishing || isDestroyed) return@runOnUiThread
        delegations[status.id] = status
        while (delegations.size > MAX_TASKS) delegations.remove(delegations.keys.first())
        if (showingTasks) showDelegations() else showTalkScreen()
    }

    private fun stopVoice(notify: Boolean = true) {
        active = false
        starting = false
        voiceState = "idle"
        orb?.state = "idle"
        val oldSession = voiceSession
        voiceSession = null
        oldSession?.stop(false)
        client?.close()
        client = null
        if (notify) showTalkScreen("Tap to talk")
    }

    override fun onStop() {
        stopVoice(false)
        super.onStop()
    }

    override fun onDestroy() {
        stopVoice(false)
        super.onDestroy()
    }

    private fun title(text: String) {
        container.addView(TextView(this).apply {
            this.text = text
            textSize = 20f
            typeface = Typeface.DEFAULT_BOLD
            setTextColor(Color.WHITE)
            gravity = Gravity.CENTER
        }, match())
    }

    private fun caption(text: String): TextView = TextView(this).apply {
        this.text = text
        textSize = 13f
        setTextColor(MUTED)
        gravity = Gravity.CENTER
    }

    private fun field(hint: String, value: String, type: Int) = EditText(this).apply {
        this.hint = hint
        setText(value)
        inputType = type
        setSingleLine(true)
        textSize = 13f
        setTextColor(Color.WHITE)
        setHintTextColor(Color.rgb(116, 127, 151))
        setPadding(dp(12), dp(8), dp(12), dp(8))
    }

    private fun actionButton(label: String, action: () -> Unit) = Button(this).apply {
        text = label
        isAllCaps = false
        setOnClickListener { action() }
    }

    private fun roundedBackground(color: Int, radius: Float) = GradientDrawable().apply {
        shape = GradientDrawable.RECTANGLE
        setColor(color)
        cornerRadius = radius
    }

    private fun match(top: Int = 0) = LinearLayout.LayoutParams(
        ViewGroup.LayoutParams.MATCH_PARENT,
        ViewGroup.LayoutParams.WRAP_CONTENT,
    ).apply { topMargin = top }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    private companion object {
        val BACKGROUND = Color.rgb(8, 11, 18)
        val STOP = Color.rgb(184, 62, 82)
        val MUTED = Color.rgb(173, 184, 207)
        val TASK_ACTIVE = Color.rgb(108, 210, 154)
        const val MAX_TASKS = 8
    }
}
