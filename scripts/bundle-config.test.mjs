// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const config = JSON.parse(
  readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
);

describe("rpm dependencies", () => {
  it("are bare package names", () => {
    // Tauri writes each entry whole as a package name, so a version constraint breaks dnf.
    const { depends = [], recommends = [] } = config.bundle.linux.rpm;
    for (const dep of [...depends, ...recommends]) {
      expect(dep, dep).toMatch(/^[\w.+-]+$/);
    }
  });
});
