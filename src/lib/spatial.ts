/**
 * Choosing what a direction on a d-pad should focus next.
 *
 * Focus order in the DOM is a single line, which is the wrong shape for a library grid:
 * pressing Down in a grid of six-per-row should move down a row, not to the next tile.
 * So movement is geometric, decided from where things actually are on screen.
 *
 * Kept free of the DOM so the rules can be tested against plain rectangles. The caller
 * measures elements and applies the result.
 */

export type Direction = "up" | "down" | "left" | "right";

/** The part of a `DOMRect` this needs. */
export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

interface Point {
  x: number;
  y: number;
}

function centre(box: Box): Point {
  return { x: (box.left + box.right) / 2, y: (box.top + box.bottom) / 2 };
}

/**
 * How far a candidate sits along the direction of travel, or null if it is not in it.
 *
 * Measured from the *edges* rather than the centres, so a tall sidebar item beside a short
 * one is still reachable: comparing centres alone makes anything whose middle is level with
 * yours unreachable in both directions.
 */
function distanceAhead(from: Box, to: Box, direction: Direction): number | null {
  // Tolerance absorbs sub-pixel rounding on scaled displays.
  const slack = 1;
  switch (direction) {
    case "right":
      return to.left >= from.right - slack ? to.left - from.right : null;
    case "left":
      return from.left >= to.right - slack ? from.left - to.right : null;
    case "down":
      return to.top >= from.bottom - slack ? to.top - from.bottom : null;
    case "up":
      return from.top >= to.bottom - slack ? from.top - to.bottom : null;
  }
}

/** How far a candidate is off the line of travel. */
function crossAxisOffset(from: Box, to: Box, direction: Direction): number {
  const a = centre(from);
  const b = centre(to);
  const horizontal = direction === "left" || direction === "right";
  if (horizontal) {
    // Vertical overlap counts as aligned, so a row of differently sized buttons feels like a row.
    if (to.bottom > from.top && to.top < from.bottom) return 0;
    return Math.abs(b.y - a.y);
  }
  if (to.right > from.left && to.left < from.right) return 0;
  return Math.abs(b.x - a.x);
}

/**
 * How much worse being off-axis is than being far away.
 *
 * Above one, so a distant item directly ahead beats a near one off to the side. This is
 * what stops Down in a grid drifting diagonally across the screen.
 */
const CROSS_AXIS_PENALTY = 3;

/**
 * Pick the index of the best candidate in a direction, or null when there is none.
 *
 * Candidates are scored, not merely filtered, because more than one is usually available
 * and "nearest" alone picks badly: in a grid it would choose the tile diagonally down-left
 * over the one directly below.
 */
export function pickInDirection(
  from: Box,
  candidates: readonly Box[],
  direction: Direction,
): number | null {
  let best: number | null = null;
  let bestScore = Infinity;

  candidates.forEach((candidate, index) => {
    const ahead = distanceAhead(from, candidate, direction);
    if (ahead === null) return;

    const score = ahead + CROSS_AXIS_PENALTY * crossAxisOffset(from, candidate, direction);
    if (score < bestScore) {
      bestScore = score;
      best = index;
    }
  });

  return best;
}

/**
 * Where focus should go when there is none yet.
 *
 * Top-left in reading order, which is where a person looks first.
 */
export function pickFirst(candidates: readonly Box[]): number | null {
  let best: number | null = null;
  let bestScore = Infinity;

  candidates.forEach((candidate, index) => {
    // Rows dominate: anything higher up wins regardless of how far left the other is.
    const score = candidate.top * 1000 + candidate.left;
    if (score < bestScore) {
      bestScore = score;
      best = index;
    }
  });

  return best;
}
