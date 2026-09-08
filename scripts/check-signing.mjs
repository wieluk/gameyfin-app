#!/usr/bin/env node
/**
 * Checks the updater signing setup before a release build starts.
 *
 * Tauri signs at the very end, so a missing or malformed key wasted the whole build and
 * then failed with a message ("Missing comment in secret key") that names neither the
 * variable nor the fix.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CONFIG = join(ROOT, "src-tauri", "tauri.conf.json");

/** Decode a Tauri key file's contents, which are base64 around the minisign text. */
function decode(value) {
  try {
    return Buffer.from(value, "base64").toString("utf8");
  } catch {
    return "";
  }
}

/** Problems with the signing setup, each with what to do about it. */
export function signingProblems({ privateKey, config }) {
  if (!config.bundle?.createUpdaterArtifacts) return [];

  const problems = [];
  const pubkey = config.plugins?.updater?.pubkey ?? "";

  if (!privateKey?.trim()) {
    problems.push(
      "TAURI_SIGNING_PRIVATE_KEY is empty or unset. Set it to the whole contents of the " +
        "private key file (one base64 line), not a path and not the decoded text.",
    );
  } else if (!decode(privateKey).startsWith("untrusted comment:")) {
    problems.push(
      "TAURI_SIGNING_PRIVATE_KEY does not look like a Tauri key file. Use the file's own " +
        "contents verbatim; `tauri signer generate` already base64-encodes them.",
    );
  }

  if (!pubkey.trim()) {
    problems.push(
      "plugins.updater.pubkey in tauri.conf.json is empty, so the app cannot verify an " +
        "update it downloads. Set it to the contents of the matching .pub file.",
    );
  } else if (!decode(pubkey).startsWith("untrusted comment:")) {
    problems.push(
      "plugins.updater.pubkey is not a Tauri public key. Use the .pub file's contents verbatim.",
    );
  }

  return problems;
}

// Run as a script; imported by `package.mjs` for the same check before a local build.
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const config = JSON.parse(readFileSync(CONFIG, "utf8"));
  const problems = signingProblems({
    privateKey: process.env.TAURI_SIGNING_PRIVATE_KEY,
    config,
  });

  if (problems.length > 0) {
    console.error("Updater signing is not set up:\n");
    for (const problem of problems) console.error(`  - ${problem}\n`);
    console.error(
      "Generate a pair with `npm run tauri signer generate -- -w ~/.tauri/gameyfin.key`,\n" +
        "put the private file's contents in the repository secret and the .pub file's\n" +
        "contents in tauri.conf.json. Both halves must come from the same generate run.\n" +
        "To release without an updater, set bundle.createUpdaterArtifacts to false.",
    );
    process.exit(1);
  }

  console.log("Updater signing looks set up.");
}
