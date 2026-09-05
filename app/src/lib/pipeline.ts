// Voice pipeline: mic -> (Hugging Face speech-to-speech realtime WS) -> speaker,
// plus asynchronous delegation of tool calls to a background agent.
//
// Transport is the core OpenAI Realtime GA event set over WebSocket, which is
// exactly what `speech-to-speech serve` exposes at /v1/realtime. Server-side VAD
// drives the turns; we only stream PCM16 up and play PCM16 down.

import { errorText } from "./errors.js";
import type {
  AudioInputDevice,
  LevelDetail,
  PipelineState,
  Settings,
  TaskDetail,
  TranscriptDetail,
} from "./types.js";

const b64FromBuf = (buf: ArrayBuffer): string => {
  const bytes = new Uint8Array(buf);
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(bin);
};
const bytesFromB64 = (b64: string): Uint8Array => {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
};

/** The subset of Realtime server events the pipeline reacts to. */
interface ServerEvent {
  type?: string;
  delta?: string;
  transcript?: string;
  arguments?: string;
  call_id?: string;
  callId?: string;
  error?: { message?: string };
  response?: { status?: string; status_details?: unknown };
}

/** Messages posted from the mic-capture / audio-playback worklets. */
interface CaptureMessage {
  kind: "level" | "capture-config";
  rms?: number;
  inputRate?: number;
  outputRate?: number;
}

export interface PipelineOptions {
  /** Runs a delegated task and resolves with the spoken-back answer. */
  delegate?: (request: string) => Promise<string>;
  /** Directory holding the audio worklets; defaults to `../worklets/`. */
  workletBase?: string | URL;
}

/** `detail` payload of each event the pipeline dispatches. */
export interface PipelineEventMap {
  state: CustomEvent<{ state: PipelineState }>;
  log: CustomEvent<{ msg: string }>;
  muted: CustomEvent<{ muted: boolean }>;
  transcript: CustomEvent<TranscriptDetail>;
  task: CustomEvent<TaskDetail>;
  "input-level": CustomEvent<LevelDetail>;
  "output-level": CustomEvent<LevelDetail>;
}

export class VoicePipeline extends EventTarget {
  readonly opts: PipelineOptions;
  settings: Settings | null = null;
  state: PipelineState = "idle";

  private _ws: WebSocket | null = null;
  private _ctx: AudioContext | null = null;
  private _stream: MediaStream | null = null;
  private _capture: AudioWorkletNode | null = null;
  private _playback: AudioWorkletNode | null = null;
  private _micAnalyser: AnalyserNode | null = null;
  private _outAnalyser: AnalyserNode | null = null;
  private _levelRaf = 0;
  private _closing = false;
  private _muted = false;
  private _reconnects = 0;
  private _asstText = "";
  private _responseActive = false;
  private _queuedAgentReports: string[] = [];
  private _nextTaskId = 1;

  constructor(opts: PipelineOptions = {}) {
    super();
    this.opts = opts;
  }

  configure(settings: Settings): void {
    this.settings = settings;
  }

  get running(): boolean {
    return this.state !== "idle" && this.state !== "error";
  }

  get muted(): boolean {
    return this._muted;
  }

  // Typed overloads so listeners see the right `detail` for each event name.
  override addEventListener<K extends keyof PipelineEventMap>(
    type: K,
    listener: (event: PipelineEventMap[K]) => void,
    options?: boolean | AddEventListenerOptions,
  ): void;
  override addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  override addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void {
    super.addEventListener(type, listener, options);
  }

  private _emit<K extends keyof PipelineEventMap>(
    name: K,
    detail: PipelineEventMap[K]["detail"],
  ): void {
    this.dispatchEvent(new CustomEvent(name, { detail }));
  }

  private _setState(s: PipelineState): void {
    if (this.state === s) return;
    this.state = s;
    this._emit("state", { state: s });
    if (s === "listening") this._flushAgentReport();
  }

  private _log(msg: string): void {
    this._emit("log", { msg });
  }

  /** Settings, or a hard failure — every audio path needs them. */
  private get _cfg(): Settings {
    if (!this.settings) throw new Error("pipeline not configured");
    return this.settings;
  }

  async toggle(): Promise<void> {
    if (this.running) this.stop();
    else await this.start();
  }

  setMuted(m: boolean): void {
    this._muted = m;
    this._capture?.port.postMessage({ kind: "enable", value: !m });
    this._emit("muted", { muted: m });
  }

  async start(): Promise<void> {
    if (this.running) return;
    if (!this.settings) throw new Error("pipeline not configured");
    this._closing = false;
    this._reconnects = 0;
    this._setState("connecting");
    try {
      await this._setupAudio();
      await this._connectRealtime();
    } catch (e) {
      this._log(`start failed: ${errorText(e)}`);
      this._setState("error");
      this.stop();
      throw e;
    }
  }

