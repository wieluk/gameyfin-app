/**
 * The overlay and panel every dialog shares: a dimmed backdrop, Escape and backdrop-click
 * dismissal, and the focus scope the controller navigator reads.
 */

import { useDismissOnEscape } from "@/lib/useDismiss";

/** Tailwind needs literal class names, so the variants are spelled out rather than built. */
const LAYER = {
  50: "z-50",
  60: "z-[60]",
  /** For a dialog opened from inside another one, which must sit above it. */
  70: "z-[70]",
} as const;

const WIDTH = {
  sm: "max-w-sm",
  md: "max-w-md",
  lg: "max-w-lg",
  "2xl": "max-w-2xl",
} as const;

const PAD = { 4: "p-4", 6: "p-6" } as const;

export function Modal({
  label,
  role = "dialog",
  size = "md",
  layer = 60,
  pad = 6,
  onDismiss,
  dismissOnBackdrop = true,
  fitted = false,
  children,
}: {
  label: string;
  role?: "dialog" | "alertdialog";
  size?: keyof typeof WIDTH;
  layer?: keyof typeof LAYER;
  pad?: keyof typeof PAD;
  /** Also what Escape does, so a dialog cannot be dismissable one way but not the other. */
  onDismiss: () => void;
  /** Off for a prompt that must be answered rather than clicked past. */
  dismissOnBackdrop?: boolean;
  /** Held to the window's height, for a dialog whose body scrolls between a fixed header and footer. */
  fitted?: boolean;
  children: React.ReactNode;
}) {
  useDismissOnEscape(onDismiss);

  return (
    <div
      data-nav-scope
      // Scrolls itself, so a dialog taller than the window can still be read top to bottom.
      className={`fixed inset-0 ${LAYER[layer]} flex overflow-y-auto bg-black/60 ${PAD[pad]} backdrop-blur-sm`}
      role={role}
      aria-modal="true"
      aria-label={label}
      onClick={dismissOnBackdrop ? onDismiss : undefined}
    >
      <div
        // `m-auto` centres a dialog that fits and starts a taller one at the top, where centring would cut it off.
        className={`m-auto w-full ${WIDTH[size]} ${
          fitted ? "flex max-h-full flex-col" : ""
        } overflow-hidden rounded-2xl border border-default-200 bg-content1 shadow-2xl`}
        onClick={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}

/** A dialog's title, and the sentence under it when there is one. */
export function ModalHeader({
  title,
  description,
}: {
  title: string;
  description?: React.ReactNode;
}) {
  return (
    <div className="px-5 py-4">
      <h2 className="text-sm font-semibold">{title}</h2>
      {description && (
        <p className="mt-2 text-xs leading-relaxed text-foreground/60">{description}</p>
      )}
    </div>
  );
}

/** The row of actions at the bottom of a dialog. */
export function ModalFooter({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
      {children}
    </div>
  );
}
