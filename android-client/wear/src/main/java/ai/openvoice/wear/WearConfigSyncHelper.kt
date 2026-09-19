package ai.openvoice.wear

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.google.android.gms.wearable.DataMap
import com.google.android.gms.wearable.Wearable
import org.json.JSONObject

object WearConfigSyncHelper {
    private const val TAG = "WearConfigSync"
    const val PATH_NODE_CONFIG = "/openvoice/node_config"
    const val PATH_REQUEST_SYNC = "/openvoice/request_sync"
    const val PATH_NODE_CONFIG_RESPONSE = "/openvoice/node_config_response"
    const val ACTION_CONFIG_SYNCED = "ai.openvoice.wear.ACTION_CONFIG_SYNCED"

    const val KEY_URL = "url"
    const val KEY_CREDENTIAL = "credential"
    const val KEY_ACCESS_TOKEN = "access_token"
    const val KEY_TRUST = "trust_self_signed"

    fun parseDataMap(dataMap: DataMap): SavedNode? {
        val url = dataMap.getString(KEY_URL)?.trim()?.trimEnd('/') ?: return null
        val token = dataMap.getString(KEY_CREDENTIAL)
            ?: dataMap.getString(KEY_ACCESS_TOKEN)
            ?: return null
        if (url.isBlank() || token.isBlank()) return null
        val trust = dataMap.getBoolean(KEY_TRUST, false)
        return SavedNode(url, token, trust)
    }

    fun parseJson(jsonStr: String): SavedNode? {
        return try {
            val json = JSONObject(jsonStr)
            val url = json.optString(KEY_URL, "").trim().trimEnd('/')
            val token = when {
                json.has(KEY_CREDENTIAL) -> json.optString(KEY_CREDENTIAL, "")
                json.has(KEY_ACCESS_TOKEN) -> json.optString(KEY_ACCESS_TOKEN, "")
                else -> ""
            }
            if (url.isBlank() || token.isBlank()) return null
            val trust = json.optBoolean(KEY_TRUST, false)
            SavedNode(url, token, trust)
        } catch (_: Throwable) {
            null
        }
    }

    fun requestSyncFromPhone(
        context: Context,
        timeoutMs: Long = 3500L,
        onComplete: (Boolean) -> Unit,
    ) {
        val handler = Handler(Looper.getMainLooper())
        var finished = false

        fun finishOnce(success: Boolean) {
            if (!finished) {
                finished = true
                onComplete(success)
            }
        }

        val timeoutRunnable = Runnable {
            try {
                Log.w(TAG, "Sync request timed out")
            } catch (_: Throwable) {}
            finishOnce(false)
        }
        handler.postDelayed(timeoutRunnable, timeoutMs)

        try {
            val nodeClient = Wearable.getNodeClient(context)
            val messageClient = Wearable.getMessageClient(context)

            nodeClient.connectedNodes
                .addOnSuccessListener { nodes ->
                    val targets = nodes.filter { !it.isNearby || true }
                    if (targets.isEmpty()) {
                        try {
                            Log.i(TAG, "No connected phone nodes found for sync")
                        } catch (_: Throwable) {}
                        handler.removeCallbacks(timeoutRunnable)
                        finishOnce(false)
                        return@addOnSuccessListener
                    }

                    var sent = false
                    for (node in targets) {
                        messageClient.sendMessage(node.id, PATH_REQUEST_SYNC, ByteArray(0))
                            .addOnSuccessListener {
                                try {
                                    Log.i(TAG, "Sent sync request to phone node: ${node.displayName} (${node.id})")
                                } catch (_: Throwable) {}
                            }
                            .addOnFailureListener { e ->
                                try {
                                    Log.w(TAG, "Failed sending sync request to ${node.id}", e)
                                } catch (_: Throwable) {}
                            }
                        sent = true
                    }
                    if (!sent) {
                        handler.removeCallbacks(timeoutRunnable)
                        finishOnce(false)
                    }
                }
                .addOnFailureListener { e ->
                    try {
                        Log.w(TAG, "Failed querying connected nodes", e)
                    } catch (_: Throwable) {}
                    handler.removeCallbacks(timeoutRunnable)
                    finishOnce(false)
                }
        } catch (e: Exception) {
            try {
                Log.w(TAG, "Error initiating phone sync request", e)
            } catch (_: Throwable) {}
            handler.removeCallbacks(timeoutRunnable)
            finishOnce(false)
        }
    }
}
