import type { ButtonHTMLAttributes } from "react";

import { Icon, type IconName } from "@/components/Icon";

/** Tailwind needs literal class names, so the variants are spelled out rather than built. */
const BASE =
  "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg transition-colors disabled:cursor-not-allowed disabled:opacity-50";

const VARIANT = {
  primary: "bg-primary font-medium text-white hover:bg-primary-600",
  secondary: "border border-default-200 text-foreground/70 hover:bg-default-100",
  danger: "bg-danger font-medium text-white hover:bg-danger-600",
  /** Quiet until hovered, for removals that sit beside ordinary actions. */
  destructive:
    "border border-default-200 text-foreground/60 hover:border-danger/40 hover:bg-danger/10 hover:text-danger",
  ghost: "text-foreground/70 hover:bg-default-100",
} as const;

const PRESSED = "border border-primary/50 bg-primary/10 text-primary";

const SIZE = {
  sm: "px-2.5 py-1 text-[11px]",
  md: "px-3 py-1.5 text-xs",
  lg: "px-4 py-2.5 text-sm",
} as const;

const ICON_SIZE = { sm: "h-3 w-3", md: "h-3.5 w-3.5", lg: "h-4 w-4" } as const;

export type ButtonVariant = keyof typeof VARIANT;
export type ControlSize = keyof typeof SIZE;

type Props = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ControlSize;
  icon?: IconName;
  /** Filled icons read better for play. */
  iconFilled?: boolean;
  /** A toggle chip: highlighted and announced as pressed while on. */
  pressed?: boolean;
};

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  iconFilled,
  pressed,
  className = "",
  children,
  type = "button",
  ...rest
}: Props) {
  const look = pressed ? PRESSED : VARIANT[variant];
  return (
    <button
      type={type}
      aria-pressed={pressed}
      className={`${BASE} ${look} ${SIZE[size]} ${className}`}
      {...rest}
    >
      {icon && <Icon name={icon} className={ICON_SIZE[size]} filled={iconFilled} />}
      {children}
    </button>
  );
}

const SQUARE = { sm: "h-7 w-7", md: "h-8 w-8" } as const;

/** A square button holding only an icon, so it must be named for screen readers. */
export function IconButton({
  icon,
  label,
  size = "md",
  bordered = false,
  iconClassName = "h-4 w-4",
  className = "",
  type = "button",
  ...rest
}: Omit<ButtonHTMLAttributes<HTMLButtonElement>, "aria-label"> & {
  icon: IconName;
  label: string;
  size?: keyof typeof SQUARE;
  bordered?: boolean;
  /** For an icon that turns, like an expand chevron. */
  iconClassName?: string;
}) {
  return (
    <button
      type={type}
      aria-label={label}
      title={rest.title ?? label}
      className={`${BASE} ${SQUARE[size]} ${
        bordered ? "border border-default-200" : ""
      } text-foreground/50 hover:bg-default-100 hover:text-foreground ${className}`}
      {...rest}
    >
      <Icon name={icon} className={iconClassName} />
    </button>
  );
}
