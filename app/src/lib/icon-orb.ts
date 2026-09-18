import type { PipelineState } from "./types.js";

/**
 * Lightweight, GPU-efficient voice agent visualizer.
 * Renders the brand icon surrounded by smooth state-aware aura glows and
 * volume-responsive pulses without running heavy fragment shaders.
 */
export class IconOrb {
  readonly element: HTMLElement;
  readonly iconEl: HTMLImageElement;
  readonly glowBg: HTMLElement;
  readonly glowPulse: HTMLElement;

  private state: PipelineState | string = "idle";
  private currentLevel = 0;
  private targetLevel = 0;
  private animFrameId: number | null = null;

  constructor(element: HTMLElement) {
    this.element = element;

    // Clean up any previous canvas if migrating
    const oldCanvas = element.querySelector(".storm-canvas");
    if (oldCanvas) oldCanvas.remove();

    // Ensure glow layers and icon exist
    let glowBg = element.querySelector<HTMLElement>(".orb-glow-bg");
    if (!glowBg) {
      glowBg = document.createElement("div");
      glowBg.className = "orb-glow-bg";
      glowBg.setAttribute("aria-hidden", "true");
      element.prepend(glowBg);
    }
    this.glowBg = glowBg;

    let glowPulse = element.querySelector<HTMLElement>(".orb-glow-pulse");
    if (!glowPulse) {
      glowPulse = document.createElement("div");
      glowPulse.className = "orb-glow-pulse";
      glowPulse.setAttribute("aria-hidden", "true");
      element.appendChild(glowPulse);
    }
    this.glowPulse = glowPulse;

    let iconEl = element.querySelector<HTMLImageElement>(".orb-icon");
    if (!iconEl) {
      iconEl = document.createElement("img");
      iconEl.className = "orb-icon";
      iconEl.src = "./assets/icon.png";
      iconEl.alt = "Open Voice Agent";
      iconEl.draggable = false;
      element.appendChild(iconEl);
    }
    this.iconEl = iconEl;

    this._frame = this._frame.bind(this);
    this.setState("idle");
  }

  setState(state: PipelineState | string): void {
    this.state = state;
    this.element.dataset.state = state;
    if (state !== "speaking" && state !== "listening" && state !== "user_speaking") {
      this.targetLevel = 0;
    }
    this._ensureLoop();
  }

  setInput(level: number, _waveform?: ArrayLike<number>): void {
    if (this.state === "listening" || this.state === "user_speaking") {
      this.targetLevel = Math.min(1, Math.max(0, level * 5.2));
      this._ensureLoop();
    }
  }

  setOutput(level: number, _waveform?: ArrayLike<number>): void {
    if (this.state === "speaking") {
      this.targetLevel = Math.min(1, Math.max(0, level * 4.4));
      this._ensureLoop();
    }
  }

  private _ensureLoop(): void {
    if (this.animFrameId === null) {
      this.animFrameId = requestAnimationFrame(this._frame);
    }
  }

  private _frame(): void {
    const isVoiceActive =
      this.state === "speaking" || this.state === "listening" || this.state === "user_speaking";

    if (!isVoiceActive) {
      this.targetLevel = 0;
    }

    const diff = this.targetLevel - this.currentLevel;
    this.currentLevel += diff * (diff > 0 ? 0.38 : 0.16);
    this.targetLevel *= 0.86;

    if (this.currentLevel < 0.002) {
      this.currentLevel = 0;
    }

    this.element.style.setProperty("--audio-level", this.currentLevel.toFixed(3));

    // If audio is settled and state doesn't require continuous raf, pause loop
    if (this.currentLevel === 0 && this.targetLevel === 0 && !isVoiceActive) {
      this.animFrameId = null;
      return;
    }

    this.animFrameId = requestAnimationFrame(this._frame);
  }

  destroy(): void {
    if (this.animFrameId !== null) {
      cancelAnimationFrame(this.animFrameId);
      this.animFrameId = null;
    }
  }
}
