import { formatBytes, formatRelative } from "@/lib/format";
import { useDismissOnEscape } from "@/lib/useDismiss";
import type { ConflictChoice, SaveVersion } from "@/types";

/**
 * Both machines played since the last sync, so one save has to lose. Showing them side by
 * side is the only way the user can tell which is which; timestamps alone are not enough
 * when both are recent.
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
  useDismissOnEscape(onCancel);

  return (
    <div
      data-nav-scope
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-6 backdrop-blur-sm"
      role="alertdialog"
      aria-modal="true"
      aria-label={`Save conflict for ${title}`}
      onClick={onCancel}
    >
      <div
        className="w-full max-w-lg overflow-hidden rounded-2xl border border-default-200 bg-content1 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
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
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-lg px-3 py-1.5 text-xs text-foreground/70 hover:bg-default-100 disabled:opacity-50"
          >
            Decide later
          </button>
          <button
            type="button"
            onClick={() => onChoose("keep-remote")}
            disabled={busy}
            className="rounded-lg bg-default-100 px-3 py-1.5 text-xs font-medium hover:bg-default-200 disabled:opacity-50"
          >
            Keep the cloud save
          </button>
          <button
            type="button"
            onClick={() => onChoose("keep-both")}
            disabled={busy}
            className="rounded-lg bg-default-100 px-3 py-1.5 text-xs font-medium hover:bg-default-200 disabled:opacity-50"
          >
            Keep both
          </button>
          <button
            type="button"
            onClick={() => onChoose("keep-local")}
            disabled={busy}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white hover:bg-primary/90 disabled:opacity-50"
          >
            Keep this PC's save
          </button>
        </div>
      </div>
    </div>
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
