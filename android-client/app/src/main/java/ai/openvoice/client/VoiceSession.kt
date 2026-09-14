package ai.openvoice.client

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.*
import android.util.Base64
import androidx.core.content.ContextCompat
import okhttp3.*
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean

class VoiceSession(
    private val context: Context,
    private val node: NodeClient,
    private val settings: NodeSettings,
    private val event: (String, String?) -> Unit,
) {
    private var socket: WebSocket? = null
    private var recorder: AudioRecord? = null
    private var player: AudioTrack? = null
    private val active = AtomicBoolean(false)
    @Volatile private var responseActive = false
    private val reports = ArrayDeque<String>()

    @SuppressLint("MissingPermission")
    fun start() {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED)
            return event("error", "Microphone permission is required")
        event("connecting", null)
        val rate = settings.sampleRate
        val inputSize = maxOf(AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT), rate / 5)
        recorder = AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, inputSize)
        val attrs = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANT).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()
        val format = AudioFormat.Builder().setSampleRate(rate).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build()
        player = AudioTrack(attrs, format, maxOf(AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT), rate), AudioTrack.MODE_STREAM, AudioManager.AUDIO_SESSION_ID_GENERATE).also { it.play() }

        val req = node.realtimeRequest(settings.realtimeUrl)
        socket = node.http.newWebSocket(req, object : WebSocketListener() {
            override fun onOpen(ws: WebSocket, response: Response) {
                active.set(true); ws.send(sessionUpdate().toString()); event("listening", null); captureLoop()
            }
            override fun onMessage(ws: WebSocket, text: String) = handle(JSONObject(text))
            override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) { active.set(false); event("error", t.message) }
            override fun onClosed(ws: WebSocket, code: Int, reason: String) { if (active.getAndSet(false)) event("idle", null) }
        })
    }

    fun stop() {
        active.set(false); recorder?.stopSafely(); recorder?.release(); recorder = null
        player?.pause(); player?.flush(); player?.release(); player = null
        socket?.close(1000, "stopped"); socket = null; event("idle", null)
    }

    private fun AudioRecord.stopSafely() = try { stop() } catch (_: Exception) {}

    private fun captureLoop() = Thread {
        val r = recorder ?: return@Thread
        val samples = ShortArray(settings.sampleRate * 40 / 1000)
        r.startRecording()
        while (active.get()) {
            val n = r.read(samples, 0, samples.size)
            if (n > 0) {
                var sum = 0.0
                val bytes = ByteArray(n * 2)
                for (i in 0 until n) {
                    val v = samples[i].toInt(); sum += v.toDouble() * v
                    bytes[i * 2] = v.toByte(); bytes[i * 2 + 1] = (v shr 8).toByte()
                }
                event("level", (kotlin.math.sqrt(sum / n) / 32768.0).toString())
                socket?.send(JSONObject().put("type", "input_audio_buffer.append").put("audio", Base64.encodeToString(bytes, Base64.NO_WRAP)).toString())
            }
        }
    }.apply { name = "ova-microphone"; start() }

    private fun sessionUpdate(): JSONObject {
        // OpenAI's Realtime schema only accepts an explicitly declared PCM
        // rate of 24 kHz. Our local cascade is natively 16 kHz and selects that
        // rate when `format` is omitted; declaring audio/pcm at 16 kHz causes
        // the server to reject the entire session.update.
        val input = JSONObject()
            .put("transcription", JSONObject().put("model", "whisper-1"))
            .put("turn_detection", JSONObject().put("type", "server_vad").put("interrupt_response", true))
        val output = JSONObject().put("voice", settings.voice).put("speed", 1)
        if (settings.sampleRate != 16000) {
            val format = JSONObject().put("type", "audio/pcm").put("rate", settings.sampleRate)
            input.put("format", format)
            output.put("format", JSONObject(format.toString()))
        }
        val session = JSONObject().put("type", "realtime").put("output_modalities", JSONArray().put("audio"))
            .put("instructions", settings.instructions).put("audio", JSONObject().put("input", input).put("output", output))
        if (settings.delegationEnabled) {
            val params = JSONObject().put("type", "object").put("properties", JSONObject().put("request", JSONObject().put("type", "string")))
                .put("required", JSONArray().put("request"))
            session.put("tools", JSONArray().put(JSONObject().put("type", "function").put("name", settings.delegationToolName).put("description", settings.delegationToolDescription).put("parameters", params))).put("tool_choice", "auto")
        } else session.put("tools", JSONArray()).put("tool_choice", "none")
        return JSONObject().put("type", "session.update").put("session", session)
    }

    private fun handle(j: JSONObject) {
        when (j.optString("type")) {
            "input_audio_buffer.speech_started" -> { player?.pause(); player?.flush(); player?.play(); event("user_speaking", null) }
            "input_audio_buffer.speech_stopped", "response.created" -> { responseActive = true; event("thinking", null) }
            "response.output_audio.delta", "response.audio.delta" -> {
                val bytes = Base64.decode(j.optString("delta"), Base64.DEFAULT)
                if (bytes.isNotEmpty()) player?.write(bytes, 0, bytes.size); event("speaking", null)
            }
            "response.function_call_arguments.done" -> handleFunction(j)
            "response.done" -> { responseActive = false; event("listening", null); flushReport() }
            "error" -> event("error", j.optJSONObject("error")?.optString("message") ?: "Realtime error")
        }
    }

    private fun handleFunction(j: JSONObject) {
        val callId = j.optString("call_id")
        val request = runCatching { JSONObject(j.optString("arguments", "{}")).optString("request") }.getOrDefault("")
        socket?.send(JSONObject().put("type", "conversation.item.create").put("item", JSONObject().put("type", "function_call_output").put("call_id", callId).put("output", "Handed to the brain. Tell the user briefly that you're on it.")).toString())
        responseActive = true; socket?.send(JSONObject().put("type", "response.create").toString()); event("delegating", request)
        node.delegate(request) { result ->
            val report = result.fold(
                { "The delegated task is complete. The agent answered: $it. Relay the result in one or two natural spoken sentences." },
                { "The delegated task failed: ${it.message}. Let me know briefly." },
            )
            if (settings.speakDelegatedResult || result.isFailure) synchronized(reports) { reports.add(report) }
            flushReport()
        }
    }

    private fun flushReport() {
        if (responseActive) return
        val text = synchronized(reports) { if (reports.isEmpty()) null else reports.removeFirst() } ?: return
        val content = JSONArray().put(JSONObject().put("type", "input_text").put("text", text))
        val item = JSONObject().put("type", "message").put("role", "user").put("content", content)
        socket?.send(JSONObject().put("type", "conversation.item.create").put("item", item).toString())
        responseActive = true; socket?.send(JSONObject().put("type", "response.create").toString())
    }
}
