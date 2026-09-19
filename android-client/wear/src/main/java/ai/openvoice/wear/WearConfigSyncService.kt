package ai.openvoice.wear

import android.content.Intent
import android.util.Log
import com.google.android.gms.wearable.*

class WearConfigSyncService : WearableListenerService() {
    private val tag = "WearConfigSyncService"

    override fun onDataChanged(dataEvents: DataEventBuffer) {
        for (event in dataEvents) {
            if (event.type == DataEvent.TYPE_CHANGED && event.dataItem.uri.path == WearConfigSyncHelper.PATH_NODE_CONFIG) {
                val dataMap = DataMapItem.fromDataItem(event.dataItem).dataMap
                val node = WearConfigSyncHelper.parseDataMap(dataMap)
                if (node != null) {
                    Log.i(tag, "Received node configuration from DataClient: ${node.url}")
                    saveAndBroadcast(node)
                }
            }
        }
    }

    override fun onMessageReceived(messageEvent: MessageEvent) {
        if (messageEvent.path == WearConfigSyncHelper.PATH_NODE_CONFIG_RESPONSE) {
            val jsonStr = String(messageEvent.data, Charsets.UTF_8)
            val node = WearConfigSyncHelper.parseJson(jsonStr)
            if (node != null) {
                Log.i(tag, "Received node configuration from MessageClient: ${node.url}")
                saveAndBroadcast(node)
            }
        }
    }

    private fun saveAndBroadcast(node: SavedNode) {
        SecureNodeStore(this).save(node)
        val intent = Intent(WearConfigSyncHelper.ACTION_CONFIG_SYNCED).apply {
            setPackage(packageName)
        }
        sendBroadcast(intent)
    }
}
