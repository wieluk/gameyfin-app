import { useEffect, useRef, useState } from "react";

import { IconButton } from "@/components/ui";
import { useDismissOnEscape } from "@/lib/useDismiss";

export type MenuItem = {
  label: string;
  onSelect: () => void;
  /** Drawn in red and set apart below a line. */
  destructive?: boolean;
  disabled?: boolean;
};

/** The "⋯" button for actions used rarely enough that they would crowd a toolbar. */
export function OverflowMenu({ label, items }: { label: string; items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative">
      <IconButton
        icon="more"
        label={label}
        bordered
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      />
      {open && <MenuPanel label={label} items={items} onClose={() => setOpen(false)} />}
    </div>
  );
}

function MenuPanel({
  label,
  items,
  onClose,
}: {
  label: string;
  items: MenuItem[];
  onClose: () => void;
}) {
  const panel = useRef<HTMLDivElement>(null);
  useDismissOnEscape(onClose);

  useEffect(() => {
    // Focused at once, so a controller or keyboard lands inside the menu it just opened.
    panel.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    function outside(event: MouseEvent) {
      const holder = panel.current?.parentElement;
      if (holder && !holder.contains(event.target as Node)) onClose();
    }
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, [onClose]);

  const plain = items.filter((item) => !item.destructive);
  const destructive = items.filter((item) => item.destructive);

  return (
    <div
      ref={panel}
      role="menu"
      aria-label={label}
      data-nav-scope
      className="absolute right-0 top-full z-[65] mt-1 w-56 overflow-hidden rounded-xl border border-default-200 bg-content1 py-1 shadow-2xl"
    >
      {plain.map((item) => (
        <Entry key={item.label} item={item} onClose={onClose} />
      ))}
      {plain.length > 0 && destructive.length > 0 && (
        <div className="my-1 border-t border-default-200/60" />
      )}
      {destructive.map((item) => (
        <Entry key={item.label} item={item} onClose={onClose} />
      ))}
    </div>
  );
}

function Entry({ item, onClose }: { item: MenuItem; onClose: () => void }) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={item.disabled}
      onClick={() => {
        onClose();
        item.onSelect();
      }}
      className={`block w-full px-3 py-2 text-left text-xs transition-colors hover:bg-default-100 disabled:cursor-not-allowed disabled:opacity-50 ${
        item.destructive ? "text-danger" : "text-foreground"
      }`}
    >
      {item.label}
    </button>
  );
}
