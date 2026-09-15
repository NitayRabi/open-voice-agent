package ai.openvoice.wear

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

data class SavedNode(val url: String, val accessToken: String, val trustSelfSigned: Boolean)

class SecureNodeStore(context: Context) {
    private val preferences: SharedPreferences = EncryptedSharedPreferences.create(
        context,
        "paired_node",
        MasterKey.Builder(context).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    init {
        // Remove the pre-device-credential field on upgrade. A pairing code is
        // an exchange secret and must never remain as the watch credential.
        if (preferences.contains(LEGACY_PAIRING_CODE)) {
            preferences.edit().remove(LEGACY_PAIRING_CODE).apply()
        }
    }

    fun load(): SavedNode? {
        val url = preferences.getString(URL, null)?.trim().orEmpty()
        val accessToken = preferences.getString(ACCESS_TOKEN, null).orEmpty()
        return if (url.isBlank() || accessToken.isBlank()) null else {
            SavedNode(url, accessToken, preferences.getBoolean(TRUST, false))
        }
    }

    fun save(node: SavedNode) {
        preferences.edit()
            .putString(URL, node.url)
            .putString(ACCESS_TOKEN, node.accessToken)
            .putBoolean(TRUST, node.trustSelfSigned)
            .apply()
    }

    fun clear() = preferences.edit().clear().apply()

    fun selectedAgent(): Pair<String, String>? {
        val id = preferences.getString(AGENT_ID, null)?.takeIf { it.isNotBlank() } ?: return null
        return id to preferences.getString(AGENT_ALIAS, "Agent").orEmpty()
    }

    fun selectAgent(id: String, alias: String) = preferences.edit()
        .putString(AGENT_ID, id).putString(AGENT_ALIAS, alias).apply()

    private companion object {
        const val URL = "url"
        // The reusable/manual pairing code is deliberately never persisted.
        const val ACCESS_TOKEN = "access_token"
        const val LEGACY_PAIRING_CODE = "pairing_code"
        const val AGENT_ID = "agent_id"
        const val AGENT_ALIAS = "agent_alias"
        const val TRUST = "trust_self_signed"
    }
}
