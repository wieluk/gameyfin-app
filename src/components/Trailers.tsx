import { useState } from "react";

import { Icon } from "@/components/Icon";
import { backend } from "@/lib/backend";

/**
 * Gameplay videos on a game's page. Nothing contacts YouTube until the user clicks a
 * placeholder, and the embed uses `youtube-nocookie.com`.
 */
export function Trailers({ urls, title }: { urls: string[]; title: string }) {
  const [playing, setPlaying] = useState<string | null>(null);

  const videos = urls
    .map((url) => ({ url, id: youtubeId(url) }))
    .filter((video, index, all) => all.findIndex((v) => v.url === video.url) === index);

  if (videos.length === 0) return null;

  return (
    <section>
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-foreground/45">
        Videos
        <span className="ml-2 font-normal normal-case tracking-normal text-foreground/30">
          {videos.length}
        </span>
      </h3>

      <div className="flex gap-2 overflow-x-auto pb-2">
        {videos.map(({ url, id }) => (
          <div
            key={url}
            className="aspect-video h-40 shrink-0 overflow-hidden rounded-lg border border-default-200 bg-black"
          >
            {playing === url && id ? (
              <iframe
                src={`https://www.youtube-nocookie.com/embed/${id}?autoplay=1&rel=0`}
                title={`${title} video`}
                allow="accelerometer; autoplay; encrypted-media; gyroscope; picture-in-picture"
                allowFullScreen
                className="h-full w-full"
              />
            ) : (
              <button
                type="button"
                onClick={() => {
                  // Anything that is not a recognisable YouTube link is handed to the
                  // browser rather than guessed at: there is no embed URL to build, and a
                  // dead frame explains nothing.
                  if (id) setPlaying(url);
                  else void backend.openPath(url);
                }}
                title={url}
                className="group relative flex h-full w-full flex-col items-center justify-center gap-1.5 bg-content2 transition-colors hover:bg-default-100"
              >
                {id && (
                  // YouTube's own poster. onError leaves the plain tile behind it, which is
                  // what a video with no thumbnail, or no connection, falls back to.
                  <img
                    src={`https://i.ytimg.com/vi/${id}/hqdefault.jpg`}
                    alt=""
                    aria-hidden
                    loading="lazy"
                    className="absolute inset-0 h-full w-full object-cover transition-opacity group-hover:opacity-75"
                    onError={(e) => {
                      e.currentTarget.style.display = "none";
                    }}
                  />
                )}
                <span className="relative flex h-9 w-9 items-center justify-center rounded-full bg-primary/20 text-primary backdrop-blur-sm transition-colors group-hover:bg-primary group-hover:text-white">
                  <Icon name="play" className="h-4 w-4" filled />
                </span>
                <span className="relative px-2 text-[11px] text-foreground/45">
                  {id ? "Play trailer" : "Open in browser"}
                </span>
              </button>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}

/**
 * The video id in a YouTube URL, or null. Parsed with `URL`, not a regex, so a lookalike
 * host like `youtube.com.example.com` cannot match.
 */
export function youtubeId(raw: string): string | null {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;

  const host = url.hostname.replace(/^www\./, "");
  const id =
    host === "youtu.be"
      ? url.pathname.slice(1)
      : host === "youtube.com" || host === "m.youtube.com" || host === "youtube-nocookie.com"
        ? (url.searchParams.get("v") ??
          url.pathname.match(/^\/(?:embed|v|shorts)\/([^/]+)/)?.[1] ??
          null)
        : null;

  // Ids are a fixed alphabet; checking it keeps anything else out of the embed URL.
  return id && /^[A-Za-z0-9_-]{6,20}$/.test(id) ? id : null;
}
