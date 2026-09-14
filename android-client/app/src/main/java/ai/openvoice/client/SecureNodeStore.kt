package ai.openvoice.client

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

data class SavedNode(
    val url: String,
    val credential: String,
    val deviceId: String,
    val trustSelfSigned: Boolean,
)

/** Keeps the per-device bearer credential in Android Keystore-backed storage. */
class SecureNodeStore(context: Context) {
    private val preferences = EncryptedSharedPreferences.create(
        context,
        "paired_node_secure",
        MasterKey.Builder(context).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    fun load(): SavedNode? {
        val url = preferences.getString(URL, null)?.trim().orEmpty()
        val credential = preferences.getString(CREDENTIAL, null).orEmpty()
        if (url.isBlank() || credential.isBlank()) return null
        return SavedNode(
            url,
            credential,
            preferences.getString(DEVICE_ID, "").orEmpty(),
            preferences.getBoolean(TRUST, false),
        )
    }

    fun save(node: SavedNode) = preferences.edit()
        .putString(URL, node.url)
        .putString(CREDENTIAL, node.credential)
        .putString(DEVICE_ID, node.deviceId)
        .putBoolean(TRUST, node.trustSelfSigned)
        .apply()

    fun clear() = preferences.edit().clear().apply()

    private companion object {
        const val URL = "url"
        const val CREDENTIAL = "credential"
        const val DEVICE_ID = "device_id"
        const val TRUST = "trust_self_signed"
    }
}
