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
    private var starting = false
    private var active = false
    private var showingTasks = false
    private var voiceStatus = "Tap to talk"
    private var agents: List<NodeAgent> = emptyList()
    private var selectedAgentId: String? = null
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
            val side = dp(if (resources.configuration.isScreenRound) 28 else 18)
            setPadding(side, dp(18), side, dp(22))
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

    // This is a short, node-issued pairing secret rather than an account
    // password. Direct entry is intentional so the standalone watch can pair
    // without requiring the phone app, and the field remains masked.
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
                        // Preserve the address the user knows reaches the node;
                        // base_url can differ behind a TLS-terminating proxy.
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
        title("Open Voice")
        val status = caption(message ?: voiceStatus)
        val talk = Button(this).apply {
            text = if (active || starting) "■" else "●"
            textSize = 34f
            setTextColor(Color.WHITE)
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(if (active || starting) STOP else ACCENT)
            }
            contentDescription = if (active || starting) "Stop talking" else "Start talking"
            setOnClickListener { if (active || starting) stopVoice() else requestTalk() }
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
        container.addView(talk, LinearLayout.LayoutParams(dp(112), dp(112)).apply {
            gravity = Gravity.CENTER_HORIZONTAL
            topMargin = dp(14)
        })
        container.addView(caption(currentAgentAlias()).apply { textSize = 12f; setTextColor(Color.WHITE) }, match(dp(3)))
        if (agents.size > 1) container.addView(caption("Swipe orb to change agent").apply { textSize = 9f }, match(dp(1)))
        container.addView(status, match(dp(12)))
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
                topMargin = dp(8)
            })
        }
        container.addView(actionButton("Forget & re-pair") { showForgetConfirmation() }.apply {
            textSize = 12f
            setTextColor(MUTED)
            background = null
        }, match(dp(8)))
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
        showTalkScreen("Connecting…")
        val newClient = NodeClient(node.url, node.accessToken, node.trustSelfSigned)
        client = newClient
        newClient.settings { result ->
            runOnUiThread {
                if (client !== newClient || isFinishing || isDestroyed) return@runOnUiThread
                result.fold(
                    onSuccess = { settings ->
                        applyAgents(settings.agents)
                        val session = VoiceSession(this, newClient, settings, { selectedAgentId }, ::onVoiceEvent, ::onDelegationEvent)
                        voiceSession = session
                        session.start()
                    },
                    onFailure = {
                        starting = false
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
                renderVoiceStatus(detail ?: "Connecting…")
            }
            VoiceSession.State.LISTENING -> {
                starting = false
                active = true
                renderVoiceStatus(detail ?: "Listening…")
            }
            VoiceSession.State.THINKING -> renderVoiceStatus(detail ?: "Thinking…")
            VoiceSession.State.SPEAKING -> renderVoiceStatus(detail ?: "Speaking…")
            VoiceSession.State.ERROR -> {
                stopVoice(false)
                renderVoiceStatus(detail ?: "Voice connection failed")
            }
            VoiceSession.State.IDLE -> {
                active = false
                starting = false
                renderVoiceStatus(detail ?: "Tap to talk")
            }
        }
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
            textSize = 21f
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
        val ACCENT = Color.rgb(70, 100, 210)
        val STOP = Color.rgb(184, 62, 82)
        val MUTED = Color.rgb(173, 184, 207)
        val TASK_ACTIVE = Color.rgb(108, 210, 154)
        const val MAX_TASKS = 8
    }
}
