#!/usr/bin/env node
/**
 * Renders the Gameyfin logo into the app's icon set. The logo's non-square viewBox is
 * widened to a padded square (artwork untouched), then `tauri icon` produces every size.
 */

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { Resvg } from "@resvg/resvg-js";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE = join(ROOT, "assets", "logo.svg");
const RENDERED = join(ROOT, "assets", "icon-1024.png");

/** Fraction of the artwork's longest edge left as margin on each side. */
const PADDING = 0.12;

const svg = readFileSync(SOURCE, "utf8");

const viewBox = svg.match(/viewBox="([\d.\-\s]+)"/);
if (!viewBox) {
  console.error("Could not find a viewBox on the logo; cannot square it reliably.");
  process.exit(1);
}

const [x, y, width, height] = viewBox[1].trim().split(/\s+/).map(Number);
const side = Math.max(width, height) * (1 + PADDING * 2);
// Centre the original artwork inside the new square.
const squared = [
  x - (side - width) / 2,
  y - (side - height) / 2,
  side,
  side,
].map((n) => n.toFixed(2));

const squareSvg = svg.replace(viewBox[0], `viewBox="${squared.join(" ")}"`);

const resvg = new Resvg(squareSvg, {
  fitTo: { mode: "width", value: 1024 },
  // Transparent: the platforms apply their own masking and backgrounds.
  background: "rgba(0,0,0,0)",
});

mkdirSync(dirname(RENDERED), { recursive: true });
writeFileSync(RENDERED, resvg.render().asPng());
console.log(`Rendered ${RENDERED} (1024x1024)`);

const tauri = join(ROOT, "node_modules", ".bin", process.platform === "win32" ? "tauri.cmd" : "tauri");
execFileSync(tauri, ["icon", RENDERED, "--output", join(ROOT, "src-tauri", "icons")], {
  cwd: ROOT,
  stdio: "inherit",
});
console.log("Icon set written to src-tauri/icons/");