  stop(): void {
    this._closing = true;
    try {
      this._ws?.close();
    } catch {}
    this._ws = null;
    if (this._stream) this._stream.getTracks().forEach((t) => t.stop());
    this._stream = null;
    this._capture?.disconnect();
    this._playback?.disconnect();
    this._capture = null;
    this._playback = null;
    this._micAnalyser = null;
    this._outAnalyser = null;
    cancelAnimationFrame(this._levelRaf);
    if (this._ctx && this._ctx.state !== "closed") void this._ctx.close();
    this._ctx = null;
    this._setState("idle");
  }

  // ── audio graph ─────────────────────────────────────────────────────────

  private async _setupAudio(): Promise<void> {
    const cfg = this._cfg;
    const base = this.opts.workletBase || new URL("../worklets/", import.meta.url);
    const rate = cfg.sample_rate || 16000;
    const audio: MediaTrackConstraints = {
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    };
    if (cfg.mic_device_id) audio.deviceId = { exact: cfg.mic_device_id };
    this._stream = await navigator.mediaDevices.getUserMedia({ audio });

    const ctx = new AudioContext({ latencyHint: "interactive" });
    this._ctx = ctx;
    if (ctx.state === "suspended") await ctx.resume().catch(() => {});
    const micSrc = ctx.createMediaStreamSource(this._stream);
    const micAnalyser = ctx.createAnalyser();
    micAnalyser.fftSize = 512;
    micSrc.connect(micAnalyser);
    this._micAnalyser = micAnalyser;

    await ctx.audioWorklet.addModule(new URL("mic-capture.js", base));
    await ctx.audioWorklet.addModule(new URL("audio-playback.js", base));

    const capture = new AudioWorkletNode(ctx, "mic-capture", {
      numberOfInputs: 1,
      numberOfOutputs: 0,
      processorOptions: { chunkMs: 40, targetRate: rate },
    });
    this._capture = capture;
    const gateOn = cfg.noise_gate_db > -99;
    capture.port.postMessage({
      kind: "gate",
      enabled: gateOn,
      thresholdDb: cfg.noise_gate_db,
    });
    capture.port.onmessage = (e: MessageEvent<ArrayBuffer | CaptureMessage>) => {
      const d = e.data;
      if (d instanceof ArrayBuffer) this._onMicChunk(d);
      // UI metering is emitted by the analyser pump below, together with the
      // actual waveform samples used by the orb boundary.
    };
    micSrc.connect(capture);

    const playback = new AudioWorkletNode(ctx, "audio-playback", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    this._playback = playback;
    playback.port.postMessage({ kind: "config", inputRate: rate });
    const outAnalyser = ctx.createAnalyser();
    outAnalyser.fftSize = 512;
    playback.connect(outAnalyser);
    outAnalyser.connect(ctx.destination);
    this._outAnalyser = outAnalyser;
    this._startLevelPump();

    if (this._muted) capture.port.postMessage({ kind: "enable", value: false });
  }

  private _startLevelPump(): void {
    const outAnalyser = this._outAnalyser;
    const micAnalyser = this._micAnalyser;
    if (!outAnalyser || !micAnalyser) return;
    const outBuf = new Uint8Array(outAnalyser.frequencyBinCount);
    const inBuf = new Uint8Array(micAnalyser.frequencyBinCount);
    const measure = (analyser: AnalyserNode, buf: Uint8Array<ArrayBuffer>): LevelDetail => {
      analyser.getByteTimeDomainData(buf);
      let sum = 0;
      const waveform = new Float32Array(128);
      for (let i = 0; i < buf.length; i++) {
        const sample = (buf[i]! - 128) / 128;
        sum += sample * sample;
      }
      for (let i = 0; i < 128; i++) {
        waveform[i] = (buf[Math.floor((i * buf.length) / 128)]! - 128) / 128;
      }
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

  private _onMicChunk(arrayBuffer: ArrayBuffer): void {
    if (this._muted) return;
    if (this._ws?.readyState === WebSocket.OPEN) {
      this._ws.send(
        JSON.stringify({ type: "input_audio_buffer.append", audio: b64FromBuf(arrayBuffer) }),
      );
    }
  }

  private _playPcm16(bytes: Uint8Array): void {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const samples = new Float32Array(bytes.byteLength / 2);
    for (let i = 0; i < samples.length; i++) {
      const s = view.getInt16(i * 2, true);
      samples[i] = s < 0 ? s / 0x8000 : s / 0x7fff;
    }
    this._playback?.port.postMessage({ kind: "audio", samples }, [samples.buffer]);
  }

  private _clearPlayback(): void {
    this._playback?.port.postMessage({ kind: "clear" });
  }

  // ── Hugging Face realtime (OpenAI Realtime GA event set) ─────────────────

  private _toolsForSession(): unknown[] {
    const cfg = this._cfg;
    if (!cfg.delegation_enabled) return [];
    return [
      {
        type: "function",
        name: cfg.delegation_tool_name || "delegate_task",
        description:
          cfg.delegation_tool_description ||
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

  private _sessionUpdate(): unknown {
    const cfg = this._cfg;
    const rate = cfg.sample_rate || 16000;
    // OpenAI's Realtime schema only permits an explicit PCM rate of 24 kHz.
    // The local speech-to-speech server uses 16 kHz natively when `format` is
    // omitted, so spelling out 16 kHz makes Pydantic reject the *entire*
    // session.update (including instructions and tools).
    const pcmFormat = rate === 16000 ? {} : { format: { type: "audio/pcm", rate } };
    return {
      type: "session.update",
      session: {
        type: "realtime",
        output_modalities: ["audio"],
        instructions: cfg.instructions || "",
        audio: {
          input: {
            ...pcmFormat,
            transcription: { model: "whisper-1" },
            turn_detection: { type: "server_vad", interrupt_response: true },
          },
          output: {
            ...pcmFormat,
            voice: cfg.voice || "Aiden",
            speed: 1,
          },
        },
        tools: this._toolsForSession(),
        tool_choice: cfg.delegation_enabled ? "auto" : "none",
      },
    };
  }

  private async _connectRealtime(): Promise<void> {
    const url = this._cfg.server_url;
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
    ws.onmessage = (ev: MessageEvent<string>) => {
      let msg: ServerEvent;
      try {
        msg = JSON.parse(ev.data) as ServerEvent;
      } catch {
        return;
      }
      this._onServerEvent(msg);
    };
  }

  private _maybeReconnect(): void {
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

  private _onServerEvent(msg: ServerEvent): void {
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
        this._responseActive = true;
        this._asstText = "";
        this._setState("thinking");
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
          this._emit("transcript", {
            role: "assistant",
            text: (msg.transcript || this._asstText).trim(),
          });
        }
        break;
      case "response.function_call_arguments.delta":
        break;
      case "response.function_call_arguments.done":
        void this._handleFunctionCall(msg);
        break;
      case "response.done": {
        this._responseActive = false;
        const status = msg.response?.status;
        if (status === "failed") {
          this._log(`response failed: ${JSON.stringify(msg.response?.status_details || {})}`);
        }
        this._setState("listening");
        this._flushAgentReport();
        break;
      }
      default:
        break;
    }
  }

  private _sendWs(obj: unknown): void {
    if (this._ws?.readyState === WebSocket.OPEN) this._ws.send(JSON.stringify(obj));
  }

  private async _handleFunctionCall(msg: ServerEvent): Promise<void> {
    const callId = msg.call_id || msg.callId || "";
    const taskId = callId || `task-${this._nextTaskId++}`;
    let request = "";
    try {
      const args = JSON.parse(msg.arguments || "{}") as {
        request?: string;
        query?: string;
        task?: string;
      };
      request = args.request || args.query || args.task || msg.arguments || "";
    } catch {
      request = msg.arguments || "";
    }
    this._emit("transcript", { role: "tool", text: `delegate_task → ${request}` });
    this._emit("task", { id: taskId, request, status: "running" });

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
    // Reserve the response slot immediately so a fast delegated result cannot
    // start a second voice response before the acknowledgement has finished.
    this._responseActive = true;
    this._sendWs({ type: "response.create" });

    // 2. Run the delegation off the critical path.
    if (!this.opts.delegate) {
      this._log("no delegate handler wired");
      this._emit("task", {
        id: taskId,
        request,
        status: "failed",
        error: "No delegate handler is configured",
      });
      return;
    }
    try {
      const answer = await this.opts.delegate(request);
      this._emit("transcript", { role: "tool", text: `brain ✓ ${answer}` });
      this._emit("task", { id: taskId, request, status: "completed", result: answer });
      if (this._cfg.delegation_speak_result && answer) {
        this._queueAgentReport(
          `The delegated task is complete. The agent answered: ${answer}. Relay the result to me in one or two natural spoken sentences.`,
        );
      }
    } catch (e) {
      const err = errorText(e);
      this._emit("transcript", { role: "tool", text: `brain ✗ ${err}` });
      this._emit("task", { id: taskId, request, status: "failed", error: err });
      this._queueAgentReport(`The delegated task failed: ${err}. Let me know briefly.`);
    }
  }

  private _queueAgentReport(text: string): void {
    this._queuedAgentReports.push(text);
    this._flushAgentReport();
  }

  private _flushAgentReport(): void {
    if (this._responseActive || this.state !== "listening" || !this._queuedAgentReports.length) return;
    const text = this._queuedAgentReports.shift();
    this._sendWs({
      type: "conversation.item.create",
      item: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text }],
      },
    });
    this._responseActive = true;
    this._sendWs({ type: "response.create" });
  }

  async listInputDevices(): Promise<AudioInputDevice[]> {
    try {
      const devices = await navigator.mediaDevices.enumerateDevices();
      return devices
        .filter((d) => d.kind === "audioinput")
        .map((d) => ({ id: d.deviceId, label: d.label }));
    } catch {
      return [];
    }
  }
}
