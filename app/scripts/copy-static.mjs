// Copies everything in src/ that isn't TypeScript (HTML, CSS, images) into
// dist/, where tsc emits the compiled modules alongside it. dist/ is what both
// Tauri (`frontendDist`) and the embedded web server (`include_dir!`) serve.
//
//   node scripts/copy-static.mjs [--clean] [--no-copy]

import { rm, cp, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const appDir = path.dirname(fileURLToPath(new URL("../package.json", import.meta.url)));
const src = path.join(appDir, "src");
const dist = path.join(appDir, "dist");

const args = new Set(process.argv.slice(2));

if (args.has("--clean")) await rm(dist, { recursive: true, force: true });

if (!args.has("--no-copy")) {
  await mkdir(dist, { recursive: true });
  await cp(src, dist, {
    recursive: true,
    filter: (from) => !/\.(ts|tsbuildinfo)$/.test(from),
  });
}
