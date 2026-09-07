import { describe, expect, it } from "vitest";
import { formatBytes, formatEta, formatPlaytime, formatSpeed } from "./format";

describe("formatBytes", () => {
  it("renders whole bytes without decimals", () => {
    expect(formatBytes(512)).toBe("512 B");
  });

  it("scales to the largest sensible unit", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
  });

  it("treats non-positive and non-finite input as zero", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(-1)).toBe("0 B");
    expect(formatBytes(Number.NaN)).toBe("0 B");
  });
});

describe("formatSpeed", () => {
  it("suffixes a rate", () => {
    expect(formatSpeed(2 * 1024 ** 2)).toBe("2.0 MB/s");
  });
});

describe("formatPlaytime", () => {
  it("distinguishes never-played from zero-ish", () => {
    expect(formatPlaytime(0)).toBe("Never played");
  });

  it("renders minutes, whole hours and mixed", () => {
    expect(formatPlaytime(45)).toBe("45 min");
    expect(formatPlaytime(120)).toBe("2 h");
    expect(formatPlaytime(135)).toBe("2 h 15 min");
  });
});

describe("formatEta", () => {
  it("returns null when it cannot be estimated", () => {
    expect(formatEta(0, 100, 0)).toBeNull();
    // Already complete.
    expect(formatEta(100, 100, 10)).toBeNull();
  });

  it("scales the unit to the remaining time", () => {
    expect(formatEta(0, 30, 1)).toBe("30s left");
    expect(formatEta(0, 600, 1)).toBe("10 min left");
    expect(formatEta(0, 7200, 1)).toBe("2.0 h left");
  });
});
