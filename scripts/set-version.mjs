/**
 * Writes one version into every file that carries it.
 *
 * The release workflow runs this from the git tag, so the tag is the single source of
 * truth and the committed numbers are only a placeholder. Without it a tag names the
 * release while the artifacts inside keep whatever version was last committed, which
 * fails silently: nothing errors, the files are just wrong.
 *
 * Usage: node scripts/set-version.mjs 1.2.3
 */

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Strips a leading `v` so both `v1.2.3` and `1.2.3` work. */
function normalize(input) {
  const version = String(input ?? "").trim().replace(/^v/, "");
  // Cargo requires three numeric parts; a pre-release suffix is allowed after them.
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`"${input}" is not a version like 1.2.3`);
  }
  return version;
}

/** Replaces one pattern in a file, failing loudly if it is not found exactly once. */
function edit(relative, pattern, replacement) {
  const path = join(root, relative);
  const before = readFileSync(path, "utf8");
  const matches = before.match(pattern);
  if (!matches || matches.length !== 1) {
    throw new Error(`${relative}: expected 1 match for ${pattern}, found ${matches?.length ?? 0}`);
  }
  writeFileSync(path, before.replace(pattern, replacement));
  console.log(`  ${relative}`);
}

const version = normalize(process.argv[2]);
// AppStream wants the release date, and a stale one is worse than none.
const today = new Date().toISOString().slice(0, 10);

console.log(`Setting version ${version}:`);

// The one that decides what the built artifacts are called: src-tauri inherits it, and
// Tauri reads it from there because tauri.conf.json deliberately carries no version.
edit("Cargo.toml", /^version = "\d+\.\d+\.\d+[^"]*"$/m, `version = "${version}"`);

edit("package.json", /^  "version": "\d+\.\d+\.\d+[^"]*",$/m, `  "version": "${version}",`);

edit(
  "flatpak/org.gameyfin.Gameyfin.metainfo.xml",
  /<release version="\d+\.\d+\.\d+[^"]*" date="\d{4}-\d{2}-\d{2}">/,
  `<release version="${version}" date="${today}">`,
);

console.log("Done. Cargo.lock is refreshed by the next cargo command.");
