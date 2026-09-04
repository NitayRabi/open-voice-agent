// Voice pipeline: mic -> (Hugging Face speech-to-speech realtime WS) -> speaker,
// plus asynchronous delegation of tool calls to a background agent.
//
// Transport is the core OpenAI Realtime GA event set over WebSocket, which is
// exactly what `speech-to-speech serve` exposes at /v1/realtime. Server-side VAD
// drives the turns; we only stream PCM16 up and play PCM16 down.

const b64FromBuf = (buf) => {
  const bytes = new Uint8Array(buf);
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(bin);
};
const bytesFromB64 = (b64) => {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
};

export class VoicePipeline extends EventTarget {
  constructor(opts = {}) {
    super();
    this.opts = opts; // { delegate?, workletBase? }
    this.settings = null;
    this.state = "idle";
    this._ws = null;
    this._ctx = null;
    this._stream = null;
    this._capture = null;
    this._playback = null;
    this._micAnalyser = null;
    this._closing = false;
    this._muted = false;
    this._reconnects = 0;
    this._pendingCalls = new Map(); // call_id -> { name, args }
    this._asstText = "";
  }

  configure(settings) {
    this.settings = settings;
  }

  get running() {
    return this.state !== "idle" && this.state !== "error";
  }

  _emit(name, detail) {
    this.dispatchEvent(new CustomEvent(name, { detail }));
  }

  _setState(s) {
    if (this.state === s) return;
    this.state = s;
    this._emit("state", { state: s });
  }

  _log(msg) {
    this._emit("log", { msg });
  }

  async toggle() {
    if (this.running) this.stop();
    else await this.start();
  }

  setMuted(m) {
    this._muted = m;
    this._capture?.port.postMessage({ kind: "enable", value: !m });
    this._emit("muted", { muted: m });
  }

  async start() {
    if (this.running) return;
    if (!this.settings) throw new Error("pipeline not configured");
    this._closing = false;
    this._reconnects = 0;
    this._setState("connecting");
    try {
      await this._setupAudio();
      await this._connectRealtime();
    } catch (e) {
      this._log(`start failed: ${e.message || e}`);
      this._setState("error");
      this.stop();
      throw e;
    }
  }

  stop() {
    this._closing = true;
    try {
      this._ws?.close();
    } catch {}
    this._ws = null;
    if (this._stream) this._stream.getTracks().forEach((t) => t.stop());
    this._stream = null;
    this._capture?.disconnect();
    this._playback?.disconnect();
    this._capture = this._playback = this._micAnalyser = this._outAnalyser = null;
    cancelAnimationFrame(this._levelRaf);
    if (this._ctx && this._ctx.state !== "closed") this._ctx.close();
    this._ctx = null;
    this._setState("idle");
  }

  // ── audio graph ─────────────────────────────────────────────────────────

