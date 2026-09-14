import { Alert } from "@/components/Alert";
import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui";
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
      <div className="mt-1 flex items-start gap-2">
        <code className="min-w-0 flex-1 break-all rounded-lg border border-default-200 bg-content2 px-3 py-1.5 font-mono text-[11px] text-foreground/70">
          {path ?? "…"}
        </code>
        <Button icon="folder" disabled={!path} onClick={() => path && void backend.openFolder(path)}>
          Open folder
        </Button>
        {action && (
          <Button
            variant={action.danger ? "destructive" : "secondary"}
            onClick={() => void action.onClick()}
          >
            {action.label}
          </Button>
        )}
      </div>
      <p className="mt-1 text-[11px] text-foreground/45">{hint}</p>
    </div>
  );
}
