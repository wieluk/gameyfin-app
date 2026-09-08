/** Inline SVG icons, no icon-font dependency, and they inherit `currentColor`. */

const PATHS = {
  library: "M4 4h5v16H4zM11 4h5v16h-5zM18 6l3 14",
  download: "M12 3v12m0 0l-4-4m4 4l4-4M4 21h16",
  installed: "M4 7l8-4 8 4v10l-8 4-8-4z M4 7l8 4 8-4 M12 11v10",
  settings: "M12 15a3 3 0 100-6 3 3 0 000 6z M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 11-4 0v-.09A1.65 1.65 0 008 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 11-2.83-2.83l.06-.06A1.65 1.65 0 004.6 15a1.65 1.65 0 00-1.51-1H3a2 2 0 110-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06A1.65 1.65 0 009 4.6a1.65 1.65 0 001-1.51V3a2 2 0 114 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06A1.65 1.65 0 0019.4 9V9a1.65 1.65 0 001.51 1H21a2 2 0 110 4h-.09a1.65 1.65 0 00-1.51 1z",
  play: "M7 4l12 8-12 8z",
  close: "M6 6l12 12M18 6L6 18",
  search: "M11 19a8 8 0 100-16 8 8 0 000 16zM21 21l-4.35-4.35",
  controller: "M6 12h4m-2-2v4m6 1h.01M17 10h.01M7 18a5 5 0 01-5-5l1-5a4 4 0 014-3h10a4 4 0 014 3l1 5a5 5 0 01-5 5c-2 0-2.5-2-5-2s-3 2-5 2z",
  star: "M12 3l2.9 5.9 6.5.9-4.7 4.6 1.1 6.5L12 17.8 6.2 20.9l1.1-6.5L2.6 9.8l6.5-.9z",
  chevron: "M9 6l6 6-6 6",
  folder: "M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z",
  offline: "M2 2l20 20 M8.5 16.4a5 5 0 016.9 0 M5 12.9a10 10 0 013.6-2.3 M19 12.9a10 10 0 00-5.5-2.7 M1.4 9.4a15 15 0 014.2-2.7 M22.6 9.4a15 15 0 00-11-3.6 M12 20h.01",
  cloud: "M7 18a4 4 0 010-8 6 6 0 0111.2-1.8A3.6 3.6 0 0118 18z",
  warning: "M12 9v4m0 4h.01M10.3 4.3L2.5 18a2 2 0 001.7 3h15.6a2 2 0 001.7-3L13.7 4.3a2 2 0 00-3.4 0z",
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({
  name,
  className = "h-5 w-5",
  filled,
}: {
  name: IconName;
  className?: string;
  filled?: boolean;
}) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {PATHS[name].split(" M").map((segment, i) => (
        <path key={i} d={i === 0 ? segment : `M${segment}`} />
      ))}
    </svg>
  );
}
