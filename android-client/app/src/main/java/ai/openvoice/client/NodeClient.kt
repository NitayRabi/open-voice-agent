package ai.openvoice.client

import okhttp3.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.io.IOException
import java.security.SecureRandom
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager

data class NodeSettings(
    val realtimeUrl: String,
    val sampleRate: Int,
    val instructions: String,
    val voice: String,
    val delegationEnabled: Boolean,
    val delegationToolName: String,
    val delegationToolDescription: String,
    val speakDelegatedResult: Boolean,
)

class NodeClient(private val baseUrl: String, private val pairingCode: String, trustSelfSigned: Boolean = false) {
    val http: OkHttpClient = if (trustSelfSigned) insecureClient() else sharedHttp

    companion object {
        private val sharedHttp = OkHttpClient()

        // Deliberately opt-in and scoped to this NodeClient instance. This is
        // useful for a private LAN/Tailnet node with the desktop app's generated
        // certificate, but should not be enabled for an untrusted network.
        private fun insecureClient(): OkHttpClient {
            val trust = object : X509TrustManager {
                override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) = Unit
                override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) = Unit
                override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
            }
            val context = SSLContext.getInstance("TLS").apply { init(null, arrayOf<TrustManager>(trust), SecureRandom()) }
            return OkHttpClient.Builder().sslSocketFactory(context.socketFactory, trust).hostnameVerifier { _, _ -> true }.build()
        }
    }

    private fun request(path: String, method: String = "GET", json: String? = null): Request {
        val body = json?.toRequestBody("application/json".toMediaType())
        return Request.Builder()
            .url(baseUrl.trimEnd('/') + path)
            .header("Authorization", "Bearer $pairingCode")
            .method(method, body)
            .build()
    }

    fun settings(callback: (Result<NodeSettings>) -> Unit) {
        http.newCall(request("/api/settings")).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) = callback(Result.failure(e))
            override fun onResponse(call: Call, response: Response) = response.use {
                val raw = it.body?.string().orEmpty()
                if (!it.isSuccessful) return callback(Result.failure(IOException(if (it.code == 401) "Pairing code rejected" else "Node returned ${it.code}")))
                runCatching {
                    val j = JSONObject(raw)
                    NodeSettings(
                        realtimeUrl = j.getString("server_url"),
                        sampleRate = j.optInt("sample_rate", 16000),
                        instructions = j.optString("instructions"),
                        voice = j.optString("voice", "Aiden"),
                        delegationEnabled = j.optBoolean("delegation_enabled", false),
                        delegationToolName = j.optString("delegation_tool_name", "delegate_task"),
                        delegationToolDescription = j.optString("delegation_tool_description", "Hand a task to the more capable brain model."),
                        speakDelegatedResult = j.optBoolean("delegation_speak_result", true),
                    )
                }.let(callback)
            }
        })
    }

    fun delegate(requestText: String, callback: (Result<String>) -> Unit) {
        val body = JSONObject().put("request", requestText).toString()
        http.newCall(request("/api/delegate", "POST", body)).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) = callback(Result.failure(e))
            override fun onResponse(call: Call, response: Response) = response.use {
                val raw = it.body?.string().orEmpty()
                if (!it.isSuccessful) callback(Result.failure(IOException("Delegation returned ${it.code}")))
                else callback(runCatching { JSONObject(raw).getString("answer") })
            }
        })
    }
}