  async _setupAudio() {
    const base = this.opts.workletBase || new URL("../worklets/", import.meta.url);
    const rate = this.settings.sample_rate || 16000;
    const audio = {
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    };
    if (this.settings.mic_device_id) audio.deviceId = { exact: this.settings.mic_device_id };
    this._stream = await navigator.mediaDevices.getUserMedia({ audio });

    this._ctx = new AudioContext({ latencyHint: "interactive" });
    if (this._ctx.state === "suspended") await this._ctx.resume().catch(() => {});
    const micSrc = this._ctx.createMediaStreamSource(this._stream);
    this._micAnalyser = this._ctx.createAnalyser();
    this._micAnalyser.fftSize = 512;
    micSrc.connect(this._micAnalyser);

    await this._ctx.audioWorklet.addModule(new URL("mic-capture.js", base));
    await this._ctx.audioWorklet.addModule(new URL("audio-playback.js", base));

    this._capture = new AudioWorkletNode(this._ctx, "mic-capture", {
      numberOfInputs: 1,
      numberOfOutputs: 0,
      processorOptions: { chunkMs: 40, targetRate: rate },
    });
    const gateOn = this.settings.noise_gate_db > -99;
    this._capture.port.postMessage({
      kind: "gate",
      enabled: gateOn,
      thresholdDb: this.settings.noise_gate_db,
    });
    this._capture.port.onmessage = (e) => {
      const d = e.data;
      if (d instanceof ArrayBuffer) this._onMicChunk(d);
      // UI metering is emitted by the analyser pump below, together with the
      // actual waveform samples used by the orb boundary.
    };
    micSrc.connect(this._capture);

    this._playback = new AudioWorkletNode(this._ctx, "audio-playback", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    this._playback.port.postMessage({ kind: "config", inputRate: rate });
    const outAnalyser = this._ctx.createAnalyser();
    outAnalyser.fftSize = 512;
    this._playback.connect(outAnalyser);
    outAnalyser.connect(this._ctx.destination);
    this._outAnalyser = outAnalyser;
    this._startLevelPump();

    if (this._muted) this._capture.port.postMessage({ kind: "enable", value: false });
  }

  _startLevelPump() {
    const outBuf = new Uint8Array(this._outAnalyser.frequencyBinCount);
    const inBuf = new Uint8Array(this._micAnalyser.frequencyBinCount);
    const measure = (analyser, buf) => {
      analyser.getByteTimeDomainData(buf);
      let sum = 0;
      const waveform = new Float32Array(128);
      for (let i = 0; i < buf.length; i++) {
        const sample = (buf[i] - 128) / 128;
        sum += sample * sample;
      }
      for (let i = 0; i < 128; i++) waveform[i] = (buf[Math.floor(i * buf.length / 128)] - 128) / 128;
      return { rms: Math.sqrt(sum / buf.length), waveform };
    };
    const tick = () => {
      if (!this._outAnalyser) return;
      this._emit("output-level", measure(this._outAnalyser, outBuf));
      if (this._micAnalyser) this._emit("input-level", measure(this._micAnalyser, inBuf));
      this._levelRaf = requestAnimationFrame(tick);
    };
    tick();
  }

  _onMicChunk(arrayBuffer) {
    if (this._muted) return;
    if (this._ws?.readyState === WebSocket.OPEN) {
      this._ws.send(
        JSON.stringify({ type: "input_audio_buffer.append", audio: b64FromBuf(arrayBuffer) }),
      );
    }
  }

  _playPcm16(bytes) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const samples = new Float32Array(bytes.byteLength / 2);
    for (let i = 0; i < samples.length; i++) {
      const s = view.getInt16(i * 2, true);
      samples[i] = s < 0 ? s / 0x8000 : s / 0x7fff;
    }
    this._playback?.port.postMessage({ kind: "audio", samples }, [samples.buffer]);
  }

  _clearPlayback() {
    this._playback?.port.postMessage({ kind: "clear" });
  }

  // ── Hugging Face realtime (OpenAI Realtime GA event set) ─────────────────

  _toolsForSession() {
    if (!this.settings.delegation_enabled) return [];
    return [
      {
        type: "function",
        name: this.settings.delegation_tool_name || "delegate_task",
        description:
          this.settings.delegation_tool_description ||
          "Hand a task to the more capable brain model when the user wants something actually done.",
        parameters: {
          type: "object",
          properties: {
            request: {
              type: "string",
              description: "The task or question, in one clear sentence.",
            },
          },
          required: ["request"],
        },
      },
    ];
  }

  _sessionUpdate() {
    const rate = this.settings.sample_rate || 16000;
    return {
      type: "session.update",
      session: {
        type: "realtime",
        output_modalities: ["audio"],
        instructions: this.settings.instructions || "",
        audio: {
          input: {
            format: { type: "audio/pcm", rate },
            transcription: { model: "whisper-1" },
            turn_detection: { type: "server_vad", interrupt_response: true },
          },
          output: {
            format: { type: "audio/pcm", rate },
            voice: this.settings.voice || "Aiden",
            speed: 1,
          },
        },
        tools: this._toolsForSession(),
        tool_choice: this.settings.delegation_enabled ? "auto" : "none",
      },
    };
  }

  async _connectRealtime() {
    const url = this.settings.server_url;
    this._log(`connecting to ${url}`);
    const ws = new WebSocket(url, [
      "realtime",
      "openai-insecure-api-key.open-voice-agent",
      "openai-beta.realtime-v1",
    ]);
    this._ws = ws;

    ws.onopen = () => {
      this._reconnects = 0;
      ws.send(JSON.stringify(this._sessionUpdate()));
      this._setState("listening");
      this._log("connected");
    };
    ws.onclose = () => {
      if (this._closing) return;
      this._log("connection closed");
      this._maybeReconnect();
    };
    ws.onerror = () => {
      if (!this._closing) this._log("websocket error");
    };
    ws.onmessage = (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      this._onServerEvent(msg);
    };
  }

