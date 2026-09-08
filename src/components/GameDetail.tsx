import { Trailers } from "@/components/Trailers";
import { useEffect, useRef, useState } from "react";
import { Icon } from "./Icon";
import { primaryAction } from "@/lib/actions";
import { formatBytes, formatPlaytime } from "@/lib/format";
import type { LibraryEntry } from "@/types";

/** Game details as a dialog over the library, so the grid keeps its scroll and filters. */
export function GameDetail({
  entry,
  onClose,
  onPrimaryAction,
}: {
  entry: LibraryEntry;
  onClose: () => void;
  onPrimaryAction: (entry: LibraryEntry) => void;
}) {
  const { game, state } = entry;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const facts: [string, string][] = [
    ["Size", formatBytes(game.metadata.fileSize)],
    ["Playtime", formatPlaytime(entry.minutesPlayed)],
    ...(game.release ? ([["Released", game.release]] as [string, string][]) : []),
    ...(game.developers.length
      ? ([["Developer", game.developers.join(", ")]] as [string, string][])
      : []),
    ...(game.publishers.length
      ? ([["Publisher", game.publishers.join(", ")]] as [string, string][])
      : []),
    ...(game.platforms.length
      ? ([["Platforms", game.platforms.join(", ")]] as [string, string][])
      : []),
  ];

  return (
    <div
      data-nav-scope
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label={game.title}
      onClick={onClose}
    >
      <div
        className="flex h-[min(88vh,860px)] w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-default-200 bg-content1 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <Header entry={entry} onClose={onClose} />

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          <div className="mb-5 flex flex-wrap gap-2">
            <PrimaryButton entry={entry} onClick={() => onPrimaryAction(entry)} />
            {state.kind === "installed" && (
              <span className="rounded-lg border border-default-200 px-3 py-2 text-xs text-foreground/55">
                {state.path}
              </span>
            )}
          </div>

          {state.kind === "failed" && (
            <p
              role="alert"
              className="mb-5 rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
            >
              {state.message}
            </p>
          )}

          {game.summary && (
            <p className="mb-5 whitespace-pre-line text-sm leading-relaxed text-foreground/75">
              {game.summary}
            </p>
          )}

          <dl className="mb-5 grid grid-cols-[auto,1fr] gap-x-6 gap-y-2 text-sm">
            {facts.map(([label, value]) => (
              <div key={label} className="contents">
                <dt className="text-foreground/45">{label}</dt>
                <dd className="text-foreground/80">{value}</dd>
              </div>
            ))}
          </dl>

          {game.genres.length > 0 && (
            <div className="mb-5 flex flex-wrap gap-1.5">
              {game.genres.map((genre) => (
                <span
                  key={genre}
                  className="rounded-full bg-default-100 px-2.5 py-0.5 text-[11px] text-foreground/65"
                >
                  {genre}
                </span>
              ))}
            </div>
          )}

          <Trailers urls={entry.videoUrls ?? []} title={game.title} />
          <Screenshots urls={entry.screenshotUrls ?? []} title={game.title} />
        </div>
      </div>
    </div>
  );
}

function Header({ entry, onClose }: { entry: LibraryEntry; onClose: () => void }) {
  const art = entry.headerUrl ?? entry.coverUrl ?? null;

  return (
    <div className="relative h-52 shrink-0 bg-default-200">
      {art && <img src={art} alt="" className="h-full w-full object-cover" />}
      <div className="absolute inset-0 bg-gradient-to-t from-content1 via-content1/70 to-transparent" />

      <button
        type="button"
        aria-label="Close"
        onClick={onClose}
        className="absolute right-3 top-3 flex h-7 w-7 items-center justify-center rounded-lg bg-black/40 text-white/80 backdrop-blur-sm transition-colors hover:bg-black/60 hover:text-white"
      >
        <Icon name="close" className="h-3.5 w-3.5" />
      </button>

      <h2 className="absolute bottom-3 left-6 right-6 truncate text-xl font-semibold text-foreground">
        {entry.game.title}
      </h2>
    </div>
  );
}

function PrimaryButton({
  entry,
  onClick,
}: {
  entry: LibraryEntry;
  onClick: () => void;
}) {
  const action = primaryAction(entry.state);

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={action.disabled}
      className="flex items-center gap-2 rounded-lg bg-primary px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50"
    >
      <Icon name={action.icon} className="h-4 w-4" filled={action.icon === "play"} />
      {action.label}
    </button>
  );
}

function Screenshots({ urls, title }: { urls: string[]; title: string }) {
  const strip = useRef<HTMLDivElement>(null);
  const [lightbox, setLightbox] = useState<string | null>(null);

  if (urls.length === 0) return null;

  // Scroll by most of a viewport so the user keeps a visual anchor.
  const scrollBy = (direction: 1 | -1) => {
    const el = strip.current;
    if (!el) return;
    el.scrollBy({ left: direction * el.clientWidth * 0.8, behavior: "smooth" });
  };

  return (
    <section>
      <div className="mb-2 flex items-center justify-between">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-foreground/45">
          Screenshots
          <span className="ml-2 font-normal normal-case tracking-normal text-foreground/30">
            {urls.length}
          </span>
        </h3>
        <div className="flex gap-1">
          <ScrollButton label="Scroll left" onClick={() => scrollBy(-1)} flip />
          <ScrollButton label="Scroll right" onClick={() => scrollBy(1)} />
        </div>
      </div>

      <div
        ref={strip}
        tabIndex={0}
        role="group"
        aria-label={`${title} screenshots`}
        onKeyDown={(e) => {
          if (e.key === "ArrowRight") scrollBy(1);
          if (e.key === "ArrowLeft") scrollBy(-1);
        }}
        className="flex gap-2 overflow-x-auto pb-2 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
      >
        {urls.map((url) => (
          <button
            key={url}
            type="button"
            onClick={() => setLightbox(url)}
            className="shrink-0 overflow-hidden rounded-lg border border-default-200 transition-transform hover:scale-[1.02]"
          >
            <img
              src={url}
              alt={`${title} screenshot`}
              loading="lazy"
              className="h-44 w-auto object-cover"
            />
          </button>
        ))}
      </div>

      {lightbox && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/85 p-8"
          role="dialog"
          aria-modal="true"
          aria-label="Screenshot"
          onClick={() => setLightbox(null)}
        >
          <img src={lightbox} alt="" className="max-h-full max-w-full rounded-lg object-contain" />
        </div>
      )}
    </section>
  );
}

function ScrollButton({
  label,
  onClick,
  flip,
}: {
  label: string;
  onClick: () => void;
  flip?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className="flex h-6 w-6 items-center justify-center rounded-md border border-default-200 text-foreground/50 transition-colors hover:bg-default-100 hover:text-foreground"
    >
      <Icon name="chevron" className={`h-3.5 w-3.5 ${flip ? "rotate-180" : ""}`} />
    </button>
  );
}
