import { Button } from "@/components/ui";
import { Modal, ModalFooter } from "./Modal";

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
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button variant="danger" icon="close" autoFocus onClick={onConfirm}>
          {confirmLabel}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