  _maybeReconnect() {
    if (this._closing || this._reconnects >= 5) {
      this._setState("error");
      return;
    }
    this._reconnects++;
    this._setState("connecting");
    setTimeout(() => {
      if (!this._closing) this._connectRealtime().catch(() => this._setState("error"));
    }, 1500 * this._reconnects);
  }

  _onServerEvent(msg) {
    const t = msg.type || "";
    switch (t) {
      case "session.created":
      case "session.updated":
        break;
      case "error":
        this._log(`server error: ${msg.error?.message || JSON.stringify(msg.error || msg)}`);
        break;
      case "input_audio_buffer.speech_started":
        this._clearPlayback();
        this._setState("user_speaking");
        break;
      case "input_audio_buffer.speech_stopped":
        this._setState("thinking");
        break;
      case "conversation.item.input_audio_transcription.completed":
        if (msg.transcript) this._emit("transcript", { role: "user", text: msg.transcript.trim() });
        break;
      case "response.created":
        this._asstText = "";
        if (this.state !== "delegating") this._setState("thinking");
        break;
      case "response.output_audio.delta":
      case "response.audio.delta":
        if (msg.delta) {
          this._playPcm16(bytesFromB64(msg.delta));
          this._setState("speaking");
        }
        break;
      case "response.output_audio_transcript.delta":
      case "response.audio_transcript.delta":
        this._asstText += msg.delta || "";
        break;
      case "response.output_audio_transcript.done":
      case "response.audio_transcript.done":
        if (msg.transcript || this._asstText) {
          this._emit("transcript", { role: "assistant", text: (msg.transcript || this._asstText).trim() });
        }
        break;
      case "response.function_call_arguments.delta":
        break;
      case "response.function_call_arguments.done":
        this._handleFunctionCall(msg);
        break;
      case "response.done": {
        const status = msg.response?.status;
        if (status === "failed") {
          this._log(`response failed: ${JSON.stringify(msg.response?.status_details || {})}`);
        }
        if (this.state !== "delegating") this._setState("listening");
        break;
      }
      default:
        break;
    }
  }

  _sendWs(obj) {
    if (this._ws?.readyState === WebSocket.OPEN) this._ws.send(JSON.stringify(obj));
  }

  async _handleFunctionCall(msg) {
    const callId = msg.call_id || msg.callId || "";
    let request = "";
    try {
      const args = JSON.parse(msg.arguments || "{}");
      request = args.request || args.query || args.task || msg.arguments || "";
    } catch {
      request = msg.arguments || "";
    }
    this._emit("transcript", { role: "tool", text: `delegate_task → ${request}` });

    // 1. Acknowledge immediately so the voice model tells the user it's on it
    //    while the brain works.
    this._sendWs({
      type: "conversation.item.create",
      item: {
        type: "function_call_output",
        call_id: callId,
        output: "Handed to the brain. Tell the user briefly that you're on it.",
      },
    });
    this._sendWs({ type: "response.create" });
    this._setState("delegating");

    // 2. Run the delegation off the critical path.
    if (!this.opts.delegate) {
      this._log("no delegate handler wired");
      return;
    }
    try {
      const answer = await this.opts.delegate(request);
      this._emit("transcript", { role: "tool", text: `brain ✓ ${answer}` });
      if (this.settings.delegation_speak_result && answer) {
        this._sendWs({
          type: "conversation.item.create",
          item: {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: `The brain answered: ${answer}. Relay this to me in one or two sentences.` }],
          },
        });
        this._sendWs({ type: "response.create" });
      }
    } catch (e) {
      const err = e.message || String(e);
      this._emit("transcript", { role: "tool", text: `brain ✗ ${err}` });
      this._sendWs({
        type: "conversation.item.create",
        item: {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: `The brain couldn't do that: ${err}. Let me know briefly.` }],
        },
      });
      this._sendWs({ type: "response.create" });
    } finally {
      if (this.state === "delegating") this._setState("listening");
    }
  }

  async listInputDevices() {
    try {
      const devices = await navigator.mediaDevices.enumerateDevices();
      return devices.filter((d) => d.kind === "audioinput").map((d) => ({ id: d.deviceId, label: d.label }));
    } catch {
      return [];
    }
  }
}
