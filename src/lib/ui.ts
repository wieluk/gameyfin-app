/**
 * Class strings shared by more than a couple of places. Written out in full, since Tailwind
 * scans for literal strings.
 */

/** The quiet explanatory line under a setting or a field. */
export const HINT = "text-[11px] leading-relaxed text-foreground/45";

/** A text or number field. */
export const INPUT =
  "rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary";

/** The default button: an outlined, low-emphasis action. */
export const BUTTON =
  "rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100";

/** The same button where the action can be unavailable. */
export const BUTTON_MAYBE_DISABLED = `${BUTTON} disabled:opacity-50`;

/** The scrolling body of a full-height panel. */
export const PANEL_BODY = "min-h-0 flex-1 overflow-y-auto px-6 py-5";

/** The high-emphasis action in a dialog or on a row. */
export const BUTTON_PRIMARY =
  "rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600 disabled:opacity-50";

/** An action that deletes or removes something. */
export const BUTTON_DANGER =
  "rounded-lg border border-danger/40 px-3 py-1.5 text-xs text-danger transition-colors hover:bg-danger/10 disabled:opacity-50";

/** A compact select, for the pickers that sit inside a row. */
export const SELECT_SM =
  "rounded-lg border border-default-200 bg-content2 px-2 py-1 text-[11px] outline-none focus:border-primary disabled:opacity-50";
