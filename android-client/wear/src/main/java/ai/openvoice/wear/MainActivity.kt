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

class MainActivity : ComponentActivity() {
    private lateinit var store: SecureNodeStore
    private lateinit var container: LinearLayout
    private var client: NodeClient? = null
    private var voiceSession: VoiceSession? = null
    private var starting = false
    private var active = false

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
        if (store.load() == null) showPairingScreen() else showTalkScreen()
    }

    // This is a short, node-issued pairing secret rather than an account
    // password. Direct entry is intentional so the standalone watch can pair
    // without requiring the phone app, and the field remains masked.
    @SuppressLint("WearPasswordInput")
    private fun showPairingScreen(message: String? = null) {
        stopVoice(false)
        container.removeAllViews()
        title("Pair watch")
        container.addView(caption(message ?: "Connect directly to your Open Voice node"), match(dp(5)))
        val saved = store.load()
        val url = field("Node URL", saved?.url.orEmpty(), InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        val code = field(
            "Pairing code",
            saved?.pairingCode.orEmpty(),
            InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD,
        )
        val trust = CheckBox(this).apply {
            setText(R.string.trust_self_signed)
            setTextColor(MUTED)
            textSize = 12f
            isChecked = saved?.trustSelfSigned ?: false
        }
        container.addView(url, match(dp(12)))
        container.addView(code, match(dp(8)))
        container.addView(trust, match(dp(4)))
        container.addView(actionButton("Pair") {
            val cleanUrl = url.text.toString().trim().trimEnd('/')
            val cleanCode = code.text.toString().trim()
            if ((!cleanUrl.startsWith("http://") && !cleanUrl.startsWith("https://")) || cleanCode.isBlank()) {
                showPairingScreen("Enter a node URL and pairing code")
                return@actionButton
            }
            verifyPairing(SavedNode(cleanUrl, cleanCode, trust.isChecked))
        }, match(dp(10)))
    }

    private fun verifyPairing(node: SavedNode) {
        container.removeAllViews()
        title("Pairing…")
        container.addView(caption("Checking the node"), match(dp(5)))
        val checkingClient = NodeClient(node.url, node.pairingCode, node.trustSelfSigned)
        client = checkingClient
        checkingClient.settings { result ->
            runOnUiThread {
                if (client !== checkingClient || isFinishing || isDestroyed) return@runOnUiThread
                result.fold(
                    onSuccess = {
                        store.save(node)
                        checkingClient.close()
                        client = null
                        showTalkScreen("Paired")
                    },
                    onFailure = {
                        checkingClient.close()
                        client = null
                        showPairingScreen(it.message ?: "Could not reach node")
                    },
                )
            }
        }
    }

    private fun showTalkScreen(message: String? = null) {
        container.removeAllViews()
        title("Open Voice")
        val status = caption(message ?: if (active) "Listening…" else "Tap to talk")
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
        }
        container.addView(talk, LinearLayout.LayoutParams(dp(112), dp(112)).apply {
            gravity = Gravity.CENTER_HORIZONTAL
            topMargin = dp(14)
        })
        container.addView(status, match(dp(12)))
        container.addView(actionButton("Pairing settings") { showPairingScreen() }.apply {
            textSize = 12f
            setTextColor(MUTED)
            background = null
        }, match(dp(8)))
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
        val newClient = NodeClient(node.url, node.pairingCode, node.trustSelfSigned)
        client = newClient
        newClient.settings { result ->
            runOnUiThread {
                if (client !== newClient || isFinishing || isDestroyed) return@runOnUiThread
                result.fold(
                    onSuccess = { settings ->
                        val session = VoiceSession(this, newClient, settings, ::onVoiceEvent)
                        voiceSession = session
                        session.start()
                    },
                    onFailure = {
                        starting = false
                        newClient.close()
                        client = null
                        showTalkScreen(it.message ?: "Could not connect")
                    },
                )
            }
        }
    }

    private fun onVoiceEvent(state: VoiceSession.State, detail: String?) = runOnUiThread {
        if (isFinishing || isDestroyed) return@runOnUiThread
        when (state) {
            VoiceSession.State.CONNECTING -> {
                starting = true
                showTalkScreen(detail ?: "Connecting…")
            }
            VoiceSession.State.LISTENING -> {
                starting = false
                active = true
                showTalkScreen(detail ?: "Listening…")
            }
            VoiceSession.State.THINKING -> showTalkScreen(detail ?: "Thinking…")
            VoiceSession.State.SPEAKING -> showTalkScreen(detail ?: "Speaking…")
            VoiceSession.State.ERROR -> {
                stopVoice(false)
                showTalkScreen(detail ?: "Voice connection failed")
            }
            VoiceSession.State.IDLE -> {
                active = false
                starting = false
                showTalkScreen(detail ?: "Tap to talk")
            }
        }
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
    }
}
