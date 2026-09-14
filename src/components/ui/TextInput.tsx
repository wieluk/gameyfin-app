import { useEffect, useState, type InputHTMLAttributes, type TextareaHTMLAttributes } from "react";

import { Icon, type IconName } from "@/components/Icon";

const FIELD =
  "w-full min-w-0 rounded-lg border border-default-200 bg-content2 text-foreground outline-none transition-colors placeholder:text-foreground/35 focus:border-primary disabled:cursor-not-allowed disabled:opacity-50";

const SIZE = {
  md: "px-3 py-1.5 text-xs",
  lg: "px-3 py-2.5 text-sm",
} as const;

const WITH_ICON = { md: "py-1.5 pl-8 pr-3 text-xs", lg: "py-2.5 pl-9 pr-3 text-sm" } as const;

export function TextInput({
  size = "md",
  mono = false,
  icon,
  className = "",
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, "size"> & {
  size?: keyof typeof SIZE;
  mono?: boolean;
  icon?: IconName;
}) {
  const input = (
    <input
      className={`${FIELD} ${icon ? WITH_ICON[size] : SIZE[size]} ${mono ? "font-mono" : ""} ${
        icon ? "" : className
      }`}
      {...rest}
    />
  );
  if (!icon) return input;
  return (
    <span className={`relative flex min-w-0 ${className}`}>
      <Icon
        name={icon}
        className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-foreground/40"
      />
      {input}
    </span>
  );
}

export function TextArea({
  mono = false,
  className = "",
  ...rest
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { mono?: boolean }) {
  return (
    <textarea
      className={`${FIELD} resize-y px-3 py-1.5 text-xs ${mono ? "font-mono" : ""} ${className}`}
      {...rest}
    />
  );
}

/** A labelled text box that saves when you leave it or press Enter, not on every keystroke. */
export function TextField({
  label,
  value,
  onCommit,
  type = "text",
  placeholder,
  mono,
}: {
  label: string;
  value: string;
  onCommit: (value: string) => void;
  type?: string;
  placeholder?: string;
  mono?: boolean;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);

  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-foreground/55">{label}</span>
      <TextInput
        type={type}
        mono={mono}
        placeholder={placeholder}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => draft !== value && onCommit(draft)}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
    </label>
  );
}
