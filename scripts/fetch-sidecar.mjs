#!/usr/bin/env node
/**
 * Downloads the Tauri sidecars, checked against pinned SHA-256s since they ship in a signed release.
 * Usage: `node scripts/fetch-sidecar.mjs [ludusavi|umu]`, all of them when given nothing.
 */

import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(ROOT, "src-tauri", "binaries");

/**
 * The pinned helpers. `sha256` is of the downloaded archive: run this script after a
 * version bump and it prints the hash it got, which is what goes here.
 */
const SIDECARS = {
  ludusavi: {
    version: "0.31.0",
    license: [
      "LUDUSAVI-LICENSE.txt",
      (v) =>
        `Ludusavi v${v}. MIT License. Copyright (c) 2020 Matthew T. Kennerly\n` +
        `https://github.com/mtkennerly/ludusavi/blob/master/LICENSE\n`,
    ],
    url: (v, asset) => `https://github.com/mtkennerly/ludusavi/releases/download/v${v}/${asset}`,
    targets: {
      "linux-x64": {
        triple: "x86_64-unknown-linux-gnu",
        asset: (v) => `ludusavi-v${v}-linux.tar.gz`,
        member: "ludusavi",
        sha256: "7322ff45d41eae7ae064a80d8c9ecccc5b8fb6fc090a603a66369cd4b054068d",
      },
      "win32-x64": {
        triple: "x86_64-pc-windows-msvc",
        asset: (v) => `ludusavi-v${v}-win64.zip`,
        member: "ludusavi.exe",
        sha256: "f47a8ad8c708f01d2eb124704973beffab205e292f5287a10fc4a101f8d68706",
      },
    },
    // Ask the binary rather than trusting its presence, so a version bump replaces the old build.
    installed: (binary) => {
      const run = spawnSync(binary, ["--version"], { encoding: "utf8" });
      if (run.error || run.status !== 0) return null;
      return run.stdout.trim().split(/\s+/).pop()?.replace(/^v/, "") ?? null;
    },
  },
  umu: {
    version: "1.4.4",
    license: [
      "UMU-LICENSE.txt",
      (v) =>
        `umu-launcher ${v}. GNU General Public License v3.0, shipped unmodified.\n` +
        `Source: https://github.com/Open-Wine-Components/umu-launcher/tree/${v}\n` +
        `Licence: https://github.com/Open-Wine-Components/umu-launcher/blob/${v}/LICENSE\n`,
    ],
    url: (v, asset) =>
      `https://github.com/Open-Wine-Components/umu-launcher/releases/download/${v}/${asset}`,
    // A Python zipapp, so one download serves every architecture Tauri can target.
    targets: {
      "linux-x64": {
        triple: "x86_64-unknown-linux-gnu",
        asset: (v) => `umu-launcher-${v}-zipapp.tar`,
        member: join("umu", "umu-run"),
        name: "umu-run",
        sha256: "eb590691841f7fad3fc3ad8fd5db4ccb87849fe7948e62b28ece7a4ee48cc851",
      },
      "linux-arm64": {
        triple: "aarch64-unknown-linux-gnu",
        asset: (v) => `umu-launcher-${v}-zipapp.tar`,
        member: join("umu", "umu-run"),
        name: "umu-run",
        sha256: "eb590691841f7fad3fc3ad8fd5db4ccb87849fe7948e62b28ece7a4ee48cc851",
      },
    },
    // Bundled only on Linux, and never run here: a downloaded program is not something to
    // execute on the build machine just to ask its version, so the marker file answers.
    onlyOn: "linux",
  },
};

const wanted = process.argv.slice(2);
for (const [name, sidecar] of Object.entries(SIDECARS)) {
  if (wanted.length > 0 && !wanted.includes(name)) continue;
  await fetchSidecar(name, sidecar);
}

async function fetchSidecar(name, sidecar) {
  if (sidecar.onlyOn && process.platform !== sidecar.onlyOn) {
    console.log(`${name} is only bundled on ${sidecar.onlyOn}; nothing to do.`);
    return;
  }
  const key = `${process.platform}-${process.arch}`;
  const target = sidecar.targets[key];
  if (!target) {
    fail(`No pinned ${name} build for ${key}. Supported: ${Object.keys(sidecar.targets).join(", ")}`);
  }

  const { version } = sidecar;
  const suffix = process.platform === "win32" ? ".exe" : "";
  const binary = join(OUT_DIR, `${target.name ?? name}-${target.triple}${suffix}`);
  // The marker covers the helpers there is no safe way to ask, and the probe catches a
  // version bump that would otherwise leave the old build in place.
  const marker = `${binary}.version`;
  const present = sidecar.installed
    ? sidecar.installed(binary) === version
    : existsSync(marker) && readFileSync(marker, "utf8").trim() === version;
  if (existsSync(binary) && present) {
    console.log(`${name} ${version} already present at ${binary}`);
    return;
  }

  const asset = target.asset(version);
  const url = sidecar.url(version, asset);
  const tmpDir = join(OUT_DIR, `.tmp-${name}`);
  rmSync(tmpDir, { recursive: true, force: true });
  mkdirSync(tmpDir, { recursive: true });

  console.log(`Downloading ${url}`);
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) fail(`Download failed: HTTP ${response.status} ${response.statusText}`);
  const bytes = Buffer.from(await response.arrayBuffer());

  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== target.sha256) {
    fail(`${asset} does not match its pinned checksum.\n  expected ${target.sha256}\n  got      ${digest}`);
  }

  const archive = join(tmpDir, asset);
  writeFileSync(archive, bytes);
  // `tar` reads zip on modern Windows and on the Linux runners, so no extra dependency.
  execFileSync("tar", ["-xf", archive, "-C", tmpDir], { stdio: "inherit" });

  const extracted = join(tmpDir, target.member);
  if (!existsSync(extracted)) {
    fail(`${asset} did not contain ${target.member}; got: ${readdirSync(tmpDir).join(", ")}`);
  }
  mkdirSync(OUT_DIR, { recursive: true });
  renameSync(extracted, binary);
  if (process.platform !== "win32") chmodSync(binary, 0o755);
  rmSync(tmpDir, { recursive: true, force: true });

  writeFileSync(marker, `${version}\n`);
  const [licenseName, licenseText] = sidecar.license;
  writeFileSync(join(OUT_DIR, licenseName), licenseText(version));
  console.log(`${name} ${version} installed at ${binary}`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
