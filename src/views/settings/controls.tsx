import { useEffect, useState } from "react";

import { Alert } from "@/components/Alert";
import { Icon } from "@/components/Icon";
import { backend } from "@/lib/backend";
import { useSettingsUpdate } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import type { SettingsPatch } from "@/types";

/**
 * Save a settings change and keep whatever went wrong. Every control in Settings needs
 * both, and a dropped rejection makes a refused value look like a no-op.
 */
export function useSettingSaver() {
  const update = useSettingsUpdate();
  const action = useAction();
  return {
    save: (patch: SettingsPatch) => action.run(() => update(patch)),
    busy: action.busy,
    error: action.error,
  };
}

/** The error line a settings section shows under its controls. */
export function SaveError({ error }: { error: string | null }) {
  return error ? <Alert inline>{error}</Alert> : null;
}

export function SmallButton({
  children,
  onClick,
  danger,
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={`rounded-lg border border-default-200 px-2.5 py-1 text-[11px] transition-colors ${
        danger
          ? "text-foreground/60 hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
          : "text-foreground/70 hover:bg-default-100"
      } disabled:opacity-50`}
    >
      {children}
    </button>
  );
}

/** A labelled checkbox with an optional explanation underneath. */
export function Check({
  label,
  hint,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (next: boolean) => void | Promise<void>;
  /** Greyed out and inert, for a setting that only applies when another one is on. */
  disabled?: boolean;
}) {
  return (
    <div className={disabled ? "opacity-45" : undefined}>
      <label
        className={`flex items-start gap-2 text-xs text-foreground/80 ${
          disabled ? "cursor-not-allowed" : "cursor-pointer"
        }`}
      >
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(e) => void onChange(e.target.checked)}
          className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-primary"
        />
        <span>{label}</span>
      </label>
      {hint && <p className="mt-0.5 pl-[1.375rem] text-[11px] leading-relaxed text-foreground/45">{hint}</p>}
    </div>
  );
}

/** A labelled text box that saves when you leave it or press Enter, not on every keystroke. */
export function Field({
  label,
  value,
  onCommit,
  type = "text",
  placeholder,
}: {
  label: string;
  value: string;
  onCommit: (value: string) => void;
  type?: string;
  placeholder?: string;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);

  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11px] text-foreground/60">{label}</span>
      <input
        type={type}
        className="rounded-lg border border-default-200 bg-content1 px-3 py-1.5 text-xs"
        placeholder={placeholder}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => draft !== value && onCommit(draft)}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
    </label>
  );
}

export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-xl border border-default-200 bg-content1 p-4">
      <h2 className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-foreground/45">
        <Icon name="settings" className="h-3.5 w-3.5" />
        {title}
      </h2>
      <div className="flex flex-col gap-2">{children}</div>
    </section>
  );
}

export function Row({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "good" | "bad";
}) {
  const colour =
    tone === "good" ? "text-success" : tone === "bad" ? "text-danger" : "text-foreground";
  return (
    <div className="flex items-baseline justify-between gap-4 text-sm">
      <span className="shrink-0 text-foreground/55">{label}</span>
      <span className={`truncate ${colour}`} title={value}>
        {value}
      </span>
    </div>
  );
}

/** A filesystem path with an Open button, and optionally a destructive action. */
export function PathRow({
  label,
  hint,
  path,
  action,
}: {
  label: string;
  hint: string;
  path: string | undefined;
  action?: { label: string; danger?: boolean; onClick: () => void | Promise<void> };
}) {
  return (
    <div>
      <p className="text-xs text-foreground/55">{label}</p>
      <div className="mt-1 flex gap-2">
        <code className="min-w-0 flex-1 break-all rounded-lg border border-default-200 bg-content2 px-3 py-2 font-mono text-[11px] text-foreground/70">
          {path ?? "…"}
        </code>
        <button
          type="button"
          disabled={!path}
          onClick={() => path && void backend.openFolder(path)}
          className="shrink-0 self-start rounded-lg border border-default-200 px-3 py-2 text-xs text-foreground/70 transition-colors hover:bg-default-100 disabled:opacity-50"
        >
          Open folder
        </button>
        {action && (
          <button
            type="button"
            onClick={() => void action.onClick()}
            className={`shrink-0 self-start rounded-lg border px-3 py-2 text-xs transition-colors ${
              action.danger
                ? "border-default-200 text-foreground/60 hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
                : "border-default-200 text-foreground/70 hover:bg-default-100"
            }`}
          >
            {action.label}
          </button>
        )}
      </div>
      <p className="mt-1 text-[11px] text-foreground/45">{hint}</p>
    </div>
  );
}
