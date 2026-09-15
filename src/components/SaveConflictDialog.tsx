import { Button } from "@/components/ui";
import { formatBytes, formatRelative } from "@/lib/format";
import { Modal } from "./Modal";
import type { ConflictChoice, SaveVersion } from "@/types";

/**
 * Both machines played since the last sync, so one save has to lose. Shown side by side, since
 * timestamps alone cannot tell two recent saves apart.
 */
export function SaveConflictDialog({
  title,
  localAt,
  remote,
  busy,
  onChoose,
  onCancel,
}: {
  title: string;
  localAt?: string | null;
  remote: SaveVersion;
  busy: boolean;
  onChoose: (choice: ConflictChoice) => void;
  onCancel: () => void;
}) {

  return (
    <Modal label={`Save conflict for ${title}`} role="alertdialog" size="lg" onDismiss={onCancel}>
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold text-foreground">
          Two versions of your {title} save
        </h2>
        <p className="text-xs leading-relaxed text-foreground/60">
          This PC and another machine both played since the last sync. Choose which one to
          keep. Keeping both uploads this PC's save and leaves the other in your history.
        </p>

        <div className="mt-4 grid grid-cols-2 gap-3">
          <SaveSummary heading="This PC" when={localAt} />
          <SaveSummary
            heading={remote.deviceName ?? "Cloud"}
            when={remote.createdAt}
            sizeBytes={remote.sizeBytes}
            platform={remote.platform}
          />
        </div>
      </div>

      <div className="flex flex-wrap justify-end gap-2 border-t border-default-200/60 px-5 py-3">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          Decide later
        </Button>
        <Button onClick={() => onChoose("keep-remote")} disabled={busy}>
          Keep the cloud save
        </Button>
        <Button onClick={() => onChoose("keep-both")} disabled={busy}>
          Keep both
        </Button>
        <Button variant="primary" onClick={() => onChoose("keep-local")} disabled={busy}>
          Keep this PC's save
        </Button>
      </div>
    </Modal>
  );
}

function SaveSummary({
  heading,
  when,
  sizeBytes,
  platform,
}: {
  heading: string;
  when?: string | null;
  sizeBytes?: number;
  platform?: string;
}) {
  return (
    <div className="rounded-xl border border-default-200/60 bg-content2 px-3 py-2">
      <p className="text-xs font-semibold text-foreground">{heading}</p>
      <p className="mt-1 text-[11px] text-foreground/60">Saved {formatRelative(when)}</p>
      {sizeBytes !== undefined && (
        <p className="text-[11px] text-foreground/60">{formatBytes(sizeBytes)}</p>
      )}
      {platform && <p className="text-[11px] text-foreground/40">{platform}</p>}
    </div>
  );
}
