package ai.openvoice.client

import android.content.Context
import android.util.Log
import com.google.android.gms.wearable.PutDataMapRequest
import com.google.android.gms.wearable.Wearable
import org.json.JSONObject

object PhoneWearSyncHelper {
    private const val TAG = "PhoneWearSync"
    const val PATH_NODE_CONFIG = "/openvoice/node_config"
    const val PATH_REQUEST_SYNC = "/openvoice/request_sync"
    const val PATH_NODE_CONFIG_RESPONSE = "/openvoice/node_config_response"

    const val KEY_URL = "url"
    const val KEY_CREDENTIAL = "credential"
    const val KEY_TRUST = "trust_self_signed"
    const val KEY_TIMESTAMP = "timestamp"

    fun syncNodeToWear(context: Context, savedNode: SavedNode) {
        try {
            val req = PutDataMapRequest.create(PATH_NODE_CONFIG).apply {
                dataMap.putString(KEY_URL, savedNode.url)
                dataMap.putString(KEY_CREDENTIAL, savedNode.credential)
                dataMap.putBoolean(KEY_TRUST, savedNode.trustSelfSigned)
                dataMap.putLong(KEY_TIMESTAMP, System.currentTimeMillis())
            }.asPutDataRequest().setUrgent()

            Wearable.getDataClient(context).putDataItem(req)
                .addOnSuccessListener {
                    Log.i(TAG, "Synced node configuration to Wearable data layer")
                }
                .addOnFailureListener { e ->
                    Log.w(TAG, "Failed to sync node configuration to Wearable data layer", e)
                }
        } catch (e: Exception) {
            Log.w(TAG, "Error initiating Wearable sync", e)
        }
    }

    fun buildConfigJson(savedNode: SavedNode): ByteArray {
        return JSONObject().apply {
            put(KEY_URL, savedNode.url)
            put(KEY_CREDENTIAL, savedNode.credential)
            put(KEY_TRUST, savedNode.trustSelfSigned)
            put(KEY_TIMESTAMP, System.currentTimeMillis())
        }.toString().toByteArray(Charsets.UTF_8)
    }
}
