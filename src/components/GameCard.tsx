import { Icon } from "./Icon";
import { primaryAction } from "@/lib/actions";
import {
  formatBytes,
  formatEta,
  formatInstallProgress,
  formatPlaytime,
  formatSpeed,
} from "@/lib/format";
import type { LibraryEntry, TransferProgress } from "@/types";

/** A library tile; 2:3 cover art so the grid reads as a shelf, not a table. */
/** What to call a failure, by the step that failed. */
const STAGE_FAILURE = {
  download: "Download failed",
  extract: "Extract failed",
  install: "Install failed",
  launch: "Launch failed",
} as const;

export function GameCard({
  entry,
  onPrimaryAction,
  onOpen,
}: {
  entry: LibraryEntry;
  onPrimaryAction: (entry: LibraryEntry) => void;
  onOpen: (entry: LibraryEntry) => void;
}) {
  const { game } = entry;
  const coverUrl = entry.coverUrl ?? null;
  const action = primaryAction(entry.state);

  return (
    // `content-visibility` skips offscreen cards; keeps scrolling smooth in the CPU-composited Flatpak webview.
    <article className="group relative flex flex-col gap-2 [contain-intrinsic-size:auto_320px] [content-visibility:auto]">
      <div
        role="button"
        tabIndex={0}
        onClick={() => onOpen(entry)}
        onKeyDown={(e) => {
          // Only the tile itself: a press on the action button inside it is that button's.
          if (e.target !== e.currentTarget) return;
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onOpen(entry);
          }
        }}
        className="relative aspect-[2/3] cursor-pointer overflow-hidden rounded-xl bg-default-200 shadow-sm ring-1 ring-default-300/40 transition-transform duration-200 group-hover:-translate-y-1 group-hover:shadow-xl focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      >
        {coverUrl ? (
          <img src={coverUrl} alt="" className="h-full w-full object-cover" loading="lazy" />
        ) : (
          <PlaceholderArt title={game.title} />
        )}

        <StateBadge entry={entry} />

        {/* `focus-within` as well as hover: a controller focuses the button without a pointer. */}
        <div className="absolute inset-0 flex items-end justify-center bg-gradient-to-t from-black/80 via-black/10 to-transparent p-3 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
          <button
            type="button"
            disabled={action.disabled}
            onClick={(e) => {
              // The tile opens the detail view; this button must not trigger it too.
              e.stopPropagation();
              onPrimaryAction(entry);
            }}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-primary px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-60"
          >
            <Icon name={action.icon} className="h-4 w-4" filled={action.icon === "play"} />
            {action.label}
          </button>
        </div>
      </div>

      <div className="min-w-0 cursor-pointer" onClick={() => onOpen(entry)}>
        <h3 className="truncate text-sm font-medium text-foreground" title={game.title}>
          {game.title}
        </h3>
        <p className="truncate text-xs text-foreground/50">{subtitle(entry)}</p>
      </div>
    </article>
  );
}

/** Deterministic gradient keyed off the title, so a coverless game still looks placed. */
function PlaceholderArt({ title }: { title: string }) {
  const hue = [...title].reduce((acc, c) => (acc * 31 + c.charCodeAt(0)) % 360, 7);
  return (
    <div
      className="flex h-full w-full items-center justify-center p-3"
      style={{
        background: `linear-gradient(145deg, hsl(${hue} 45% 28%), hsl(${(hue + 40) % 360} 40% 16%))`,
      }}
    >
      <span className="line-clamp-3 text-center text-sm font-semibold text-white/85">{title}</span>
    </div>
  );
}

function InstallingBadge({ progress }: { progress?: TransferProgress }) {
  const percent =
    progress && progress.totalBytes > 0 ? (progress.receivedBytes / progress.totalBytes) * 100 : undefined;
  return (
    <div className="absolute inset-x-0 bottom-0 bg-black/75 px-2.5 py-1.5 backdrop-blur-sm">
      <div className="mb-1 flex justify-between text-[10px] text-white/80">
        <span>Installing</span>
        {progress ? <span>{formatInstallProgress(progress)}</span> : null}
      </div>
      <div className="h-1 overflow-hidden rounded-full bg-white/20">
        <div
          className={`h-full rounded-full bg-warning transition-[width] ${percent === undefined ? "animate-pulse" : ""}`}
          style={{ width: `${percent ?? 100}%` }}
        />
      </div>
    </div>
  );
}

