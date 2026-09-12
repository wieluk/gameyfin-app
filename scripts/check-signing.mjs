#!/usr/bin/env node
/**
 * Checks the updater signing setup before a release build starts: Tauri signs at the very
 * end, and its own failure names neither the variable nor the fix.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CONFIG = join(ROOT, "src-tauri", "tauri.conf.json");

/** Strict base64 decode: `Buffer.from` skips junk, so a key with text appended looks valid. */
function decode(value) {
  const trimmed = value.trim();
  // Characters outside the alphabet mean this is not base64 at all, usually the decoded
  // key file pasted in place of the one line the generator prints.
  if (!/^[A-Za-z0-9+/=]+$/.test(trimmed)) return { error: "not-base64" };
  // Padding anywhere but the end means the value continues past a complete key.
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(trimmed)) return { error: "appended" };

  const decoded = Buffer.from(trimmed, "base64");
  // A round trip is what proves nothing was skipped; the padding is normalised because a
  // correctly padded value and its stripped form decode identically.
  const unpadded = (text) => text.replace(/=+$/, "");
  if (unpadded(decoded.toString("base64")) !== unpadded(trimmed)) return { error: "appended" };

  return { text: decoded.toString("utf8") };
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
  } else {
    const { text, error } = decode(privateKey);
    if (error === "appended") {
      problems.push(
        "TAURI_SIGNING_PRIVATE_KEY carries something after a complete key: base64 padding " +
          "turns up in the middle of it. Usually the key was pasted twice, or the public " +
          "key follows it. Set the secret to exactly the one line `tauri signer generate` " +
          "printed for Private, with nothing before or after it.",
      );
    } else if (error || !text.startsWith("untrusted comment:")) {
      problems.push(
        "TAURI_SIGNING_PRIVATE_KEY does not look like a Tauri key file. Use the file's own " +
          "contents verbatim; `tauri signer generate` already base64-encodes them.",
      );
    } else if (!text.includes("secret key")) {
      problems.push(
        "TAURI_SIGNING_PRIVATE_KEY holds a public key. The two are easy to swap: the " +
          "private one is the value printed for Private, and its comment says secret key.",
      );
    }
  }

  if (!pubkey.trim()) {
    problems.push(
      "plugins.updater.pubkey in tauri.conf.json is empty, so the app cannot verify an " +
        "update it downloads. Set it to the contents of the matching .pub file.",
    );
  } else {
    const { text: decoded } = decode(pubkey);
    if (!decoded?.startsWith("untrusted comment:")) {
      problems.push(
        "plugins.updater.pubkey is not a Tauri public key. Use the .pub file's contents verbatim.",
      );
    } else if (decoded.includes("secret key")) {
      problems.push(
        "plugins.updater.pubkey holds the private key. Publishing that would let anyone " +
          "sign an update your app accepts: generate a new pair and replace both halves.",
      );
    }
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
