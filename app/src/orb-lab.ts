import { IconOrb } from "./lib/icon-orb.js";
import { element, input as inputEl, output as outputEl, select } from "./lib/dom.js";

const state = select("state");
const input = inputEl("input");
const output = inputEl("output");
const inputValue = outputEl("input-value");
const outputValue = outputEl("output-value");
const animate = inputEl("animate");
const heroOrbEl = element("hero-orb");
const hero = new IconOrb(heroOrbEl);

state.addEventListener("change", () => {
  heroOrbEl.className = `orb state-${state.value}`;
  hero.setState(state.value);
});
state.dispatchEvent(new Event("change"));

/** [state, input level, output level, label] */
type Case = [string, number, number, string];

const CASES: Case[] = [
  ["idle", 0, 0, "At rest"],
  ["connecting", 0, 0, "Connecting"],
  ["listening", 0.035, 0, "Listening · room tone"],
  ["user_speaking", 0.13, 0, "Audio in · voice"],
  ["thinking", 0, 0, "Thinking"],
  ["delegating", 0, 0, "Delegating"],
  ["speaking", 0, 0.12, "Audio out · soft"],
  ["speaking", 0, 0.2, "Audio out · loud"],
  ["error", 0, 0, "Disconnected"],
];

interface GalleryOrb {
  visual: IconOrb;
  inLevel: number;
  outLevel: number;
  phase: number;
}

const galleryOrbs: GalleryOrb[] = [];
for (const [orbState, inLevel, outLevel, label] of CASES) {
  const card = document.createElement("figure");
  card.className = "case";
  const mount = document.createElement("div");
  mount.className = `orb state-${orbState}`;
  const caption = document.createElement("figcaption");
  caption.innerHTML = `<strong>${label}</strong><span>in ${inLevel.toFixed(2)} · out ${outLevel.toFixed(2)}</span>`;
  card.append(mount, caption);
  element("gallery").append(card);
  const visual = new IconOrb(mount);
  visual.setState(orbState);
  galleryOrbs.push({ visual, inLevel, outLevel, phase: Math.random() * 6 });
}

document.querySelectorAll<HTMLElement>("[data-preset]").forEach((button) =>
  button.addEventListener("click", () => {
    const preset = button.dataset.preset;
    animate.checked = false;
    input.value = String(preset === "input" ? 0.14 : 0);
    output.value = String(preset === "output" ? 0.18 : 0);
    state.value = preset === "input" ? "user_speaking" : preset === "output" ? "speaking" : "idle";
    state.dispatchEvent(new Event("change"));
  }),
);

function pump(now: number): void {
  const t = now / 1000;
  let inLevel = Number(input.value);
  let outLevel = Number(output.value);
  if (animate.checked) {
    const voice = Math.max(0, Math.sin(t * 7) * 0.55 + Math.sin(t * 13.7) * 0.28 + 0.15);
    if (state.value === "speaking") outLevel = voice * 0.22;
    else inLevel = voice * 0.16;
  }
  inputValue.value = inLevel.toFixed(2);
  outputValue.value = outLevel.toFixed(2);
  hero.setInput(inLevel);
  hero.setOutput(outLevel);
  for (const item of galleryOrbs) {
    const cadence = 0.6 + 0.4 * Math.max(0, Math.sin(t * 6.5 + item.phase));
    const actualIn = item.inLevel * cadence;
    const actualOut = item.outLevel * cadence;
    item.visual.setInput(actualIn);
    item.visual.setOutput(actualOut);
  }
  requestAnimationFrame(pump);
}
requestAnimationFrame(pump);
