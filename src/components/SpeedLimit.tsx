import { useState } from "react";

import { Select, TextInput } from "@/components/ui";
import { useAppSettings, useSettingsUpdate } from "@/lib/queries";

/** The download speed cap. Presets cover the common cases; the field takes anything else. */

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
 * A typed cap as KiB/s, or null to leave the setting alone. Hand-parsed so decimal commas work
 * and nothing unusable becomes 0, which reads as unlimited.
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
  const save = useSettingsUpdate();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const current = value ?? stored;
  const isPreset = PRESETS.some((p) => p.kib === current);

  async function apply(kib: number) {
    setValue(kib);
    try {
      await save({ downloadLimitKib: kib });
    } finally {
      // Whether it stuck or not, the stored value is the truth from here.
      setValue(null);
    }
  }

  // An off-list cap gets its own entry rather than opening the field and staying there,
  // which would hide the list for good and put unlimited out of reach.
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
          <span className="w-20">
            <TextInput
              id="speed-limit"
              // `type="number"` blanks unparseable input, which would read as unlimited; `inputMode`
              // still brings up a numeric keypad.
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
            />
          </span>
          <span className="text-xs text-foreground/45">MB/s</span>
        </div>
      ) : (
        <Select
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
        >
          {options.map((option) => (
            <option key={option.kib} value={option.kib}>
              {option.label}
            </option>
          ))}
          <option value="custom">Custom…</option>
        </Select>
      )}
    </div>
  );

  async function commit() {
    setEditing(false);
    const kib = parseLimit(draft);
    if (kib !== null) await apply(kib);
  }
}
