import { IconOrb } from "./icon-orb.js";

/**
 * Lightweight, icon-centric voice agent visualizer.
 * Maintained as an alias to `IconOrb` for backwards compatibility.
 */
export class StormOrb extends IconOrb {
  // Expose gl property as null so fallback checks cleanly pass
  readonly gl = null;
}
