import { describe, expect, it } from "vitest";

import { labelFor, parseLimit, toMegabytes } from "./SpeedLimit";

describe("parseLimit", () => {
  it("reads a decimal cap in either notation", () => {
    expect(parseLimit("0.5")).toBe(512);
    expect(parseLimit("0,5")).toBe(512);
    expect(parseLimit("0.25")).toBe(256);
    expect(parseLimit("3")).toBe(3072);
    expect(parseLimit(" 1.5 ")).toBe(1536);
  });

  it("never turns a typed cap into unlimited", () => {
    // Zero is what the backend reads as unlimited, so a positive request must not reach
    // it by rounding: this is the bug where typing a limit removed the limit.
    expect(parseLimit("0.0001")).toBe(1);
  });

  it("leaves the setting alone on anything unusable", () => {
    expect(parseLimit("")).toBeNull();
    expect(parseLimit("   ")).toBeNull();
    expect(parseLimit("fast")).toBeNull();
    expect(parseLimit("0")).toBeNull();
    expect(parseLimit("-2")).toBeNull();
  });
});

describe("toMegabytes", () => {
  it("round-trips the values the field accepts", () => {
    for (const typed of ["0.25", "0.5", "1.5", "3", "12.75"]) {
      expect(toMegabytes(parseLimit(typed) as number)).toBe(String(Number(typed)));
    }
  });
});

describe("labelFor", () => {
  it("shows sub-1 MB/s caps in KB/s", () => {
    expect(labelFor(768)).toBe("768 KB/s");
    expect(labelFor(1536)).toBe("1.5 MB/s");
  });
});
