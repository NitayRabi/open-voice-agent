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

data class PairingCredential(
    val accessToken: String,
    val baseUrl: String?,
    val deviceId: String?,
)

class NodeAccessRevokedException(message: String) : IOException(message)

internal fun pairingRequestBody(code: String): String = JSONObject()
    .put("code", code.trim())
    .put("device", JSONObject().put("type", "wear").put("label", "Pixel Watch"))
    .toString()

internal fun parsePairingResponse(raw: String): PairingCredential {
    val json = JSONObject(raw)
    val token = json.optString("access_token").trim()
    require(token.isNotEmpty()) { "Node did not return a device access token" }
    return PairingCredential(
        accessToken = token,
        baseUrl = json.optString("base_url").trim().ifEmpty { null },
        deviceId = json.optJSONObject("device")?.optString("id")?.trim()?.ifEmpty { null },
    )
}

class NodeClient(baseUrl: String, accessToken: String, trustSelfSigned: Boolean) : Closeable {
    val http: OkHttpClient = if (trustSelfSigned) insecureClient() else OkHttpClient()
    private val root = baseUrl.trim().trimEnd('/')
    private val auth = accessToken.trim()

    /** Exchange the reusable desktop pairing code for this watch's revocable credential. */
    fun pair(code: String, callback: (Result<PairingCredential>) -> Unit) {
        val body = pairingRequestBody(code).toRequestBody("application/json".toMediaType())
        val request = Request.Builder().url(root + "/api/access/pair").post(body).build()
        http.newCall(request).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) = callback(Result.failure(e))

            override fun onResponse(call: Call, response: Response) = response.use {
                val raw = it.body?.string().orEmpty()
                if (!it.isSuccessful) {
                    val message = runCatching { JSONObject(raw).optString("error") }.getOrNull()
                        ?.takeIf(String::isNotBlank)
                        ?: if (it.code == 401) "Pairing code rejected" else "Pairing returned ${it.code}"
                    callback(Result.failure(IOException(message)))
                } else {
                    callback(runCatching { parsePairingResponse(raw) })
                }
            }
        })
    }

    fun settings(callback: (Result<NodeSettings>) -> Unit) {
        http.newCall(request("/api/settings")).enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) = callback(Result.failure(e))

            override fun onResponse(call: Call, response: Response) = response.use {
                val raw = it.body?.string().orEmpty()
                if (!it.isSuccessful) {
                    callback(Result.failure(if (it.code == 401) NodeAccessRevokedException("Watch access expired or was revoked") else IOException("Node returned ${it.code}")))
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
                if (!it.isSuccessful) callback(Result.failure(if (it.code == 401) NodeAccessRevokedException("Watch access expired or was revoked") else IOException("Delegation returned ${it.code}")))
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

    fun realtimeRequest(url: String): Request = Request.Builder()
        .url(url)
        .header("Authorization", "Bearer $auth")
        .header("Sec-WebSocket-Protocol", "realtime, openai-insecure-api-key.open-voice-agent, openai-beta.realtime-v1")
        .build()

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
