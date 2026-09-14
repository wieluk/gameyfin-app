import type { InputHTMLAttributes } from "react";

import { Icon } from "@/components/Icon";

type Props = Omit<InputHTMLAttributes<HTMLInputElement>, "type">;

/** For picking several from a list. A single on/off choice is a `Switch`. */
export function Checkbox({ className = "", ...rest }: Props) {
  return (
    <span className={`relative inline-flex h-3.5 w-3.5 shrink-0 ${className}`}>
      <input
        type="checkbox"
        className="peer h-3.5 w-3.5 cursor-pointer appearance-none rounded border border-default-300 bg-content2 transition-colors checked:border-primary checked:bg-primary disabled:cursor-not-allowed disabled:opacity-50"
        {...rest}
      />
      <Icon
        name="check"
        className="pointer-events-none absolute inset-0 hidden h-3.5 w-3.5 text-white peer-checked:block"
      />
    </span>
  );
}

export function Radio({ className = "", ...rest }: Props) {
  return (
    <input
      type="radio"
      className={`h-3.5 w-3.5 shrink-0 cursor-pointer appearance-none rounded-full border border-default-300 bg-content2 transition-all checked:border-4 checked:border-primary disabled:cursor-not-allowed disabled:opacity-50 ${className}`}
      {...rest}
    />
  );
}
