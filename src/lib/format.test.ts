import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatEta,
  formatPlaytime,
  formatRelative,
  formatSpeed,
} from "./format";

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

describe("formatRelative", () => {
  it("reads as a relative time for recent saves", () => {
    const minutesAgo = new Date(Date.now() - 5 * 60_000).toISOString();
    expect(formatRelative(minutesAgo)).toBe("5 minutes ago");
  });

  it("says just now rather than 0 minutes ago", () => {
    expect(formatRelative(new Date().toISOString())).toBe("just now");
  });

  it("singularises a count of one", () => {
    const anHourAgo = new Date(Date.now() - 3_600_000).toISOString();
    expect(formatRelative(anHourAgo)).toBe("1 hour ago");
  });

  it("falls back to a date once a save is old", () => {
    const longAgo = new Date(Date.now() - 90 * 86_400_000).toISOString();
    expect(formatRelative(longAgo)).toContain("/");
  });

  it("handles a save that was never taken", () => {
    expect(formatRelative(null)).toBe("never");
    expect(formatRelative("not a date")).toBe("never");
  });
});
