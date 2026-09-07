#!/usr/bin/env node
/**
 * Builds a Flatpak bundle from the packaged `.deb`.
 *
 * Requires `flatpak` and `flatpak-builder`, plus the GNOME runtime and SDK:
 *
 *   flatpak remote-add --if-not-exists --user flathub \
 *     https://flathub.org/repo/flathub.flatpakrepo
 *   flatpak install --user flathub org.gnome.Platform//48 org.gnome.Sdk//48
 *
 * The result is `build/Gameyfin_<version>.flatpak`, installable with
 * `flatpak install --user ./Gameyfin_<version>.flatpak`.
 *
 * Pass `--install` to install straight from the build directory instead. That skips the
 * repository commit and the bundle round-trip, which is most of the time a bundle install
 * spends, worth using while iterating, since only the changed objects are written.
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

// The manifest expects the deb at a stable filename, since its version is in the name.
const deb = readdirSync(BUILD, { withFileTypes: true })
  .filter((e) => e.isFile() && e.name.endsWith(".deb") && e.name !== "gameyfin.deb")
  .map((e) => e.name)
  .sort()
  .pop();

if (!deb) {
  console.error("No .deb found in build/. Run `npm run package deb` first.");
  process.exit(1);
}
copyFileSync(join(BUILD, deb), join(BUILD, "gameyfin.deb"));

const version = deb.match(/_(\d+\.\d+\.\d+)_/)?.[1] ?? "0.0.0";
const buildDir = join(ROOT, "target", "flatpak-build");
const repoDir = join(ROOT, "target", "flatpak-repo");
rmSync(buildDir, { recursive: true, force: true });
rmSync(repoDir, { recursive: true, force: true });

const installDirectly = process.argv.includes("--install");

console.log(`Building ${APP_ID} ${version}...`);

if (installDirectly) {
  // Installs from the build tree. Only changed objects are written, so this is far
  // quicker than committing to a repo, packing a bundle and importing it again.
  execFileSync(
    "flatpak-builder",
    ["--force-clean", "--user", "--install", buildDir, join(FLATPAK_DIR, `${APP_ID}.yml`)],
    { cwd: ROOT, stdio: "inherit" },
  );
  rmSync(join(BUILD, "gameyfin.deb"), { force: true });
  console.log(`\nInstalled ${APP_ID}. Run it with: flatpak run ${APP_ID}`);
} else {
  execFileSync(
    "flatpak-builder",
    ["--force-clean", "--repo", repoDir, buildDir, join(FLATPAK_DIR, `${APP_ID}.yml`)],
    { cwd: ROOT, stdio: "inherit" },
  );

  mkdirSync(BUILD, { recursive: true });
  const bundle = join(BUILD, `Gameyfin_${version}.flatpak`);
  execFileSync("flatpak", ["build-bundle", repoDir, bundle, APP_ID], {
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
