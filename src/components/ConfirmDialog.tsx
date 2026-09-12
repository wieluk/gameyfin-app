import { Icon } from "./Icon";
import { Modal, ModalFooter } from "./Modal";
import { BUTTON } from "@/lib/ui";

/** Confirmation for actions that destroy files: neither should happen on one stray click. */
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
    <Modal label={title} role="alertdialog" size="sm" onDismiss={onCancel}>
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold text-foreground">{title}</h2>
        <div className="text-xs leading-relaxed text-foreground/60">{body}</div>
      </div>
      <ModalFooter>
        <button
          type="button"
          onClick={onCancel}
          className={BUTTON}
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
      </ModalFooter>
    </Modal>
  );
}
