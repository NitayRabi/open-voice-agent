import { StormOrb } from "./lib/storm-orb.js";
import { element, input as inputEl, output as outputEl, select } from "./lib/dom.js";

const state = select("state");
const input = inputEl("input");
const output = inputEl("output");
const inputValue = outputEl("input-value");
const outputValue = outputEl("output-value");
const animate = inputEl("animate");
const hero = new StormOrb(element("hero-orb"));

state.addEventListener("change", () => hero.setState(state.value));
state.dispatchEvent(new Event("change"));

/** [state, input level, output level, label] */
type Case = [string, number, number, string];

const CASES: Case[] = [
  ["idle", 0, 0, "At rest"], ["connecting", 0, 0, "Connecting"],
  ["listening", .035, 0, "Listening · room tone"], ["user_speaking", .13, 0, "Audio in · voice"],
  ["thinking", 0, 0, "Thinking"], ["delegating", 0, 0, "Delegating"],
  ["speaking", 0, .12, "Audio out · soft"], ["speaking", 0, .2, "Audio out · loud"],
  ["error", 0, 0, "Disconnected"],
];

interface GalleryOrb {
  visual: StormOrb;
  inLevel: number;
  outLevel: number;
  phase: number;
}

const galleryOrbs: GalleryOrb[] = [];
for (const [orbState, inLevel, outLevel, label] of CASES) {
  const card = document.createElement("figure"); card.className = "case";
  const mount = document.createElement("div"); mount.className = "orb";
  const caption = document.createElement("figcaption");
  caption.innerHTML = `<strong>${label}</strong><span>in ${inLevel.toFixed(2)} · out ${outLevel.toFixed(2)}</span>`;
  card.append(mount, caption); element("gallery").append(card);
  const visual = new StormOrb(mount); visual.setState(orbState);
  galleryOrbs.push({ visual, inLevel, outLevel, phase: Math.random() * 6 });
}

document.querySelectorAll<HTMLElement>("[data-preset]").forEach((button) => button.addEventListener("click", () => {
  const preset = button.dataset.preset;
  animate.checked = false;
  input.value = String(preset === "input" ? .14 : 0);
  output.value = String(preset === "output" ? .18 : 0);
  state.value = preset === "input" ? "user_speaking" : preset === "output" ? "speaking" : "idle";
  state.dispatchEvent(new Event("change"));
}));

function pump(now: number): void {
  const t = now / 1000;
  let inLevel = Number(input.value), outLevel = Number(output.value);
  if (animate.checked) {
    const voice = Math.max(0, Math.sin(t * 7) * .55 + Math.sin(t * 13.7) * .28 + .15);
    if (state.value === "speaking") outLevel = voice * .22;
    else inLevel = voice * .16;
  }
  const makeWave = (level: number, offset = 0) => Float32Array.from({ length: 128 }, (_, i) => {
    const x = i / 128 * Math.PI * 2;
    return level * 1.7 * (Math.sin(x * 3 + t * 13 + offset) * .55 + Math.sin(x * 7 - t * 19) * .28 + Math.sin(x * 11 + t * 7) * .17);
  });
  inputValue.value = inLevel.toFixed(2); outputValue.value = outLevel.toFixed(2);
  hero.setInput(inLevel, makeWave(inLevel)); hero.setOutput(outLevel, makeWave(outLevel, 1.7));
  for (const item of galleryOrbs) {
    const cadence = .6 + .4 * Math.max(0, Math.sin(t * 6.5 + item.phase));
    const actualIn = item.inLevel * cadence, actualOut = item.outLevel * cadence;
    item.visual.setInput(actualIn, makeWave(actualIn, item.phase));
    item.visual.setOutput(actualOut, makeWave(actualOut, item.phase + 1.7));
  }
  requestAnimationFrame(pump);
}
requestAnimationFrame(pump);
