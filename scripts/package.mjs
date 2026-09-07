#!/usr/bin/env node
/**
 * Builds the app and collects the installers into `build/` at the project root.
 *
 * Tauri writes bundles into the Cargo target directory, which is a deep path that varies
 * by profile and bundle type. This gathers whatever was produced into one predictable
 * place so the artifacts are easy to find and easy to publish.
 *
 * Only the host platform's packages can be produced: Linux bundles need dpkg/rpm and
 * linuxdeploy, Windows installers need WiX and NSIS on Windows. Cross-building installers
 * is not supported, CI builds each platform on its own runner.
 */

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readdirSync, rmSync, statSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { rustEnv } from "./rust-env.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "build");
const BUNDLE_DIR = join(ROOT, "target", "release", "bundle");

/** Bundle types worth attempting per platform. */
const BUNDLES = {
  linux: ["deb", "rpm", "appimage"],
  win32: ["msi", "nsis"],
};

const platform = process.platform;
const bundles = BUNDLES[platform];
if (!bundles) {
  console.error(`No packaging configured for ${platform}. Supported: ${Object.keys(BUNDLES).join(", ")}`);
  process.exit(1);
}

const requested = process.argv.slice(2).filter((a) => !a.startsWith("-"));
const targets = requested.length > 0 ? requested : bundles;

console.log(`Building ${targets.join(", ")} for ${platform}...`);
const tauri = join(ROOT, "node_modules", ".bin", platform === "win32" ? "tauri.cmd" : "tauri");
try {
  execFileSync(tauri, ["build", "--bundles", targets.join(",")], {
    cwd: ROOT,
    stdio: "inherit",
    env: rustEnv(),
  });
} catch {
  // A single bundle type can fail (a missing packaging tool) while others succeed, so
  // carry on and report what actually landed rather than aborting.
  console.warn("\nBundling reported an error; collecting whatever was produced.");
}

if (!existsSync(BUNDLE_DIR)) {
  console.error(`No bundles were produced (${BUNDLE_DIR} does not exist).`);
  process.exit(1);
}

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

const INSTALLER = /\.(deb|rpm|AppImage|msi|exe)$/i;
const collected = [];

for (const type of readdirSync(BUNDLE_DIR)) {
  const dir = join(BUNDLE_DIR, type);
  if (!statSync(dir).isDirectory()) continue;
  for (const file of readdirSync(dir)) {
    if (!INSTALLER.test(file)) continue;
    const from = join(dir, file);
    if (!statSync(from).isFile()) continue;
    copyFileSync(from, join(OUT, file));
    collected.push([file, statSync(from).size]);
  }
}

if (collected.length === 0) {
  console.error("No installers found to collect.");
  process.exit(1);
}

console.log(`\nInstallers in ${OUT}:`);
for (const [name, size] of collected) {
  console.log(`  ${name}  (${(size / 1024 / 1024).toFixed(1)} MB)`);
}
