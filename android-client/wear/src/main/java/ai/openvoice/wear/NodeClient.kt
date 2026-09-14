@file:Suppress("CustomX509TrustManager")

package ai.openvoice.wear

import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import org.json.JSONObject
import java.io.Closeable
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

class NodeClient(baseUrl: String, pairingCode: String, trustSelfSigned: Boolean) : Closeable {
    val http: OkHttpClient = if (trustSelfSigned) insecureClient() else OkHttpClient()
    private val root = baseUrl.trim().trimEnd('/')
    private val auth = pairingCode.trim()

    fun settings(callback: (Result<NodeSettings>) -> Unit) {
        http.newCall(request("/api/settings")).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) = callback(Result.failure(e))

            override fun onResponse(call: Call, response: Response) = response.use {
                val raw = it.body?.string().orEmpty()
                if (!it.isSuccessful) {
                    callback(Result.failure(IOException(if (it.code == 401) "Pairing code rejected" else "Node returned ${it.code}")))
                    return@use
                }
                callback(runCatching {
                    val json = JSONObject(raw)
                    NodeSettings(
                        realtimeUrl = json.getString("server_url"),
                        sampleRate = json.optInt("sample_rate", 16_000),
                        instructions = json.optString("instructions"),
                        voice = json.optString("voice", "Aiden"),
                        delegationEnabled = json.optBoolean("delegation_enabled", false),
                        delegationToolName = json.optString("delegation_tool_name", "delegate_task"),
                        delegationToolDescription = json.optString("delegation_tool_description", "Hand a task to the more capable brain model."),
                        speakDelegatedResult = json.optBoolean("delegation_speak_result", true),
                    )
                })
            }
        })
    }

    fun delegate(requestText: String, callback: (Result<String>) -> Unit) {
        val json = JSONObject().put("request", requestText).toString()
        http.newCall(request("/api/delegate", "POST", json)).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) = callback(Result.failure(e))

            override fun onResponse(call: Call, response: Response) = response.use {
                val raw = it.body?.string().orEmpty()
                if (!it.isSuccessful) callback(Result.failure(IOException("Delegation returned ${it.code}")))
                else callback(runCatching { JSONObject(raw).getString("answer") })
            }
        })
    }

    private fun request(path: String, method: String = "GET", json: String? = null): Request {
        val body = json?.toRequestBody("application/json".toMediaType())
        return Request.Builder()
            .url(root + path)
            .header("Authorization", "Bearer $auth")
            .method(method, body)
            .build()
    }

    override fun close() {
        http.dispatcher.cancelAll()
        http.connectionPool.evictAll()
    }

    private companion object {
        fun insecureClient(): OkHttpClient {
            val trust = object : X509TrustManager {
                override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) = Unit
                override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) = Unit
                override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
            }
            val context = SSLContext.getInstance("TLS").apply {
                init(null, arrayOf<TrustManager>(trust), SecureRandom())
            }
            return OkHttpClient.Builder()
                .sslSocketFactory(context.socketFactory, trust)
                .hostnameVerifier { _, _ -> true }
                .build()
        }
    }
}
