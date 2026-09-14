package ai.openvoice.wear

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

data class SavedNode(val url: String, val pairingCode: String, val trustSelfSigned: Boolean)

class SecureNodeStore(context: Context) {
    private val preferences: SharedPreferences = EncryptedSharedPreferences.create(
        context,
        "paired_node",
        MasterKey.Builder(context).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    fun load(): SavedNode? {
        val url = preferences.getString(URL, null)?.trim().orEmpty()
        val code = preferences.getString(CODE, null).orEmpty()
        return if (url.isBlank() || code.isBlank()) null else SavedNode(url, code, preferences.getBoolean(TRUST, false))
    }

    fun save(node: SavedNode) {
        preferences.edit()
            .putString(URL, node.url)
            .putString(CODE, node.pairingCode)
            .putBoolean(TRUST, node.trustSelfSigned)
            .apply()
    }

    fun clear() = preferences.edit().clear().apply()

    private companion object {
        const val URL = "url"
        const val CODE = "pairing_code"
        const val TRUST = "trust_self_signed"
    }
}
