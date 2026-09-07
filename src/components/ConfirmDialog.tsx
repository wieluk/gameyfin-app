import { Icon } from "./Icon";

/**
 * Confirmation for an action that destroys files.
 *
 * Deleting a download or an install removes gigabytes that took a long time to fetch, so
 * neither should happen on a single stray click.
 */
export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: React.ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-6 backdrop-blur-sm"
      role="alertdialog"
      aria-modal="true"
      aria-label={title}
      onClick={onCancel}
    >
      <div
        className="w-full max-w-sm overflow-hidden rounded-2xl border border-default-200 bg-content1 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-5 py-4">
          <h2 className="mb-1 text-sm font-semibold text-foreground">{title}</h2>
          <div className="text-xs leading-relaxed text-foreground/60">{body}</div>
        </div>
        <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
          >
            Cancel
          </button>
          <button
            type="button"
            autoFocus
            onClick={onConfirm}
            className="flex items-center gap-1.5 rounded-lg bg-danger px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-danger-600"
          >
            <Icon name="close" className="h-3 w-3" />
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
