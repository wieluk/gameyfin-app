import type { SelectHTMLAttributes } from "react";

import { Icon } from "@/components/Icon";

/** Native underneath, so keyboards, gamepads and the system popup all keep working. */
const SIZE = {
  sm: "py-1 pl-2.5 pr-7 text-[11px]",
  md: "py-1.5 pl-3 pr-8 text-xs",
} as const;

export function Select({
  size = "md",
  active = false,
  mono = false,
  className = "",
  children,
  ...rest
}: Omit<SelectHTMLAttributes<HTMLSelectElement>, "size"> & {
  size?: keyof typeof SIZE;
  /** A filter holding a value, highlighted so a narrowed list is never a surprise. */
  active?: boolean;
  mono?: boolean;
}) {
  return (
    <span className={`relative inline-flex min-w-0 ${className}`}>
      <select
        className={`w-full min-w-0 cursor-pointer appearance-none truncate rounded-lg border bg-content2 outline-none transition-colors focus:border-primary disabled:cursor-not-allowed disabled:opacity-50 ${
          SIZE[size]
        } ${active ? "border-primary/50 text-primary" : "border-default-200 text-foreground"} ${
          mono ? "font-mono" : ""
        }`}
        {...rest}
      >
        {children}
      </select>
      <Icon
        name="chevron"
        className="pointer-events-none absolute right-2.5 top-1/2 h-3 w-3 -translate-y-1/2 rotate-90 text-foreground/45"
      />
    </span>
  );
}
