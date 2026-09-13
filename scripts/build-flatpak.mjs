#!/usr/bin/env node
/**
 * Builds `build/Gameyfin_<version>.flatpak` from the packaged `.deb`. Needs `flatpak-builder`
 * and the GNOME 50 SDK. `--install` installs straight from the build tree instead.
 */

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BUILD = join(ROOT, "build");
const FLATPAK_DIR = join(ROOT, "flatpak");
const APP_ID = "org.gameyfin.Gameyfin";

function require(tool) {
  try {
    execFileSync("which", [tool], { stdio: "pipe" });
  } catch {
    console.error(`${tool} is not installed.`);
    console.error("On Debian/Ubuntu: sudo apt-get install flatpak flatpak-builder");
    process.exit(1);
  }
}

require("flatpak");
require("flatpak-builder");

/** The version in a deb's filename, as `[major, minor, patch]`; `null` when it has none. */
function versionOf(name) {
  const parts = name.match(/_(\d+)\.(\d+)\.(\d+)(?:[-.~][^_]*)?_/);
  return parts ? parts.slice(1, 4).map(Number) : null;
}

// Copied to a stable filename for the manifest. Compared as numbers so 1.10.0 beats 1.9.0.
const deb = readdirSync(BUILD, { withFileTypes: true })
  .filter((e) => e.isFile() && e.name.endsWith(".deb") && e.name !== "gameyfin.deb")
  .map((e) => e.name)
  .sort((a, b) => {
    const [left, right] = [versionOf(a) ?? [0, 0, 0], versionOf(b) ?? [0, 0, 0]];
    return left[0] - right[0] || left[1] - right[1] || left[2] - right[2];
  })
  .pop();

if (!deb) {
  console.error("No .deb found in build/. Run `npm run package deb` first.");
  process.exit(1);
}
copyFileSync(join(BUILD, deb), join(BUILD, "gameyfin.deb"));

// Pre-release versions are legal in a tag, so the suffix is kept rather than dropped.
const version = deb.match(/_([^_]+)_/)?.[1] ?? "0.0.0";
const buildDir = join(ROOT, "target", "flatpak-build");
const repoDir = join(ROOT, "target", "flatpak-repo");
rmSync(buildDir, { recursive: true, force: true });
rmSync(repoDir, { recursive: true, force: true });

// Set by the release workflow. A local build carries no key, so it still installs unsigned.
const gpgKey = process.env.FLATPAK_GPG_KEY_ID;
const repoUrl = process.env.FLATPAK_REPO_URL;
const publicKey = join(FLATPAK_DIR, "gameyfin-repo.gpg");
if (gpgKey && !existsSync(publicKey)) {
  console.error(`FLATPAK_GPG_KEY_ID is set, but ${publicKey} is missing.`);
  process.exit(1);
}

const installDirectly = process.argv.includes("--install");

console.log(`Building ${APP_ID} ${version}...`);

if (installDirectly) {
  // Installs from the build tree: only changed objects are written.
  execFileSync(
    "flatpak-builder",
    ["--force-clean", "--user", "--install", buildDir, join(FLATPAK_DIR, `${APP_ID}.yml`)],
    { cwd: ROOT, stdio: "inherit" },
  );
  rmSync(join(BUILD, "gameyfin.deb"), { force: true });
  console.log(`\nInstalled ${APP_ID}. Run it with: flatpak run ${APP_ID}`);
} else {
  const signing = gpgKey ? [`--gpg-sign=${gpgKey}`] : [];
  execFileSync(
    "flatpak-builder",
    ["--force-clean", ...signing, "--repo", repoDir, buildDir, join(FLATPAK_DIR, `${APP_ID}.yml`)],
    { cwd: ROOT, stdio: "inherit" },
  );

  mkdirSync(BUILD, { recursive: true });
  const bundle = join(BUILD, `Gameyfin_${version}.flatpak`);
  // An install keeps this URL and key, so it updates from the repo and rejects unsigned builds.
  const origin = [
    ...(repoUrl ? [`--repo-url=${repoUrl}`] : []),
    ...(gpgKey ? [`--gpg-keys=${publicKey}`] : []),
  ];
  execFileSync("flatpak", ["build-bundle", ...origin, repoDir, bundle, APP_ID], {
    cwd: ROOT,
    stdio: "inherit",
  });

  rmSync(join(BUILD, "gameyfin.deb"), { force: true });

  if (!existsSync(bundle)) {
    console.error("flatpak build-bundle reported success but produced no file.");
    process.exit(1);
  }
  console.log(`\nFlatpak bundle: ${bundle}`);
}
