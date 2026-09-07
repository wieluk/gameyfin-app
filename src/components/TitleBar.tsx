import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon } from "./Icon";

/**
 * Custom window chrome.
 *
 * Dragging is delegated to the compositor through `data-tauri-drag-region`. That
 * attribute is the mechanism Tauri looks for, the CSS `-webkit-app-region: drag`
 * property used by Electron has no effect here, and its absence is why the window could
 * not be moved.
 */
export function TitleBar({ title }: { title?: string }) {
  return (
    <header
      data-tauri-drag-region
      onDoubleClick={() => void windowAction("toggleMaximize")}
      className="flex h-9 shrink-0 select-none items-center justify-between border-b border-default-200/60 bg-content1 px-3"
    >
      {/* Children of a drag region are not draggable, so the attribute is repeated on
          the inert areas that should also move the window. */}
      <div data-tauri-drag-region className="pointer-events-none flex items-center gap-2">
        <Icon name="controller" className="h-4 w-4 text-primary" />
        <span className="text-xs font-medium tracking-wide text-foreground/70">
          {title ?? "Gameyfin"}
        </span>
      </div>

      <div className="flex items-center gap-1">
        <WindowButton label="Minimise" onClick={() => void windowAction("minimize")}>
          <span className="block h-px w-2.5 bg-current" />
        </WindowButton>
        <WindowButton label="Maximise" onClick={() => void windowAction("toggleMaximize")}>
          <span className="block h-2 w-2 border border-current" />
        </WindowButton>
        <WindowButton label="Close" danger onClick={() => void windowAction("close")}>
          <Icon name="close" className="h-3 w-3" />
        </WindowButton>
      </div>
    </header>
  );
}

type Action = "minimize" | "toggleMaximize" | "close";

async function windowAction(action: Action) {
  if (!("__TAURI_INTERNALS__" in window)) return;
  const w = getCurrentWindow();
  if (action === "minimize") await w.minimize();
  else if (action === "toggleMaximize") await w.toggleMaximize();
  else await w.close();
}

function WindowButton({
  children,
  label,
  onClick,
  danger,
}: {
  children: React.ReactNode;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className={`flex h-6 w-8 items-center justify-center rounded text-foreground/60 transition-colors hover:text-foreground ${
        danger ? "hover:bg-danger hover:text-white" : "hover:bg-default-200"
      }`}
    >
      {children}
    </button>
  );
}
