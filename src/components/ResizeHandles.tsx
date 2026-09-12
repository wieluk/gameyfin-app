import { isMockBackend } from "@/lib/backend";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Resize grips for a window without OS decorations, forwarded to `startResizeDragging` so they
 * behave natively on X11 and Wayland.
 */

/** Matches Tauri's `ResizeDirection`. */
type Direction =
  | "North" | "South" | "East" | "West"
  | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";

interface Grip {
  direction: Direction;
  className: string;
}

/** 4px edges, 12px corners, small enough to stay out of the way, large enough to hit. */
const GRIPS: Grip[] = [
  { direction: "North", className: "top-0 left-3 right-3 h-1 cursor-n-resize" },
  { direction: "South", className: "bottom-0 left-3 right-3 h-1 cursor-s-resize" },
  { direction: "West", className: "left-0 top-3 bottom-3 w-1 cursor-w-resize" },
  { direction: "East", className: "right-0 top-3 bottom-3 w-1 cursor-e-resize" },
  { direction: "NorthWest", className: "top-0 left-0 h-3 w-3 cursor-nw-resize" },
  { direction: "NorthEast", className: "top-0 right-0 h-3 w-3 cursor-ne-resize" },
  { direction: "SouthWest", className: "bottom-0 left-0 h-3 w-3 cursor-sw-resize" },
  { direction: "SouthEast", className: "bottom-0 right-0 h-3 w-3 cursor-se-resize" },
];

export function ResizeHandles() {
  // Outside Tauri (browser dev) the window is the browser's; grips would do nothing.
  if (isMockBackend) return null;

  return (
    <>
      {GRIPS.map(({ direction, className }) => (
        <div
          key={direction}
          role="presentation"
          className={`fixed z-50 ${className}`}
          onMouseDown={(e) => {
            // Only a primary-button drag resizes; let other buttons through.
            if (e.button !== 0) return;
            e.preventDefault();
            void getCurrentWindow().startResizeDragging(direction);
          }}
        />
      ))}
    </>
  );
}
