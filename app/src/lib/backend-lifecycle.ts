import { invoke } from "./tauri.js";
import type { Settings } from "./types.js";

const READY_TIMEOUT_MS = 10 * 60_000;
const READY_POLL_MS = 750;

const delay = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Start the managed voice stack on demand, then wait until its HTTP health
 * endpoint responds. Externally managed backends retain their old behavior.
 */
export async function ensureVoiceBackend(settings: Settings): Promise<void> {
  if (!settings.manage_backend) return;

  await invoke("backend_start");
  const deadline = Date.now() + READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await invoke("backend_ready").catch(() => false)) return;
    await invoke("backend_touch").catch(() => {});
    await delay(READY_POLL_MS);
  }
  throw new Error("voice engine did not become ready within 10 minutes");
}

/** Keep the managed engine resident while at least one voice session is open. */
export function keepVoiceBackendAlive(settings: Settings): void {
  if (settings.manage_backend) void invoke("backend_touch").catch(() => {});
}
