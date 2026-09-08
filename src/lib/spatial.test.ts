import { describe, expect, it } from "vitest";

import { pickFirst, pickInDirection, type Box } from "./spatial";

/** A box from top-left corner and size, which reads more like a layout than four edges. */
function box(left: number, top: number, width = 100, height = 40): Box {
  return { left, top, right: left + width, bottom: top + height };
}

describe("pickInDirection", () => {
  it("moves down a row in a grid rather than along it", () => {
    // The case DOM order gets wrong: the next tile in the document is to the right.
    const grid = [
      box(0, 0), box(120, 0), box(240, 0),
      box(0, 60), box(120, 60), box(240, 60),
    ];
    const from = grid[1]; // top middle
    expect(pickInDirection(from, grid, "down")).toBe(4); // directly below
  });

  it("prefers a distant item straight ahead over a near one off to the side", () => {
    const from = box(0, 0);
    const candidates = [
      box(120, 200), // near-ish, but far off the line of travel
      box(0, 400), // further, directly below
    ];
    expect(pickInDirection(from, candidates, "down")).toBe(1);
  });

  it("finds nothing beyond the edge of the layout", () => {
    const from = box(240, 0);
    const row = [box(0, 0), box(120, 0), from];
    expect(pickInDirection(from, row, "right")).toBeNull();
    expect(pickInDirection(from, row, "up")).toBeNull();
  });

  it("never picks the element it started from", () => {
    const from = box(0, 0);
    expect(pickInDirection(from, [from], "right")).toBeNull();
    expect(pickInDirection(from, [from], "down")).toBeNull();
  });

  it("treats a vertical overlap as being in the same row", () => {
    // A tall sidebar item beside a short button: comparing centres alone would make
    // the short one unreachable in both directions.
    const tall = { left: 0, top: 0, right: 60, bottom: 200 };
    const short = { left: 80, top: 90, right: 180, bottom: 120 };
    expect(pickInDirection(tall, [tall, short], "right")).toBe(1);
    expect(pickInDirection(short, [tall, short], "left")).toBe(0);
  });

  it("moves in every direction symmetrically", () => {
    const centre = box(120, 60);
    const cross = [box(120, 0), box(120, 120), box(0, 60), box(240, 60), centre];
    expect(pickInDirection(centre, cross, "up")).toBe(0);
    expect(pickInDirection(centre, cross, "down")).toBe(1);
    expect(pickInDirection(centre, cross, "left")).toBe(2);
    expect(pickInDirection(centre, cross, "right")).toBe(3);
  });

  it("picks the nearer of two equally aligned items", () => {
    const from = box(0, 0);
    const candidates = [box(0, 400), box(0, 100)];
    expect(pickInDirection(from, candidates, "down")).toBe(1);
  });

  it("ignores a sub-pixel stagger between items in a row", () => {
    // Fractional layout on a scaled display should not break a row into two.
    const from = box(0, 0);
    const nudged = box(120, 0.4);
    expect(pickInDirection(from, [from, nudged], "right")).toBe(1);
  });
});

describe("pickFirst", () => {
  it("starts at the top-left, in reading order", () => {
    const grid = [box(120, 60), box(0, 60), box(120, 0), box(0, 0)];
    expect(pickFirst(grid)).toBe(3);
  });

  it("prefers a higher row even when another item is further left", () => {
    expect(pickFirst([box(0, 100), box(500, 0)])).toBe(1);
  });

  it("has nothing to pick from an empty layout", () => {
    expect(pickFirst([])).toBeNull();
  });
});
