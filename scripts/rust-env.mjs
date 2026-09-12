/**
 * Prefers a user-local rustup when the configured `RUSTUP_HOME` is read-only, so cargo
 * doesn't fail trying to install a toolchain into a directory it cannot write.
 */

import { accessSync, constants, existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join } from "node:path";

function writable(path) {
  try {
    accessSync(path, constants.W_OK);
    return true;
  } catch {
    return false;
  }
}

export function rustEnv(base = process.env) {
  const env = { ...base };
  const configured = env.RUSTUP_HOME;

  // Nothing to fix if rustup is unset (cargo uses defaults) or already writable.
  if (!configured || writable(configured)) return env;

  const localRustup = join(homedir(), ".rustup");
  const localCargo = join(homedir(), ".cargo");
  if (!existsSync(localRustup)) return env;

  env.RUSTUP_HOME = localRustup;
  env.CARGO_HOME = localCargo;
  env.PATH = `${join(localCargo, "bin")}${delimiter}${env.PATH ?? ""}`;
  console.log(`Using user-local rustup at ${localRustup} (${configured} is not writable)`);
  return env;
}
