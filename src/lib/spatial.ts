/**
 * What a direction should focus next, decided geometrically: DOM order is a single line,
 * and Down in a grid should move a row. Free of the DOM, so the rules test as rectangles.
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
 * How far a candidate sits along the direction of travel, from the edges rather than the
 * centres: by centre, anything level with you is unreachable in both directions.
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

/** Above one, so a distant item straight ahead beats a near one off to the side. */
const CROSS_AXIS_PENALTY = 3;

/**
 * The best candidate in a direction. Scored rather than filtered: "nearest" alone would
 * pick the tile diagonally down-left over the one directly below.
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

/** Where focus goes when there is none: top-left, where a person looks first. */
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
