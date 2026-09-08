#!/usr/bin/env node
/**
 * Downloads the pinned Ludusavi release as a Tauri sidecar (named `ludusavi-<triple>`).
 * MIT licensed; `LUDUSAVI-LICENSE.txt` is written alongside to carry the notice.
 */

import { execFileSync } from "node:child_process";
import { mkdirSync, existsSync, writeFileSync, renameSync, rmSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

/** Pinned: the save-backup engine must not change under us. */
const VERSION = "0.31.0";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(ROOT, "src-tauri", "binaries");

const TARGETS = {
  "linux-x64": { triple: "x86_64-unknown-linux-gnu", asset: `ludusavi-v${VERSION}-linux.tar.gz`, exe: "ludusavi" },
  "win32-x64": { triple: "x86_64-pc-windows-msvc", asset: `ludusavi-v${VERSION}-win64.zip`, exe: "ludusavi.exe" },
};

const key = `${process.platform}-${process.arch}`;
const target = TARGETS[key];
if (!target) {
  console.error(`No pinned Ludusavi build for ${key}. Supported: ${Object.keys(TARGETS).join(", ")}`);
  process.exit(1);
}

const suffix = process.platform === "win32" ? ".exe" : "";
const finalPath = join(OUT_DIR, `ludusavi-${target.triple}${suffix}`);

// Ask the binary rather than trusting its presence: bumping VERSION used to leave the
// old build in place, so the app shipped a version nobody had chosen.
if (existsSync(finalPath) && versionOf(finalPath) === VERSION) {
  console.log(`Ludusavi ${VERSION} already present at ${finalPath}`);
  process.exit(0);
}

/** The version an installed binary reports, or null if it cannot be run. */
function versionOf(binary) {
  try {
    const out = execFileSync(binary, ["--version"], { encoding: "utf8" });
    return out.trim().split(/\s+/).pop()?.replace(/^v/, "") ?? null;
  } catch {
    return null;
  }
}

mkdirSync(OUT_DIR, { recursive: true });
const url = `https://github.com/mtkennerly/ludusavi/releases/download/v${VERSION}/${target.asset}`;
const tmpDir = join(OUT_DIR, ".tmp");
rmSync(tmpDir, { recursive: true, force: true });
mkdirSync(tmpDir, { recursive: true });

console.log(`Downloading ${url}`);
const response = await fetch(url, { redirect: "follow" });
if (!response.ok) {
  console.error(`Download failed: HTTP ${response.status} ${response.statusText}`);
  process.exit(1);
}
const archive = join(tmpDir, target.asset);
writeFileSync(archive, Buffer.from(await response.arrayBuffer()));

// `tar` reads zip on modern Windows and the Linux runners, so no extra dependency.
execFileSync("tar", ["-xf", archive, "-C", tmpDir], { stdio: "inherit" });

const extracted = readdirSync(tmpDir).find((f) => f === target.exe);
if (!extracted) {
  console.error(`Archive did not contain ${target.exe}; got: ${readdirSync(tmpDir).join(", ")}`);
  process.exit(1);
}

renameSync(join(tmpDir, extracted), finalPath);
if (process.platform !== "win32") execFileSync("chmod", ["+x", finalPath]);
rmSync(tmpDir, { recursive: true, force: true });

writeFileSync(
  join(OUT_DIR, "LUDUSAVI-LICENSE.txt"),
  `Ludusavi v${VERSION}. MIT License. Copyright (c) 2020 Matthew T. Kennerly\n` +
    `https://github.com/mtkennerly/ludusavi/blob/master/LICENSE\n`,
);

console.log(`Ludusavi ${VERSION} installed at ${finalPath}`);
