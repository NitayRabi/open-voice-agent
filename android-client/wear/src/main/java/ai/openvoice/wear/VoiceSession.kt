package ai.openvoice.wear

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import android.os.SystemClock
import android.util.Base64
import androidx.core.content.ContextCompat
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean

data class DelegationStatus(
    val id: String,
    val request: String,
    val state: State,
    val detail: String? = null,
) {
    enum class State { RUNNING, SUCCEEDED, FAILED }
}

class VoiceSession(
    private val context: Context,
    private val node: NodeClient,
    private val settings: NodeSettings,
    private val selectedAgentId: () -> String?,
    private val event: (State, String?) -> Unit,
    private val delegationEvent: (DelegationStatus) -> Unit = {},
    private val onLevel: (Float) -> Unit = {},
) {
    enum class State { CONNECTING, LISTENING, THINKING, SPEAKING, ERROR, IDLE }

    private var socket: WebSocket? = null
    private var recorder: AudioRecord? = null
    private var player: AudioTrack? = null
    private var captureThread: Thread? = null
    private var echoCanceler: AcousticEchoCanceler? = null
    private var noiseSuppressor: NoiseSuppressor? = null
    private val active = AtomicBoolean(false)
    private val stopped = AtomicBoolean(false)
    @Volatile private var responseActive = false
    @Volatile private var microphoneMutedUntil = 0L
    private val reports = ArrayDeque<String>()

    @SuppressLint("MissingPermission")
    fun start() {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            event(State.ERROR, "Microphone permission is required")
            return
        }
        if (active.get()) return
        stopped.set(false)
        event(State.CONNECTING, null)
        try {
            val rate = settings.sampleRate
            val inputSize = maxOf(
                AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT),
                rate / 5,
            )
            recorder = AudioRecord(
                MediaRecorder.AudioSource.VOICE_COMMUNICATION,
                rate,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                inputSize,
            )
            check(recorder?.state == AudioRecord.STATE_INITIALIZED) { "Watch microphone could not be initialized" }
            recorder?.audioSessionId?.let { sessionId ->
                if (AcousticEchoCanceler.isAvailable()) {
                    echoCanceler = runCatching {
                        AcousticEchoCanceler.create(sessionId)?.also { it.enabled = true }
                    }.getOrNull()
                }
                if (NoiseSuppressor.isAvailable()) {
                    noiseSuppressor = runCatching {
                        NoiseSuppressor.create(sessionId)?.also { it.enabled = true }
                    }.getOrNull()
                }
            }

            val attributes = AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_ASSISTANT)
                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                .build()
            val format = AudioFormat.Builder()
                .setSampleRate(rate)
                .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .build()
            player = AudioTrack(
                attributes,
                format,
                maxOf(AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT), rate),
                AudioTrack.MODE_STREAM,
                AudioManager.AUDIO_SESSION_ID_GENERATE,
            ).also { it.play() }

            val request = node.realtimeRequest(settings.realtimeUrl)
            socket = node.http.newWebSocket(request, listener)
        } catch (error: Throwable) {
            releaseAudio()
            event(State.ERROR, error.message ?: "Could not start audio")
        }
    }

    fun stop(notify: Boolean = true) {
        stopped.set(true)
        active.set(false)
        captureThread?.interrupt()
        captureThread = null
        releaseAudio()
        socket?.close(1000, "stopped")
        socket = null
        if (notify) event(State.IDLE, null)
    }

    private val listener = object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (stopped.get()) {
                webSocket.close(1000, "activity stopped")
                return
            }
            active.set(true)
            webSocket.send(sessionUpdate().toString())
            event(State.LISTENING, null)
            startCapture()
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            runCatching { handle(JSONObject(text)) }
                .onFailure { event(State.ERROR, "Invalid response from node") }
        }

        override fun onFailure(webSocket: WebSocket, error: Throwable, response: Response?) {
            active.set(false)
            releaseAudio()
            if (!stopped.get()) event(State.ERROR, error.message ?: "Voice connection failed")
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            if (!stopped.get() && active.getAndSet(false)) {
                releaseAudio()
                event(State.IDLE, null)
            }
        }
    }

    private fun startCapture() {
        captureThread = Thread({
            val audioRecord = recorder ?: return@Thread
            val samples = ShortArray(settings.sampleRate * 40 / 1_000)
            val gate = VoiceNoiseGate()
            try {
                audioRecord.startRecording()
                while (active.get() && !Thread.currentThread().isInterrupted) {
                    val count = audioRecord.read(samples, 0, samples.size)
                    if (count <= 0) continue
                    var sum = 0.0
                    for (index in 0 until count) {
                        val value = samples[index].toDouble()
                        sum += value * value
                    }
                    onLevel((kotlin.math.sqrt(sum / count) / 32768.0).toFloat())
                    val suppressPlayback = responseActive || SystemClock.elapsedRealtime() < microphoneMutedUntil
                    val passAudio = !suppressPlayback && gate.shouldPass(samples, count)
                    val bytes = ByteArray(count * 2)
                    if (passAudio) {
                        for (index in 0 until count) {
                            val value = samples[index].toInt()
                            bytes[index * 2] = value.toByte()
                            bytes[index * 2 + 1] = (value shr 8).toByte()
                        }
                    }
                    socket?.send(
                        JSONObject()
                            .put("type", "input_audio_buffer.append")
                            .put("audio", Base64.encodeToString(bytes, Base64.NO_WRAP))
                            .toString(),
                    )
                }
            } catch (_: Throwable) {
                if (active.get()) event(State.ERROR, "Microphone stopped")
            }
        }, "ova-watch-microphone").also { it.start() }
    }

    private fun handle(message: JSONObject) {
        when (message.optString("type")) {
            "input_audio_buffer.speech_started" -> {
                event(State.LISTENING, "Listening…")
            }
            "input_audio_buffer.speech_stopped", "response.created" -> {
                responseActive = true
                event(State.THINKING, null)
            }
            "response.output_audio.delta", "response.audio.delta" -> {
                val bytes = Base64.decode(message.optString("delta"), Base64.DEFAULT)
                if (bytes.isNotEmpty()) {
                    val now = SystemClock.elapsedRealtime()
                    val queuedAfter = maxOf(now, microphoneMutedUntil - PLAYBACK_TAIL_MILLIS)
                    val audioMillis = bytes.size * 1_000L / (settings.sampleRate * 2L)
                    microphoneMutedUntil = queuedAfter + audioMillis + PLAYBACK_TAIL_MILLIS
                    player?.write(bytes, 0, bytes.size)
                }
                event(State.SPEAKING, null)
            }
            "response.function_call_arguments.done" -> handleFunction(message)
            "response.done" -> {
                responseActive = false
                event(State.LISTENING, null)
                flushReport()
            }
            "error" -> event(State.ERROR, message.optJSONObject("error")?.optString("message") ?: "Realtime error")
        }
    }

    private fun sessionUpdate(): JSONObject {
        // The local 16 kHz server uses its native format when this field is
        // absent. OpenAI-compatible endpoints only accept an explicit PCM rate
        // of 24 kHz, so mirroring the browser client here is important.
        val input = JSONObject()
            .put("transcription", JSONObject().put("model", "whisper-1"))
            // Barge-in is disabled on the watch: its speaker and microphone are
            // centimetres apart, and leaked TTS otherwise cancels itself.
            .put("turn_detection", JSONObject().put("type", "server_vad").put("interrupt_response", false))
        val output = JSONObject()
            .put("voice", settings.voice)
            .put("speed", 1)
        if (settings.sampleRate != 16_000) {
            val format = JSONObject().put("type", "audio/pcm").put("rate", settings.sampleRate)
            input.put("format", format)
            output.put("format", JSONObject(format.toString()))
        }
        val session = JSONObject()
            .put("type", "realtime")
            .put("output_modalities", JSONArray().put("audio"))
            .put("instructions", watchInstructions())
            .put("audio", JSONObject().put("input", input).put("output", output))
        if (settings.delegationEnabled) {
            val parameters = JSONObject()
                .put("type", "object")
                .put("properties", JSONObject().put("request", JSONObject().put("type", "string")))
                .put("required", JSONArray().put("request"))
            session.put(
                "tools",
                JSONArray().put(
                    JSONObject()
                        .put("type", "function")
                        .put("name", settings.delegationToolName)
                        .put("description", settings.delegationToolDescription)
                        .put("parameters", parameters),
                ),
            ).put("tool_choice", "auto")
        } else {
            session.put("tools", JSONArray()).put("tool_choice", "none")
        }
        return JSONObject().put("type", "session.update").put("session", session)
    }

    private fun handleFunction(message: JSONObject) {
        val callId = message.optString("call_id")
        val request = runCatching {
            JSONObject(message.optString("arguments", "{}")).optString("request")
        }.getOrDefault("")
        val taskId = callId.ifBlank { "task-${SystemClock.elapsedRealtime()}" }
        val taskRequest = request.ifBlank { "Delegated action" }
        delegationEvent(DelegationStatus(taskId, taskRequest, DelegationStatus.State.RUNNING))
        val output = JSONObject()
            .put("type", "conversation.item.create")
            .put(
                "item",
                JSONObject()
                    .put("type", "function_call_output")
                    .put("call_id", callId)
                    .put("output", "Handed to the brain. Tell the user briefly that you're on it."),
            )
        socket?.send(output.toString())
        responseActive = true
        socket?.send(JSONObject().put("type", "response.create").toString())
        event(State.THINKING, "Handing off…")
        val agentId = selectedAgentId()
        node.delegate(request, agentId) { result ->
            delegationEvent(
                result.fold(
                    onSuccess = { DelegationStatus(taskId, taskRequest, DelegationStatus.State.SUCCEEDED, it) },
                    onFailure = { DelegationStatus(taskId, taskRequest, DelegationStatus.State.FAILED, it.message) },
                ),
            )
            if (stopped.get()) return@delegate
            val report = result.fold(
                onSuccess = { "The delegated task is complete. The agent answered: $it. Relay the result in one or two natural spoken sentences." },
                onFailure = { "The delegated task failed: ${it.message}. Let me know briefly." },
            )
            val speak = settings.agents.find { it.id == agentId }?.speakDelegatedResult ?: settings.speakDelegatedResult
            if (speak || result.isFailure) synchronized(reports) { reports.add(report) }
            flushReport()
        }
    }

    private fun flushReport() {
        if (responseActive || stopped.get()) return
        val text = synchronized(reports) { if (reports.isEmpty()) null else reports.removeFirst() } ?: return
        val content = JSONArray().put(JSONObject().put("type", "input_text").put("text", text))
        val item = JSONObject().put("type", "message").put("role", "user").put("content", content)
        socket?.send(JSONObject().put("type", "conversation.item.create").put("item", item).toString())
        responseActive = true
        socket?.send(JSONObject().put("type", "response.create").toString())
    }

    private fun watchInstructions(): String = buildString {
        append(settings.instructions.trim())
        if (settings.delegationEnabled) {
            append("\n\nYou have real action capability through the ")
            append(settings.delegationToolName)
            append(" tool and OpenClaw. For smart-home controls, reminders, messages, searches, or any request to do something, you MUST call that tool before replying. Never claim you cannot access or control something before trying the tool. Use the user's complete request as its request argument.")
        }
    }

    @Synchronized
    private fun releaseAudio() {
        echoCanceler?.release()
        echoCanceler = null
        noiseSuppressor?.release()
        noiseSuppressor = null
        recorder?.runCatching { stop() }
        recorder?.release()
        recorder = null
        player?.runCatching { pause() }
        player?.flush()
        player?.release()
        player = null
    }

    private companion object {
        const val PLAYBACK_TAIL_MILLIS = 300L
    }
}
