#!/usr/bin/env node
/**
 * Builds the app and gathers the installers Tauri scatters through the target directory
 * into `build/`. Host platform only; CI builds each platform on its own runner.
 */

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  utimesSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { rustEnv } from "./rust-env.mjs";
import { signingProblems } from "./check-signing.mjs";

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
// The CLI's own JS entry, not the .bin shim. Node refuses to execFileSync a .cmd without a
// shell, and running one through cmd.exe would mangle the quotes in the --config JSON below.
const tauri = join(ROOT, "node_modules", "@tauri-apps", "cli", "tauri.js");

// Marks this run, so leftovers in `target/` can be told apart from what it produced. A
// failed build used to be reported as a success, listing stale installers as if fresh.
const startedAt = Date.now();
let buildFailed = false;

// Tauri treats a missing signing key as an error, not a reason to skip updater artifacts,
// so a local build failed after every bundle was already written. CI sets the key.
const args = ["build", "--bundles", targets.join(",")];
if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  console.log("No TAURI_SIGNING_PRIVATE_KEY set; building without updater artifacts.");
  args.push("--config", JSON.stringify({ bundle: { createUpdaterArtifacts: false } }));
} else {
  // A key that is set but unusable would only surface after the whole build.
  const config = JSON.parse(readFileSync(join(ROOT, "src-tauri", "tauri.conf.json"), "utf8"));
  const problems = signingProblems({
    privateKey: process.env.TAURI_SIGNING_PRIVATE_KEY,
    config,
  });
  for (const problem of problems) console.warn(`Warning: ${problem}`);
}

try {
  execFileSync(process.execPath, [tauri, ...args], {
    cwd: ROOT,
    stdio: "inherit",
    env: rustEnv(),
  });
} catch (error) {
  // One bundle type can fail while others succeed; report what landed.
  buildFailed = true;
  console.warn("\nBundling reported an error; collecting whatever this run produced.");
  // Tauri's own output already went to the terminal, but a failure to *start* it prints
  // nothing at all, which is how a Windows build could fail in total silence.
  if (error?.code) console.warn(`Could not run the Tauri CLI: ${error.code} ${error.message}`);
}

if (!existsSync(BUNDLE_DIR)) {
  console.error(`No bundles were produced (${BUNDLE_DIR} does not exist).`);
  process.exit(1);
}

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

const INSTALLER = /\.(deb|rpm|AppImage|msi|exe)$/i;
const collected = [];
const stale = [];

for (const type of readdirSync(BUNDLE_DIR)) {
  const dir = join(BUNDLE_DIR, type);
  if (!statSync(dir).isDirectory()) continue;
  for (const file of readdirSync(dir)) {
    if (!INSTALLER.test(file)) continue;
    const from = join(dir, file);
    const info = statSync(from);
    if (!info.isFile()) continue;
    // A second's slack: some bundlers stamp the file from just before they were invoked.
    if (info.mtimeMs < startedAt - 1000) {
      stale.push(file);
      continue;
    }
    const to = join(OUT, file);
    copyFileSync(from, to);
    // Keep the build's own timestamp, so the next run judges it the same way.
    utimesSync(to, info.atime, info.mtime);
    collected.push([file, info.size]);
  }
}

if (stale.length > 0) {
  console.warn(`\nIgnored ${stale.length} installer(s) left from an earlier build: ${stale.join(", ")}`);
}

if (collected.length > 0) {
  console.log(`\nInstallers in ${OUT}:`);
  for (const [name, size] of collected) {
    console.log(`  ${name}  (${(size / 1024 / 1024).toFixed(1)} MB)`);
  }
}

if (buildFailed) {
  console.error(
    collected.length > 0
      ? "\nThe build reported an error; the installers above are only the bundles that got that far."
      : "\nThe build failed and produced nothing; see the error above.",
  );
  if (platform === "linux") {
    console.error(
      "The usual cause is a missing dev package: libudev-dev (systemd-devel on Fedora) plus the WebKit and GTK ones.",
    );
  }
  process.exit(1);
}

if (collected.length === 0) {
  console.error("No installers found to collect.");
  process.exit(1);
}
