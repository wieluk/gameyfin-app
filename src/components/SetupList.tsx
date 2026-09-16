import { useRef, useState } from "react";

import type { SetupProgram } from "@/bindings/SetupProgram";
import { Icon } from "@/components/Icon";
import { Checkbox, IconButton } from "@/components/ui";
import { ROLE_LABEL, reordered } from "@/lib/setups";

const ROLE_STYLE: Record<SetupProgram["role"], string> = {
  game: "bg-primary/15 text-primary",
  patch: "bg-warning/15 text-warning-600",
  dlc: "bg-success/15 text-success",
  other: "bg-default-200 text-foreground/60",
};

type ListProps = {
  setups: SetupProgram[];
  selected: string[];
  onToggle: (path: string, on: boolean) => void;
  /** Rearranges the rows; without it they stay as given. */
  onMove?: (from: number, to: number) => void;
  disabled?: boolean;
};

/** The programs that will run, in the order they run, with the rest of the download folded below. */
export function SetupPicker({
  setups,
  selected,
  onToggle,
  onOrder,
  heading,
  disabled,
}: {
  setups: SetupProgram[];
  selected: string[];
  onToggle: (path: string, on: boolean) => void;
  onOrder: (order: string[]) => void;
  heading?: string;
  disabled?: boolean;
}) {
  // A ticked program joins the list it runs in, so it can be put in its place.
  const listed = setups.filter((s) => s.matched || selected.includes(s.path));
  const others = setups.filter((s) => !s.matched && !selected.includes(s.path));
  const all = setups.map((s) => s.path);

  return (
    <>
      {listed.length > 0 && (
        <>
          {heading && <p className="mb-1.5 text-xs text-foreground/55">{heading}</p>}
          <SetupList
            setups={listed}
            selected={selected}
            onToggle={onToggle}
            disabled={disabled}
            onMove={(from, to) => onOrder(reordered(all, listed.map((s) => s.path), from, to))}
          />
        </>
      )}
      {others.length > 0 && (
        <OtherPrograms
          setups={others}
          selected={selected}
          onToggle={onToggle}
          disabled={disabled}
        />
      )}
    </>
  );
}

/** Setup programs, each ticked on or off, and dragged or nudged into order. */
export function SetupList({ setups, selected, onToggle, onMove, disabled = false }: ListProps) {
  const list = useRef<HTMLUListElement>(null);
  // The gap the dragged row would drop into. Rows stay put until it drops, so the pointer is
  // never captured by a row that moves under it.
  const [drag, setDrag] = useState<{ from: number; gap: number } | null>(null);
  const movable = Boolean(onMove) && !disabled && setups.length > 1;

  function gapAt(clientY: number): number {
    const rows = Array.from(list.current?.children ?? []);
    const at = rows.findIndex((row) => {
      const box = row.getBoundingClientRect();
      return clientY < box.top + box.height / 2;
    });
    return at === -1 ? rows.length : at;
  }

  function drop() {
    if (drag && onMove) {
      const to = drag.gap > drag.from ? drag.gap - 1 : drag.gap;
      if (to !== drag.from) onMove(drag.from, to);
    }
    setDrag(null);
  }

  // The gaps either side of the dragged row would leave it where it is.
  const shownGap = drag && drag.gap !== drag.from && drag.gap !== drag.from + 1 ? drag.gap : null;

  return (
    <ul
      ref={list}
      className={`flex flex-col divide-y divide-default-200/60 rounded-lg border border-default-200 ${
        drag ? "select-none" : ""
      }`}
    >
      {setups.map((setup, index) => {
        const note = setup.installed
          ? "Installed"
          : setup.superseded
            ? "Already in the game installer"
            : // Only recognised setups were inspected for a silent mode.
              setup.matched && !setup.silent
              ? "Opens a wizard"
              : null;
        const line =
          shownGap === index
            ? "border-t-2 border-t-primary"
            : shownGap === setups.length && index === setups.length - 1
              ? "border-b-2 border-b-primary"
              : "";
        return (
          <li
            key={setup.path}
            className={`flex items-center ${line} ${drag?.from === index ? "opacity-50" : ""}`}
          >
            {movable && (
              <span
                title="Drag to change the order"
                className="flex h-8 w-6 shrink-0 cursor-grab touch-none items-center justify-center pl-1.5 text-foreground/35 hover:text-foreground/70 active:cursor-grabbing"
                onPointerDown={(e) => {
                  e.preventDefault();
                  e.currentTarget.setPointerCapture?.(e.pointerId);
                  setDrag({ from: index, gap: index });
                }}
                onPointerMove={(e) => {
                  if (drag) setDrag({ ...drag, gap: gapAt(e.clientY) });
                }}
                onPointerUp={drop}
                onPointerCancel={() => setDrag(null)}
              >
                <Icon name="grip" className="h-3.5 w-3.5" />
              </span>
            )}
            <label
              className={`flex min-w-0 flex-1 items-center gap-2.5 py-2 pr-3 ${
                movable ? "pl-1.5" : "pl-3"
              } ${
                disabled ? "cursor-not-allowed opacity-60" : "cursor-pointer hover:bg-default-100/60"
              }`}
            >
              <Checkbox
                checked={selected.includes(setup.path)}
                disabled={disabled}
                onChange={(e) => onToggle(setup.path, e.target.checked)}
              />
              {setup.matched && (
                <span
                  className={`w-12 shrink-0 rounded px-1.5 py-0.5 text-center text-[10px] font-medium ${ROLE_STYLE[setup.role]}`}
                >
                  {ROLE_LABEL[setup.role]}
                </span>
              )}
              <span
                className="min-w-0 flex-1 truncate font-mono text-xs text-foreground/80"
                title={setup.path}
              >
                {setup.path}
              </span>
              {note && <span className="shrink-0 text-[11px] text-foreground/45">{note}</span>}
            </label>
            {/* For keyboards and controllers, which cannot drag. */}
            {movable && (
              <span className="flex shrink-0 pr-1">
                <IconButton
                  icon="chevron"
                  size="sm"
                  label={`Move ${setup.path} up`}
                  iconClassName="h-3 w-3 -rotate-90"
                  disabled={index === 0}
                  onClick={() => onMove?.(index, index - 1)}
                />
                <IconButton
                  icon="chevron"
                  size="sm"
                  label={`Move ${setup.path} down`}
                  iconClassName="h-3 w-3 rotate-90"
                  disabled={index === setups.length - 1}
                  onClick={() => onMove?.(index, index + 1)}
                />
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/** Every other program in the download, folded away, for when the names gave nothing away. */
export function OtherPrograms({ setups, selected, onToggle, disabled }: ListProps) {
  return (
    <details className="mt-2 rounded-lg border border-default-200/60 px-2.5 py-1.5">
      <summary className="cursor-pointer text-[11px] text-foreground/45">
        Other programs in the download ({setups.length})
      </summary>
      <p className="mt-2 text-[11px] leading-relaxed text-foreground/45">
        Not recognised as a setup, patch or DLC. Tick one to add it to the list above.
      </p>
      <div className="mt-2">
        <SetupList setups={setups} selected={selected} onToggle={onToggle} disabled={disabled} />
      </div>
    </details>
  );
}
