/**
 * What a controller's buttons mean. Everything moves real DOM focus rather than a selection
 * of its own, so keyboard users get the same behaviour and scrolling takes care of itself.
 * Movement between elements is geometric: see `spatial.ts`.
 */

import { pickFirst, pickInDirection, type Box, type Direction } from "./spatial";

/** The logical buttons the Rust side sends. Mirrors `gamepad::Button`. */
export type GamepadButton =
  | "south"
  | "east"
  | "west"
  | "north"
  | "left-bumper"
  | "right-bumper"
  | "left-trigger"
  | "right-trigger"
  | "select"
  | "start"
  | "up"
  | "down"
  | "left"
  | "right";

export interface ButtonEvent {
  button: GamepadButton;
  /** True when the pad has been held rather than freshly pressed. */
  repeat: boolean;
}

export interface AxisEvent {
  rightX: number;
  rightY: number;
}

export interface ConnectionEvent {
  connected: boolean;
  name: string | null;
}

/** Elements that can take focus, in document order. */
const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type=hidden])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

/**
 * Whether an element can be reached now. A zero-sized rectangle catches what a CSS check
 * misses: a collapsed sidebar's labels, anything inside a hidden ancestor.
 */
function isReachable(element: Element): boolean {
  const rect = element.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return false;
  if (element.getAttribute("aria-hidden") === "true") return false;
  return !element.closest("[inert],[aria-hidden='true']");
}

/**
 * What the user can move between. Confined to an open dialog, or a direction press would
 * focus the library behind it and activate something nobody can see.
 */
export function navigableElements(): HTMLElement[] {
  const scope = document.querySelector<HTMLElement>("[data-nav-scope]") ?? document;
  return Array.from(scope.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(isReachable);
}

function boxOf(element: HTMLElement): Box {
  const { left, top, right, bottom } = element.getBoundingClientRect();
  return { left, top, right, bottom };
}

/** Move focus one step in a direction. Returns whether anything moved. */
export function moveFocus(direction: Direction): boolean {
  const elements = navigableElements();
  if (elements.length === 0) return false;

  const active = document.activeElement;
  const current =
    active instanceof HTMLElement && elements.includes(active) ? active : null;

  if (!current) {
    // Nothing focused yet, so a direction press means "start here".
    const first = pickFirst(elements.map(boxOf));
    if (first === null) return false;
    focus(elements[first]);
    return true;
  }

  const boxes = elements.map(boxOf);
  const index = pickInDirection(boxOf(current), boxes, direction);
  if (index === null) return false;
  focus(elements[index]);
  return true;
}

/** Focus an element and bring it into view. Nearest, not centred: centring makes a grid
 * lurch under the user even when the target was already on screen. */
function focus(element: HTMLElement) {
  document.documentElement.dataset.padFocus = "true";
  element.focus({ preventScroll: true });
  element.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
}

/** Drop the controller focus ring once a pointer is in use again. See `styles.css`. */
export function clearPadFocus() {
  delete document.documentElement.dataset.padFocus;
}

export function activateFocused(): boolean {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) return false;

  // In a text field, leave focus where it is: there is no on-screen keyboard to offer.
  if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {
    return false;
  }
  active.click();
  return true;
}

/**
 * Back: close the topmost dialog, or leave the field being edited. Dispatched as Escape,
 * which every dialog already handles for keyboard users.
 */
export function goBack(): boolean {
  const active = document.activeElement;
  if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {
    active.blur();
    return true;
  }
  document.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
  );
  return true;
}

/** The scrollable element containing the focus, or the document. */
function scrollContainer(): HTMLElement | null {
  let node = document.activeElement as HTMLElement | null;
  while (node && node !== document.body) {
    const style = getComputedStyle(node);
    const scrolls = /auto|scroll/.test(style.overflowY);
    if (scrolls && node.scrollHeight > node.clientHeight) return node;
    node = node.parentElement;
  }
  // Panes scroll internally, so use the largest one on screen, not the document.
  const panes = Array.from(document.querySelectorAll<HTMLElement>(".overflow-y-auto"));
  return panes.find((pane) => pane.scrollHeight > pane.clientHeight) ?? null;
}

/** Scroll by a fraction of a page. Used by the right stick and the triggers. */
export function scrollBy(amount: number) {
  const container = scrollContainer();
  if (!container) return;
  container.scrollBy({ top: amount, behavior: "auto" });
}

/** How fast the right stick scrolls, in pixels per poll at full deflection. */
const SCROLL_SPEED = 22;

export function scrollByStick(event: AxisEvent) {
  // The stick reports up as positive; scrolling down is a positive offset.
  const amount = -event.rightY * SCROLL_SPEED;
  if (Math.abs(amount) < 1) return;
  scrollBy(amount);
}

/** One page, near enough, for the triggers. */
export function scrollPage(direction: 1 | -1) {
  const container = scrollContainer();
  const height = container?.clientHeight ?? window.innerHeight;
  scrollBy(direction * height * 0.9);
}

/** The tabs the bumpers cycle between, in order. */
export const TAB_ORDER = ["/", "/downloads", "/installed", "/saves", "/settings"] as const;

/** The route a bumper press should go to, given where we are. */
export function nextTab(current: string, step: 1 | -1): string {
  // An unknown route starts from the library rather than refusing to move.
  const index = TAB_ORDER.indexOf(current as (typeof TAB_ORDER)[number]);
  const from = index === -1 ? 0 : index;
  const next = (from + step + TAB_ORDER.length) % TAB_ORDER.length;
  return TAB_ORDER[next];
}

/** What each button is for, shown in the overlay. */
export const BUTTON_HELP: ReadonlyArray<{ keys: string; action: string }> = [
  { keys: "D-pad / Left stick", action: "Move between items" },
  { keys: "A", action: "Select" },
  { keys: "B", action: "Back, or close" },
  { keys: "Y", action: "Refresh" },
  { keys: "LB / RB", action: "Previous / next tab" },
  { keys: "LT / RT", action: "Page up / down" },
  { keys: "Right stick", action: "Scroll" },
  { keys: "Start", action: "Show these controls" },
];