function StateBadge({ entry }: { entry: LibraryEntry }) {
  const { state } = entry;

  if (state.kind === "downloading") {
    const percent = state.totalBytes > 0 ? (state.receivedBytes / state.totalBytes) * 100 : 0;
    const eta = formatEta(state.receivedBytes, state.totalBytes, state.bytesPerSecond);
    return (
      <div className="absolute inset-x-0 bottom-0 bg-black/75 px-2.5 py-1.5 backdrop-blur-sm">
        <div className="mb-1 flex justify-between text-[10px] text-white/80">
          <span>{formatSpeed(state.bytesPerSecond)}</span>
          {eta ? <span>{eta}</span> : null}
        </div>
        <div className="h-1 overflow-hidden rounded-full bg-white/20">
          <div className="h-full rounded-full bg-primary transition-[width]" style={{ width: `${percent}%` }} />
        </div>
      </div>
    );
  }

  if (state.kind === "installing") return <InstallingBadge progress={state.progress} />;

  if (state.kind === "downloaded") {
    return (
      <span className="absolute left-2 top-2 rounded-md bg-warning/90 px-2 py-0.5 text-[10px] font-medium text-white shadow">
        Ready to install
      </span>
    );
  }

  if (state.kind === "failed") {
    return (
      <div
        className="absolute inset-x-0 bottom-0 bg-danger/85 px-2.5 py-1.5 backdrop-blur-sm"
        title={state.message}
      >
        <p className="truncate text-[10px] font-medium text-white">
          {STAGE_FAILURE[state.stage]}
        </p>
        <p className="truncate text-[10px] text-white/80">{state.message}</p>
      </div>
    );
  }

  if (state.kind === "running") {
    return (
      <span className="absolute left-2 top-2 flex items-center gap-1.5 rounded-md bg-success px-2 py-0.5 text-[10px] font-semibold text-white shadow">
        <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-white" />
        Running
      </span>
    );
  }

  if (state.kind === "installed") {
    const busy = state.busy;
    const download = busy?.kind === "preparing" ? busy.progress : undefined;
    if (download && download.totalBytes > 0) {
      return (
        <div
          className="absolute inset-x-0 bottom-0 bg-black/75 px-2.5 py-1.5 backdrop-blur-sm"
          title={busy?.kind === "preparing" ? busy.message : undefined}
        >
          <div className="mb-1 flex justify-between text-[10px] text-white/80">
            <span>Setting up</span>
            <span>{formatSpeed(download.bytesPerSecond)}</span>
          </div>
          <div className="h-1 overflow-hidden rounded-full bg-white/20">
            <div
              className="h-full rounded-full bg-primary transition-[width]"
              style={{ width: `${(download.receivedBytes / download.totalBytes) * 100}%` }}
            />
          </div>
        </div>
      );
    }
    if (busy?.kind === "installing" && busy.progress) {
      return <InstallingBadge progress={busy.progress} />;
    }
    if (busy) {
      return (
        <span
          title={busy.kind === "installing" ? undefined : busy.message}
          className={`absolute left-2 top-2 rounded-md px-2 py-0.5 text-[10px] font-medium text-white backdrop-blur-sm ${
            busy.kind === "failed" ? "bg-danger/85" : "bg-black/75"
          }`}
        >
          {busy.kind === "preparing"
            ? "Setting up…"
            : busy.kind === "installing"
              ? "Installing…"
              : "Setup failed"}
        </span>
      );
    }
    return (
      <span className="absolute left-2 top-2 rounded-md bg-black/65 px-2 py-0.5 text-[10px] font-medium text-white/90 backdrop-blur-sm">
        Installed
      </span>
    );
  }

  return null;
}


function subtitle(entry: LibraryEntry): string {
  if (entry.state.kind === "downloading") {
    return `${formatBytes(entry.state.receivedBytes)} of ${formatBytes(entry.state.totalBytes)}`;
  }
  if (entry.state.kind === "downloaded") {
    return `Downloaded (${formatBytes(entry.state.bytes)})`;
  }
  if (entry.minutesPlayed > 0) return formatPlaytime(entry.minutesPlayed);
  return formatBytes(entry.game.metadata.fileSize);
}
