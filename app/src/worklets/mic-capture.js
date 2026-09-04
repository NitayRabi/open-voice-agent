// Mic capture worklet: resample the AudioContext rate (typically 48 kHz) down to
// the pipeline rate (16 kHz for the Hugging Face speech-to-speech cascade), pack
// as little-endian Int16 PCM, and post fixed-size chunks to the main thread.
//
// Adapted from huggingface/speech-to-speech (demo/worklets/mic-capture.js,
// Apache-2.0): the target rate is configurable and defaults to 16 kHz.

const DEFAULT_TARGET_RATE = 16000;
const DEFAULT_CHUNK_MS = 40;
const GATE_ATTACK_MS = 5;
const GATE_HOLD_MS = 250;
const GATE_RELEASE_MS = 80;

class MicCaptureProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const chunkMs = options?.processorOptions?.chunkMs ?? DEFAULT_CHUNK_MS;
    const requestedRate = Number(options?.processorOptions?.targetRate);
    this._targetRate =
      Number.isFinite(requestedRate) && requestedRate > 0 ? requestedRate : DEFAULT_TARGET_RATE;
    this._inputRate = sampleRate;
    this._ratio = this._inputRate / this._targetRate;
    this._chunkSamples = Math.round((this._targetRate * chunkMs) / 1000);
    this._scratch = new Float32Array(0);
    this._decimated = new Float32Array(this._chunkSamples);
    this._enabled = true;

    this._gateEnabled = false;
    this._thresholdLin = 0;
    this._gateGain = 1;
    this._holdRemaining = 0;
    this._attackCoef = Math.exp(-1 / ((GATE_ATTACK_MS / 1000) * this._targetRate));
    this._releaseCoef = Math.exp(-1 / ((GATE_RELEASE_MS / 1000) * this._targetRate));
    this._holdSamples = Math.round((GATE_HOLD_MS / 1000) * this._targetRate);

    this.port.onmessage = (e) => {
      const data = e.data;
      if (data?.kind === "enable") this._enabled = !!data.value;
      else if (data?.kind === "probe") {
        this.port.postMessage({
          kind: "capture-config",
          inputRate: this._inputRate,
          outputRate: this._targetRate,
        });
      } else if (data?.kind === "gate") {
        this._gateEnabled = !!data.enabled;
        this._thresholdLin = data.enabled ? Math.pow(10, data.thresholdDb / 20) : 0;
      }
    };
  }

  _ingest(incoming) {
    if (incoming.length === 0) return;
    const next = new Float32Array(this._scratch.length + incoming.length);
    next.set(this._scratch, 0);
    next.set(incoming, this._scratch.length);
    this._scratch = next;
    this._maybeEmit();
  }

  _maybeEmit() {
    const r = this._ratio;
    const n = this._chunkSamples;
    const needIn = Math.ceil(n * r);
    const dec = this._decimated;
    while (this._scratch.length >= needIn) {
      let sumSq = 0;
      if (Math.abs(r - Math.round(r)) < 1e-6 && r >= 2) {
        const k = Math.round(r);
        for (let i = 0; i < n; i++) {
          let acc = 0;
          for (let j = 0; j < k; j++) acc += this._scratch[i * k + j];
          const s = acc / k;
          dec[i] = s;
          sumSq += s * s;
        }
      } else {
        for (let i = 0; i < n; i++) {
          const srcPos = i * r;
          const idx = Math.floor(srcPos);
          const frac = srcPos - idx;
          const a = this._scratch[idx];
          const b = this._scratch[idx + 1] ?? a;
          const s = a + (b - a) * frac;
          dec[i] = s;
          sumSq += s * s;
        }
      }
      const rms = Math.sqrt(sumSq / n);

      let target = 1;
      if (this._gateEnabled) {
        if (rms >= this._thresholdLin) this._holdRemaining = this._holdSamples;
        else if (this._holdRemaining > 0) this._holdRemaining -= n;
        else target = 0;
      }

      const out = new Int16Array(n);
      let gain = this._gateGain;
      for (let i = 0; i < n; i++) {
        const coef = target > gain ? this._attackCoef : this._releaseCoef;
        gain = target + (gain - target) * coef;
        const s = dec[i] * gain;
        const clamped = s < -1 ? -1 : s > 1 ? 1 : s;
        out[i] = clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff;
      }
      this._gateGain = gain;

      const consumed = Math.floor(n * r);
      this._scratch = this._scratch.slice(consumed);

      this.port.postMessage({ kind: "level", rms });
      if (this._enabled) this.port.postMessage(out.buffer, [out.buffer]);
    }
  }

  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0 || !input[0]) return true;
    const mono = input[0];
    if (mono.length > 0) this._ingest(mono);
    return true;
  }
}

registerProcessor("mic-capture", MicCaptureProcessor);
