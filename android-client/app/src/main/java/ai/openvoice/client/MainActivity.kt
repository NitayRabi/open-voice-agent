package ai.openvoice.client

import android.Manifest
import android.content.*
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.view.ViewGroup
import android.widget.*
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : AppCompatActivity() {
    private lateinit var url: EditText
    private lateinit var code: EditText
    private lateinit var status: TextView
    private lateinit var orb: OrbView
    private lateinit var agentButton: Button
    private lateinit var trustSelfSigned: CheckBox
    private lateinit var store: SecureNodeStore
    private var enableOverlayAfterPermission = false
    private var agents: List<NodeAgent> = emptyList()
    private var selectedAgentId: String? = null

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            val s = intent?.getStringExtra(VoiceService.EXTRA_STATE) ?: return
            orb.state = s; status.text = intent.getStringExtra(VoiceService.EXTRA_MESSAGE) ?: s.replace('_', ' ')
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.statusBarColor = Color.rgb(9, 11, 18)
        store = SecureNodeStore(this)
        val saved = store.load()
        url = field("Node URL", saved?.url.orEmpty())
        code = field("Pairing code (blank when already paired)", "").apply { inputType = 0x81 }
        trustSelfSigned = CheckBox(this).apply {
            text = "Trust this node's self-signed certificate"
            setTextColor(Color.rgb(190,198,216))
            isChecked = saved?.trustSelfSigned ?: false
        }
        status = TextView(this).apply { text = if (saved == null) "Pair with your remote node" else "Paired — tap to talk"; setTextColor(Color.rgb(174,184,202)); textSize = 15f; gravity = Gravity.CENTER }
        orb = OrbView(this).apply { layoutParams = LinearLayout.LayoutParams(dp(190), dp(190)).also { it.gravity = Gravity.CENTER_HORIZONTAL }; setOnClickListener { startVoice(false) } }
        agentButton = Button(this).apply { text = saved?.let { store.selectedAgent()?.second } ?: "Agent"; isAllCaps = false; setOnClickListener { cycleAgent() } }
        val orbRow = LinearLayout(this).apply { gravity = Gravity.CENTER; addView(orb); addView(agentButton, LinearLayout.LayoutParams(dp(92), ViewGroup.LayoutParams.WRAP_CONTENT)) }
        val connect = Button(this).apply { text = "Connect"; setOnClickListener { startVoice(false) } }
        val floating = Button(this).apply { text = "Enable floating orb"; setOnClickListener { startVoice(true) } }
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER_HORIZONTAL; setPadding(dp(28), dp(36), dp(28), dp(24)); setBackgroundColor(Color.rgb(9,11,18))
            addView(TextView(this@MainActivity).apply { text = "Open Voice Agent"; textSize = 28f; setTextColor(Color.WHITE); gravity = Gravity.CENTER; setPadding(0,0,0,dp(10)) }, match())
            addView(TextView(this@MainActivity).apply { text = "A remote-only Android client"; textSize = 14f; setTextColor(Color.rgb(130,145,176)); gravity = Gravity.CENTER }, match())
            addView(orbRow); addView(status, match()); addView(url, match(dp(14))); addView(code, match(dp(10))); addView(trustSelfSigned, match(dp(8))); addView(connect, match(dp(12))); addView(floating, match(dp(8)))
            addView(TextView(this@MainActivity).apply { text = "The node URL is the web-client URL (for example https://voice.example.com). No server or model runs on this phone."; textSize = 13f; setTextColor(Color.rgb(121,131,151)); setPadding(4,dp(18),4,0) }, match())
        }
        setContentView(ScrollView(this).apply { addView(content) })
        requestRuntimePermissions()
        if (saved != null) loadAgents(saved)
    }

    private fun field(hintText: String, value: String) = EditText(this).apply {
        hint = hintText; setHintTextColor(Color.rgb(105,115,140)); setTextColor(Color.WHITE); setText(value); setSingleLine(); setPadding(dp(14),dp(12),dp(14),dp(12))
    }
    private fun match(top: Int = 0) = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = top }

    private fun startVoice(withOverlay: Boolean) {
        val cleanUrl = url.text.toString().trim().trimEnd('/')
        if (!cleanUrl.startsWith("http://") && !cleanUrl.startsWith("https://")) { status.text = "Enter an http:// or https:// node URL"; return }
        val enteredCode = code.text.toString().trim()
        val saved = store.load()?.takeIf { it.url == cleanUrl && it.trustSelfSigned == trustSelfSigned.isChecked }
        if (enteredCode.isNotEmpty()) {
            status.text = "Pairing this phone…"
            val client = NodeClient(cleanUrl, "", trustSelfSigned.isChecked)
            client.pair(enteredCode, "${Build.MANUFACTURER} ${Build.MODEL}".trim()) { result ->
                runOnUiThread {
                    result.fold(
                        onSuccess = {
                            store.save(SavedNode(cleanUrl, it.accessToken, it.deviceId, trustSelfSigned.isChecked))
                            code.setText("")
                            status.text = "Paired"
                            loadAgents(store.load()!!)
                            launchVoice(withOverlay)
                        },
                        onFailure = { status.text = it.message ?: "Pairing failed" },
                    )
                }
            }
            return
        }
        if (saved == null) { status.text = "Enter the pairing code shown by the desktop app"; return }
        launchVoice(withOverlay)
    }

    private fun launchVoice(withOverlay: Boolean) {
        if (withOverlay && !Settings.canDrawOverlays(this)) {
            enableOverlayAfterPermission = true
            showOverlayHelp(); return
        }
        val action = if (withOverlay) VoiceService.ACTION_SHOW else VoiceService.ACTION_TOGGLE
        ContextCompat.startForegroundService(this, Intent(this, VoiceService::class.java).setAction(action))
        status.text = if (withOverlay) "Floating orb enabled" else "connecting"
    }

    private fun loadAgents(saved: SavedNode) {
        NodeClient(saved.url, saved.credential, saved.trustSelfSigned).settings { result ->
            result.onSuccess { cfg -> runOnUiThread { applyAgents(cfg.agents) } }
        }
    }

    private fun applyAgents(incoming: List<NodeAgent>) {
        agents = incoming
        if (agents.isEmpty()) { selectedAgentId = null; agentButton.text = "Agent"; agentButton.isEnabled = false; return }
        val savedId = store.selectedAgent()?.first
        val selected = agents.find { it.id == savedId } ?: agents.first()
        selectedAgentId = selected.id
        store.selectAgent(selected.id, selected.alias)
        agentButton.text = selected.alias
        agentButton.isEnabled = agents.size > 1
    }

    private fun cycleAgent() {
        if (agents.size < 2) return
        val index = agents.indexOfFirst { it.id == selectedAgentId }.coerceAtLeast(0)
        val selected = agents[(index + 1) % agents.size]
        selectedAgentId = selected.id
        store.selectAgent(selected.id, selected.alias)
        agentButton.text = selected.alias
    }

    private fun showOverlayHelp() {
        AlertDialog.Builder(this)
            .setTitle("Allow the floating orb")
            .setMessage("Because this APK was installed outside Google Play, Android may block this permission. If it says “App was denied access”, open App info, tap the three-dot menu, choose “Allow restricted settings”, then try again.")
            .setPositiveButton("Open overlay setting") { _, _ ->
                startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            }
            .setNeutralButton("Open app info") { _, _ ->
                startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")))
            }
            .setNegativeButton("Cancel") { _, _ -> enableOverlayAfterPermission = false }
            .show()
    }

    override fun onResume() {
        super.onResume()
        if (enableOverlayAfterPermission && Settings.canDrawOverlays(this)) { enableOverlayAfterPermission = false; startVoice(true) }
    }
    override fun onStart() { super.onStart(); ContextCompat.registerReceiver(this, receiver, IntentFilter(VoiceService.ACTION_STATE), ContextCompat.RECEIVER_NOT_EXPORTED) }
    override fun onStop() { unregisterReceiver(receiver); super.onStop() }

    private fun requestRuntimePermissions() {
        val missing = mutableListOf<String>()
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) missing += Manifest.permission.RECORD_AUDIO
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) missing += Manifest.permission.POST_NOTIFICATIONS
        if (missing.isNotEmpty()) ActivityCompat.requestPermissions(this, missing.toTypedArray(), 10)
    }
    private fun dp(n: Int) = (n * resources.displayMetrics.density).toInt()
}
