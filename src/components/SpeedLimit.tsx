import { useState } from "react";
import { backend } from "@/lib/backend";
import { useAppSettings } from "@/lib/queries";

/**
 * The download speed cap, where downloads are watched. Presets cover the common cases;
 * the field accepts anything else, because nobody can guess "leave me 2 MB/s for a
 * video call" in advance.
 */

const PRESETS = [
  { label: "Unlimited", kib: 0 },
  // Sub-1 MB/s caps matter on shared or metered connections.
  { label: "256 KB/s", kib: 256 },
  { label: "512 KB/s", kib: 512 },
  { label: "1 MB/s", kib: 1024 },
  { label: "2 MB/s", kib: 2048 },
  { label: "5 MB/s", kib: 5120 },
  { label: "10 MB/s", kib: 10240 },
  { label: "25 MB/s", kib: 25600 },
];

/** MB/s for the custom field. Three decimals, so 0.25 and 1.5 survive a round trip. */
export function toMegabytes(kib: number): string {
  return String(Math.round((kib / 1024) * 1000) / 1000);
}

/** How an off-list cap reads in the list. Sub-1 MB/s values are clearer in KB/s. */
export function labelFor(kib: number): string {
  return kib < 1024 ? `${kib} KB/s` : `${toMegabytes(kib)} MB/s`;
}

/**
 * A typed cap as KiB/s, or null to leave the setting alone.
 *
 * Hand-parsed so decimal commas work, and so nothing unusable becomes 0: that reads as
 * unlimited, which is only ever a deliberate choice from the list.
 */
export function parseLimit(typed: string): number | null {
  const cleaned = typed.trim().replace(",", ".");
  if (cleaned === "") return null;

  const megabytes = Number(cleaned);
  if (!Number.isFinite(megabytes) || megabytes <= 0) return null;

  // Never round down to zero, which reads as unlimited.
  return Math.max(Math.round(megabytes * 1024), 1);
}

export function SpeedLimit() {
  const settings = useAppSettings();
  const stored = settings.data?.downloadLimitKib ?? 0;

  const [value, setValue] = useState<number | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const current = value ?? stored;
  const isPreset = PRESETS.some((p) => p.kib === current);

  async function apply(kib: number) {
    setValue(kib);
    try {
      await backend.setDownloadLimit(kib);
    } catch {
      // A rejected setting is not worth an alert; the refetch shows what stuck.
      void settings.refetch();
    }
  }

  // An off-list cap gets its own entry rather than opening the field and staying there,
  // which used to hide the list for good and put unlimited out of reach.
  const options = [
    ...PRESETS,
    ...(isPreset ? [] : [{ label: labelFor(current), kib: current }]),
  ];

  return (
    <div className="flex items-center gap-2">
      <label className="text-xs text-foreground/45" htmlFor="speed-limit">
        Speed
      </label>

      {editing ? (
        <div className="flex items-center gap-1">
          <input
            id="speed-limit"
            // `type="number"` blanks unparseable input, which silently read as unlimited;
            // `inputMode` still brings up a numeric keypad on touch.
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
          {options.map((option) => (
            <option key={option.kib} value={option.kib}>
              {option.label}
            </option>
          ))}
          <option value="custom">Custom…</option>
        </select>
      )}
    </div>
  );

  async function commit() {
    setEditing(false);
    const kib = parseLimit(draft);
    if (kib !== null) await apply(kib);
  }
}
