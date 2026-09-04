// Playback worklet: play Float32 mono samples from the main thread, resampling
// the server's rate (16 kHz PCM16 for the speech-to-speech cascade) up to the
// AudioContext rate. Adapted from huggingface/speech-to-speech
// (demo/worklets/audio-playback.js, Apache-2.0).

const STATS_INTERVAL_FRAMES = 12000;
const FADE_FRAMES = 32;
const PREBUFFER_MS = 400;

class AudioPlaybackProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this._inputRate = 16000;
    this._stepRatio = this._inputRate / sampleRate;
    this._queue = [];
    this._readIdx = 0;
    this._fracPos = 0;
    this._playing = false;
    this._framesSinceStats = 0;
    this._fadeIn = 0;
    this._fadeOut = 0;
    this._lastSample = 0;

    this.port.onmessage = (e) => {
      const data = e.data;
      if (!data || typeof data !== "object") return;
      switch (data.kind) {
        case "config":
          if (typeof data.inputRate === "number" && data.inputRate > 0) {
            this._inputRate = data.inputRate;
            this._stepRatio = this._inputRate / sampleRate;
          }
          break;
        case "audio":
          if (data.samples instanceof Float32Array && data.samples.length > 0) {
            this._queue.push(data.samples);
            if (!this._playing && this._queuedSamples() >= (this._inputRate * PREBUFFER_MS) / 1000) {
              this._startPlayback();
            }
          }
          break;
        case "flush":
          if (!this._playing && this._queue.length > 0) this._startPlayback();
          break;
        case "clear":
          this._queue.length = 0;
          this._readIdx = 0;
          this._fracPos = 0;
          this._fadeOut = FADE_FRAMES;
          break;
      }
    };
  }

  _startPlayback() {
    this._playing = true;
    this._fadeIn = FADE_FRAMES;
    this._fadeOut = 0;
  }

  _queuedSamples() {
    let total = -this._readIdx;
    for (const buf of this._queue) total += buf.length;
    return Math.max(0, total);
  }

  _readInterpolated() {
    if (this._queue.length === 0) return null;
    const head = this._queue[0];
    const idx = this._readIdx;
    const frac = this._fracPos;
    const a = head[idx];
    let b;
    if (idx + 1 < head.length) b = head[idx + 1];
    else if (this._queue.length > 1) b = this._queue[1][0];
    else b = a;
    return a + (b - a) * frac;
  }

  _advance() {
    this._fracPos += this._stepRatio;
    while (this._fracPos >= 1) {
      this._fracPos -= 1;
      this._readIdx += 1;
    }
    while (this._queue.length > 0 && this._readIdx >= this._queue[0].length) {
      this._readIdx -= this._queue[0].length;
      this._queue.shift();
    }
  }

  process(_, outputs) {
    const channels = outputs[0];
    if (!channels || channels.length === 0) return true;
    const out = channels[0];
    const stereo = channels.length > 1 ? channels[1] : null;

    for (let i = 0; i < out.length; i++) {
      let sample = 0;
      if (this._playing) {
        const v = this._readInterpolated();
        if (v === null) {
          sample = this._lastSample * Math.max(0, 1 - 1 / FADE_FRAMES);
          this._lastSample = sample;
          if (Math.abs(sample) < 1e-4) {
            this._playing = false;
            this._lastSample = 0;
            this.port.postMessage({ kind: "underrun" });
          }
        } else {
          sample = v;
          this._lastSample = v;
          this._advance();
        }
        if (this._fadeIn > 0) {
          sample *= 1 - this._fadeIn / FADE_FRAMES;
          this._fadeIn -= 1;
        }
        if (this._fadeOut > 0) {
          sample *= this._fadeOut / FADE_FRAMES;
          this._fadeOut -= 1;
          if (this._fadeOut === 0) {
            this._playing = false;
            this._lastSample = 0;
          }
        }
      }
      out[i] = sample;
      if (stereo) stereo[i] = sample;
    }

    this._framesSinceStats += out.length;
    if (this._framesSinceStats >= STATS_INTERVAL_FRAMES) {
      this._framesSinceStats = 0;
      const queuedMs = (this._queuedSamples() / this._inputRate) * 1000;
      this.port.postMessage({ kind: "stats", queuedMs });
    }
    return true;
  }
}

registerProcessor("audio-playback", AudioPlaybackProcessor);
