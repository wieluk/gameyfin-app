import { useId } from "react";

/** A button, not a hidden checkbox: the controller navigator only reaches elements with a size. */
export function Switch({
  checked,
  onChange,
  disabled = false,
  label,
  id,
  ...aria
}: {
  checked: boolean;
  onChange: (next: boolean) => void | Promise<void>;
  disabled?: boolean;
  /** Shown to the right, for a switch standing alone in a toolbar. */
  label?: string;
  id?: string;
  "aria-label"?: string;
  "aria-describedby"?: string;
}) {
  const control = (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => void onChange(!checked)}
      className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-not-allowed ${
        checked ? "bg-primary" : "bg-default-300"
      }`}
      {...aria}
    >
      <span
        className={`absolute h-3 w-3 rounded-full bg-white shadow-sm transition-transform ${
          checked ? "translate-x-3.5" : "translate-x-0.5"
        }`}
      />
    </button>
  );

  if (!label) return control;
  return (
    <label
      className={`flex shrink-0 items-center gap-2 text-xs transition-colors ${
        disabled
          ? "cursor-not-allowed text-foreground/40"
          : "cursor-pointer text-foreground/70 hover:text-foreground"
      }`}
    >
      {control}
      {label}
    </label>
  );
}

/** An on/off setting: what it is on the left, the switch on the right. */
export function SwitchField({
  label,
  hint,
  checked,
  onChange,
  disabled = false,
  children,
}: {
  label: string;
  hint?: React.ReactNode;
  checked: boolean;
  onChange: (next: boolean) => void | Promise<void>;
  /** Greyed out and inert, for a setting that only applies when another one is on. */
  disabled?: boolean;
  /** Dependent settings, shown on a rail linked to this switch. */
  children?: React.ReactNode;
}) {
  const id = useId();
  if (!children) {
    return (
      <SwitchRow {...{ id, label, hint, checked, onChange, disabled }} />
    );
  }
  return (
    <div>
      <SwitchRow {...{ id, label, hint, checked, onChange, disabled }} />
      <div
        role="group"
        aria-labelledby={id}
        className={`ml-1 mt-2 flex flex-col gap-3 border-l-2 pl-4 transition-colors ${
          checked ? "border-primary/50" : "border-default-200"
        }`}
      >
        {children}
      </div>
    </div>
  );
}

function SwitchRow({
  id,
  label,
  hint,
  checked,
  onChange,
  disabled,
}: {
  id: string;
  label: string;
  hint?: React.ReactNode;
  checked: boolean;
  onChange: (next: boolean) => void | Promise<void>;
  disabled: boolean;
}) {
  return (
    <div className={`flex items-start justify-between gap-4 ${disabled ? "opacity-45" : ""}`}>
      <div className="min-w-0">
        <label
          htmlFor={id}
          className={`block text-xs text-foreground/80 ${
            disabled ? "cursor-not-allowed" : "cursor-pointer"
          }`}
        >
          {label}
        </label>
        {hint && (
          <p id={`${id}-hint`} className="mt-0.5 text-[11px] leading-relaxed text-foreground/45">
            {hint}
          </p>
        )}
      </div>
      <Switch
        id={id}
        checked={checked}
        onChange={onChange}
        disabled={disabled}
        aria-describedby={hint ? `${id}-hint` : undefined}
      />
    </div>
  );
}
