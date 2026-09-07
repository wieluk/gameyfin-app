import { useEffect, useState } from "react";
import { backend } from "@/lib/backend";
import { useAppSettings } from "@/lib/queries";

/**
 * The download speed cap, where downloads are actually watched.
 *
 * Presets cover the common cases; the field accepts any value for the rest, because
 * "leave me 2 MB/s for a video call" is not a number anyone can guess in advance.
 */

const PRESETS = [
  { label: "Unlimited", kib: 0 },
  // Below 1 MB/s matters on a shared or metered connection, and used to be unreachable:
  // the custom field was whole MB/s, so the slowest cap anyone could set was 1 MB/s.
  { label: "256 KB/s", kib: 256 },
  { label: "512 KB/s", kib: 512 },
  { label: "1 MB/s", kib: 1024 },
  { label: "2 MB/s", kib: 2048 },
  { label: "5 MB/s", kib: 5120 },
  { label: "10 MB/s", kib: 10240 },
  { label: "25 MB/s", kib: 25600 },
];

/** MB/s for the custom field, keeping one decimal so 0.5 MB/s survives a round trip. */
function toMegabytes(kib: number): string {
  return String(Math.round((kib / 1024) * 10) / 10);
}

export function SpeedLimit() {
  const settings = useAppSettings();
  const stored = settings.data?.downloadLimitKib ?? 0;

  const [value, setValue] = useState<number | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const current = value ?? stored;
  const isPreset = PRESETS.some((p) => p.kib === current);

  useEffect(() => {
    // Anything not on the list is shown in the field rather than silently snapping to a
    // preset that is not what the user chose.
    if (!isPreset && current > 0) {
      setEditing(true);
      setDraft(toMegabytes(current));
    }
  }, [isPreset, current]);

  async function apply(kib: number) {
    setValue(kib);
    try {
      await backend.setDownloadLimit(kib);
    } catch {
      // A rejected setting is not worth interrupting a download list for; the next read
      // shows what actually stuck.
      void settings.refetch();
    }
  }

  return (
    <div className="flex items-center gap-2">
      <label className="text-xs text-foreground/45" htmlFor="speed-limit">
        Speed
      </label>

      {editing ? (
        <div className="flex items-center gap-1">
          <input
            id="speed-limit"
            // Deliberately not `type="number"`: it blanks anything the browser cannot
            // parse, which silently turned a mistyped limit into "unlimited". `inputMode`
            // still brings up a numeric keypad on touch.
            type="text"
            inputMode="decimal"
            value={draft}
            autoFocus
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => void commit()}
            onKeyDown={(e) => {
              if (e.key === "Enter") void commit();
              if (e.key === "Escape") setEditing(false);
            }}
            className="w-20 rounded-lg border border-default-200 bg-content2 px-2 py-1.5 text-xs outline-none focus:border-primary"
          />
          <span className="text-xs text-foreground/45">MB/s</span>
        </div>
      ) : (
        <select
          id="speed-limit"
          value={current}
          onChange={(e) => {
            if (e.target.value === "custom") {
              setDraft(current > 0 ? toMegabytes(current) : "");
              setEditing(true);
              return;
            }
            void apply(Number(e.target.value));
          }}
          className="rounded-lg border border-default-200 bg-content2 px-2.5 py-1.5 text-xs outline-none focus:border-primary"
        >
          {PRESETS.map((preset) => (
            <option key={preset.kib} value={preset.kib}>
              {preset.label}
            </option>
          ))}
          <option value="custom">Custom…</option>
        </select>
      )}
    </div>
  );

  async function commit() {
    setEditing(false);

    // A decimal comma is what most of Europe types, and `<input type="number">` reports an
    // unparseable value as the empty string, so "2,5" arrived here as "", became 0, and 0
    // means unlimited. Typing a limit therefore removed the limit. Parsed by hand instead,
    // from a text field, so what the user typed is what gets read.
    const typed = draft.trim().replace(",", ".");
    if (typed === "") return;

    const megabytes = Number(typed);
    // Anything unusable leaves the setting alone. "Unlimited" is a deliberate choice from
    // the list, never something a mistyped number falls into.
    if (!Number.isFinite(megabytes) || megabytes <= 0) return;

    // A positive request must never round down to zero, which would read as unlimited.
    await apply(Math.max(Math.round(megabytes * 1024), 1));
  }
}
