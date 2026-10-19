/**
 * Class strings shared by more than a couple of places. Controls live in `components/ui`.
 */

/** The quiet explanatory line under a setting or a field. */
export const HINT = "text-[0.6875rem] leading-relaxed text-foreground/45";

/** The scrolling body of a full-height panel. */
export const PANEL_BODY = "min-h-0 flex-1 overflow-y-auto px-6 py-5";

/** A gradient keyed off the title, so a game without art still looks placed and stays recognisable. */
export function titleGradient(title: string): string {
  const hue = [...title].reduce((acc, c) => (acc * 31 + c.charCodeAt(0)) % 360, 7);
  return `linear-gradient(145deg, hsl(${hue} 45% 28%), hsl(${(hue + 40) % 360} 40% 16%))`;
}
