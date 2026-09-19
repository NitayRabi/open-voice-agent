package ai.openvoice.client

import android.util.Log
import com.google.android.gms.wearable.MessageEvent
import com.google.android.gms.wearable.Wearable
import com.google.android.gms.wearable.WearableListenerService

class PhoneWearListenerService : WearableListenerService() {
    private val tag = "PhoneWearListener"

    override fun onMessageReceived(messageEvent: MessageEvent) {
        if (messageEvent.path == PhoneWearSyncHelper.PATH_REQUEST_SYNC) {
            Log.i(tag, "Received sync request from Wear node: ${messageEvent.sourceNodeId}")
            val store = SecureNodeStore(this)
            val saved = store.load()
            if (saved != null) {
                // Update data layer
                PhoneWearSyncHelper.syncNodeToWear(this, saved)

                // Send direct message response
                val payload = PhoneWearSyncHelper.buildConfigJson(saved)
                Wearable.getMessageClient(this)
                    .sendMessage(messageEvent.sourceNodeId, PhoneWearSyncHelper.PATH_NODE_CONFIG_RESPONSE, payload)
                    .addOnSuccessListener {
                        Log.i(tag, "Sent node config response to Wear node: ${messageEvent.sourceNodeId}")
                    }
                    .addOnFailureListener { e ->
                        Log.w(tag, "Failed to send node config response to Wear node", e)
                    }
            } else {
                Log.i(tag, "No saved node available to sync to Wear")
            }
        }
    }
}
